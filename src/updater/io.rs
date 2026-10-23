use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

pub(super) fn uid() -> Result<u32, String> {
    fs::metadata("/proc/self").map(|m| m.uid()).map_err(|e| e.to_string())
}

pub(super) fn private_dir(path: &Path) -> Result<(), String> {
    if !path.exists() {
        DirBuilder::new().recursive(true).mode(0o700).create(path).map_err(|e| e.to_string())?;
    }
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_dir() || meta.uid() != uid()? {
        return Err("Update directory must be owned by the current user and not a symlink".into());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}

pub(super) fn new_file(path: &Path, mode: u32) -> Result<File, String> {
    OpenOptions::new().write(true).read(true).create_new(true).mode(mode).open(path).map_err(|e| e.to_string())
}

pub(super) fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > max as u64 {
        return Err("Invalid update file type or size".into());
    }
    let mut out = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(max as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    if out.len() > max {
        return Err("Update file is too large".into());
    }
    Ok(out)
}

pub(super) fn nonce() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("{:x}-{:x}-{:x}", std::process::id(), crate::persist::now(), NEXT.fetch_add(1, Ordering::Relaxed))
}

pub(super) struct Workspace {
    pub path: PathBuf,
}
impl Workspace {
    pub fn new(parent: &Path) -> Result<Self, String> {
        private_dir(parent)?;
        for _ in 0..32 {
            let path = parent.join(format!("work-{}", nonce()));
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("Could not allocate an update workspace".into())
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path); // Best-effort cleanup of this operation's private scratch files.
    }
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let parent = path.parent().ok_or("Update file has no parent")?;
    private_dir(parent)?;
    let temp = parent.join(format!(".write-{}", nonce()));
    let result = (|| {
        let mut file = new_file(&temp, mode)?;
        file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|e| e.to_string())?;
        fs::rename(&temp, path).map_err(|e| e.to_string())?;
        File::open(parent).and_then(|dir| dir.sync_all()).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub(super) fn stream(
    mut cmd: Command,
    cancel: &AtomicBool,
    timeout: Duration,
    emit: &mut dyn FnMut(&[u8]),
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err(crate::proc::CANCELLED.into());
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| format!("Could not run {}: {e}", cmd.get_program().to_string_lossy()))?;
    let done = crate::proc::stream(child, Vec::new(), cancel, timeout, emit)?;
    if !done.status.success() {
        return Err(format!("Update command failed: {}", done.stderr.trim()));
    }
    Ok(())
}

pub(super) fn output(cmd: Command, max: usize, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut too_large = false;
    let result = stream(cmd, cancel, Duration::from_secs(30), &mut |bytes| {
        if out.len().saturating_add(bytes.len()) > max {
            too_large = true;
            cancel.store(true, Ordering::Relaxed);
        } else {
            out.extend_from_slice(bytes);
        }
    });
    if too_large {
        return Err("Update response exceeded its size limit".into());
    }
    result?;
    Ok(out)
}

pub(super) fn sha256(path: &Path, cancel: &AtomicBool) -> Result<String, String> {
    let mut cmd = Command::new("openssl");
    cmd.args(["dgst", "-sha256"]).arg(path);
    let bytes = output(cmd, 4096, cancel)?;
    let text = String::from_utf8(bytes).map_err(|_| "Invalid checksum output")?;
    let hash = text.split_whitespace().last().ok_or("Missing checksum output")?;
    if !super::valid_hash(hash) {
        return Err("Invalid SHA-256 output".into());
    }
    Ok(hash.to_owned())
}

pub(super) fn verify_signature(bytes: &[u8], signature: &[u8], key: &str, cancel: &AtomicBool) -> Result<(), String> {
    let work = Workspace::new(&std::env::temp_dir().join(format!("mira-signature-{}", uid()?)))?;
    for (name, contents) in [("metadata", bytes), ("signature", signature), ("public.pem", key.as_bytes())] {
        new_file(&work.path.join(name), 0o600)?.write_all(contents).map_err(|e| e.to_string())?;
    }
    let mut cmd = Command::new("openssl");
    cmd.args(["pkeyutl", "-verify", "-pubin", "-rawin", "-inkey"])
        .arg(work.path.join("public.pem"))
        .arg("-in")
        .arg(work.path.join("metadata"))
        .arg("-sigfile")
        .arg(work.path.join("signature"));
    output(cmd, 4096, cancel)
        .map(|_| ())
        .map_err(|e| format!("Update signature verification failed (OpenSSL 3 is required): {e}"))
}

pub(super) fn curl(url: &str) -> Command {
    let mut cmd = Command::new("curl");
    cmd.args([
        "--disable", // Ignore curlrc: release requests use only these explicit options.
        "--silent",
        "--show-error",
        "--location",
        "--max-redirs",
        "5",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "10",
        "--max-time",
        "600",
        "--user-agent",
        concat!("mira-updater/", env!("CARGO_PKG_VERSION")),
    ]);
    cmd.arg(url);
    cmd
}

pub(super) struct Response {
    pub status: u16,
    pub etag: Option<String>,
    pub retry_after: u64,
    pub body: Vec<u8>,
}

pub(super) fn get(url: &str, etag: Option<&str>, max: usize, cancel: &AtomicBool) -> Result<Response, String> {
    let work = Workspace::new(&super::state_dir()?)?;
    let headers = work.path.join("headers");
    let mut cmd = curl(url);
    cmd.args(["--max-time", "30", "--max-filesize", &max.to_string(), "--dump-header"]).arg(&headers);
    if let Some(etag) = etag.filter(|v| v.len() <= 256 && !v.contains(['\r', '\n'])) {
        cmd.arg("--header").arg(format!("If-None-Match: {etag}"));
    }
    let body = output(cmd, max, cancel)?;
    let raw = read_bounded(&headers, 32 * 1024)?;
    let (status, etag, retry_after) = parse_headers(&String::from_utf8_lossy(&raw));
    Ok(Response { status, etag, retry_after, body })
}

pub(super) fn parse_headers(raw: &str) -> (u16, Option<String>, u64) {
    let mut status = 0;
    let mut etag = None;
    let mut retry_after = 0;
    for line in raw.lines().map(str::trim) {
        if line.starts_with("HTTP/") {
            status = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            etag = None;
            retry_after = 0;
        } else if let Some((key, value)) = line.split_once(':') {
            let value = value.trim();
            if key.eq_ignore_ascii_case("etag") && value.len() <= 256 {
                etag = Some(value.to_string());
            }
            if key.eq_ignore_ascii_case("retry-after") {
                retry_after = crate::persist::now().saturating_add(value.parse::<u64>().unwrap_or(60));
            }
            if key.eq_ignore_ascii_case("x-ratelimit-reset") {
                retry_after = retry_after.max(value.parse().unwrap_or(0));
            }
        }
    }
    (status, etag, retry_after)
}
