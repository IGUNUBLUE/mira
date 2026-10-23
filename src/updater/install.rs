use super::{Manifest, Release, io};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    format: u8,
    binary_sha256: String,
    #[serde(default)]
    previous_binary_sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub binary: PathBuf,
    marker: PathBuf,
    old_hash: String,
    device: u64,
    inode: u64,
}
impl Target {
    pub fn detect(exe: &Path) -> Result<Self, String> {
        let meta = fs::symlink_metadata(exe).map_err(|e| e.to_string())?;
        let bin = exe.parent().ok_or("No binary directory")?;
        let prefix = bin.parent().ok_or("No installation prefix")?;
        if exe.file_name().and_then(|n| n.to_str()) != Some("mira")
            || !meta.is_file()
            || meta.uid() != io::uid()?
            || meta.uid() == 0
            || fs::metadata(bin).map_err(|e| e.to_string())?.uid() != meta.uid()
        {
            return Err(
                "This installation is managed externally. Update it with its installer or package manager.".into()
            );
        }
        let marker = prefix.join("share/mira/install.json");
        let bytes = io::read_bounded(&marker, 4096)
            .map_err(|_| "Use Mira's current install.sh to enable in-app updates for this installation")?;
        let record: Record = serde_json::from_slice(&bytes).map_err(|e| format!("Installation record: {e}"))?;
        if record.format != 1 || !super::valid_hash(&record.binary_sha256) {
            return Err("Invalid installation record; reinstall Mira with install.sh".into());
        }
        let target = Self {
            binary: exe.to_path_buf(),
            marker,
            old_hash: record.binary_sha256,
            device: meta.dev(),
            inode: meta.ino(),
        };
        target.revalidate(&AtomicBool::new(false))?;
        Ok(target)
    }
    pub(super) fn revalidate(&self, cancel: &AtomicBool) -> Result<(), String> {
        let meta = fs::symlink_metadata(&self.binary).map_err(|e| e.to_string())?;
        let record: Record =
            serde_json::from_slice(&io::read_bounded(&self.marker, 4096)?).map_err(|e| e.to_string())?;
        if !meta.is_file()
            || meta.dev() != self.device
            || meta.ino() != self.inode
            || meta.uid() != io::uid()?
            || record.format != 1
            || record.binary_sha256 != self.old_hash
            || io::sha256(&self.binary, cancel)? != self.old_hash
        {
            return Err("Mira changed on disk. Restart it before updating.".into());
        }
        Ok(())
    }
    fn pending_path(&self, m: &Manifest) -> PathBuf {
        self.binary.with_file_name(format!(".mira-update-{}-{}.pending", m.version, m.binary_sha256))
    }
}

pub(super) fn repair_record(exe: &Path, release: &Release, cancel: &AtomicBool) -> Result<Target, String> {
    let meta = fs::symlink_metadata(exe).map_err(|e| e.to_string())?;
    let bin = exe.parent().ok_or("No binary directory")?;
    if !meta.is_file()
        || meta.uid() == 0
        || meta.uid() != io::uid()?
        || fs::metadata(bin).map_err(|e| e.to_string())?.uid() != meta.uid()
        || exe.file_name().and_then(|s| s.to_str()) != Some("mira")
        || release.manifest.version != env!("CARGO_PKG_VERSION")
    {
        return Err("Cannot recover this installation record".into());
    }
    let _lock = installation_lock(exe)?;
    let marker = bin.parent().ok_or("No prefix")?.join("share/mira/install.json");
    let old: Record = serde_json::from_slice(&io::read_bounded(&marker, 4096)?).map_err(|e| e.to_string())?;
    if old.format != 1
        || !super::valid_hash(&old.binary_sha256)
        || io::sha256(exe, cancel)? != release.manifest.binary_sha256
    {
        return Err("Cannot authenticate interrupted update".into());
    }
    let backup = exe.with_file_name(format!(".mira-previous-{}", old.binary_sha256));
    if io::sha256(&backup, cancel)? != old.binary_sha256 {
        return Err("Interrupted update has no valid backup".into());
    }
    let pending = exe.with_file_name(format!(
        ".mira-update-{}-{}.pending",
        release.manifest.version, release.manifest.binary_sha256
    ));
    for notice in notice_changes(exe, &notices_path(&pending))? {
        io::atomic_write(&notice.path, &notice.next, 0o644)?;
    }
    let new = Record {
        format: 1,
        binary_sha256: release.manifest.binary_sha256.clone(),
        previous_binary_sha256: Some(old.binary_sha256),
    };
    io::atomic_write(&marker, &serde_json::to_vec(&new).map_err(|e| e.to_string())?, 0o600)?;
    Target::detect(exe)
}

const NOTICES: [&str; 3] = ["LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY_LICENSES.html"];
const MAX_NOTICE: usize = 4 * 1024 * 1024;

fn notices_path(binary: &Path) -> PathBuf {
    binary.with_file_name(format!("{}.notices", binary.file_name().unwrap_or_default().to_string_lossy()))
}

#[derive(Clone, Debug)]
pub struct Prepared {
    pub target: Target,
    pub release: Release,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Installed {
    pub binary: PathBuf,
    pub backup: PathBuf,
    marker: PathBuf,
    previous_record: Vec<u8>,
    new_hash: String,
    pub version: String,
    obsolete_backup: Option<PathBuf>,
    previous_notices: Vec<(PathBuf, Option<Vec<u8>>)>,
}

fn validate_binary(path: &Path, release: &Release, cancel: &AtomicBool, launch: bool) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file()
        || meta.uid() != io::uid()?
        || meta.len() != release.manifest.binary_size
        || io::sha256(path, cancel)? != release.manifest.binary_sha256
    {
        return Err("Downloaded Mira failed its signed binary checksum or size check".into());
    }
    let mut header = [0_u8; 20];
    File::open(path).and_then(|mut f| f.read_exact(&mut header)).map_err(|e| e.to_string())?;
    let machine = if std::env::consts::ARCH == "x86_64" { 62 } else { 183 };
    if &header[..4] != b"\x7fELF"
        || header[4] != 2
        || header[5] != 1
        || u16::from_le_bytes([header[18], header[19]]) != machine
    {
        return Err("Downloaded Mira is not a compatible Linux executable".into());
    }
    if launch {
        let mut cmd = Command::new(path);
        cmd.arg("--version");
        let output = io::output(cmd, 1024, cancel)?;
        if String::from_utf8_lossy(&output).trim() != format!("mira {}", release.manifest.version) {
            return Err("Downloaded Mira reports an unexpected version".into());
        }
    }
    Ok(())
}

pub(super) fn prepare(
    target: Target,
    release: Release,
    archive: &Path,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    let path = target.pending_path(&release.manifest);
    let temp = target.binary.with_file_name(format!(".mira-download-{}", io::nonce()));
    let result = (|| {
        let mut file = io::new_file(&temp, 0o700)?;
        let mut cmd = Command::new("tar");
        cmd.arg("-xOzf")
            .arg(archive)
            .arg("--")
            .arg(format!("{}/mira", super::name(&release.manifest.version, &release.manifest.arch)));
        let mut count = 0_u64;
        let mut error = None;
        let result = io::stream(cmd, cancel, Duration::from_secs(60), &mut |bytes| {
            if error.is_some() {
                return;
            }
            count = count.saturating_add(bytes.len() as u64);
            if count > release.manifest.binary_size {
                error = Some("Extracted Mira exceeds its signed size".to_string());
            } else if let Err(e) = file.write_all(bytes) {
                error = Some(e.to_string());
            }
            if error.is_some() {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        if let Some(e) = error {
            return Err(e);
        }
        result?;
        file.sync_all().map_err(|e| e.to_string())?;
        validate_binary(&temp, &release, cancel, true)?;
        // A private staging name is reserved for this signed version and digest.
        if path.exists() {
            validate_binary(&path, &release, cancel, false)?;
            fs::remove_file(&temp).map_err(|e| e.to_string())?;
        } else {
            fs::rename(&temp, &path).map_err(|e| e.to_string())?;
        }
        let notices = notices_path(&path);
        io::private_dir(&notices)?;
        for name in NOTICES {
            let mut cmd = Command::new("tar");
            cmd.arg("-xOzf")
                .arg(archive)
                .arg("--")
                .arg(format!("{}/{name}", super::name(&release.manifest.version, &release.manifest.arch)));
            let bytes = io::output(cmd, MAX_NOTICE, cancel)?;
            io::atomic_write(&notices.join(name), &bytes, 0o600)?;
        }
        Ok(Prepared { target, release, path })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    } // Remove this operation's incomplete file only.
    result
}

pub(super) fn restore(target: &Target, release: Release, cancel: &AtomicBool) -> Result<Prepared, String> {
    let path = target.pending_path(&release.manifest);
    validate_binary(&path, &release, cancel, true)?;
    for name in NOTICES {
        io::read_bounded(&notices_path(&path).join(name), MAX_NOTICE)?;
    }
    Ok(Prepared { target: target.clone(), release, path })
}

struct NoticeChange {
    path: PathBuf,
    previous: Option<Vec<u8>>,
    next: Vec<u8>,
}

// Keep each binary's dependency notices together with its atomic installation and rollback.
fn notice_changes(binary: &Path, staged: &Path) -> Result<Vec<NoticeChange>, String> {
    let docs = binary.parent().and_then(Path::parent).ok_or("No installation prefix")?.join("share/doc/mira");
    io::private_dir(&docs)?;
    NOTICES
        .into_iter()
        .map(|name| {
            let path = docs.join(name);
            let previous = if path.exists() { Some(io::read_bounded(&path, MAX_NOTICE)?) } else { None };
            let next = io::read_bounded(&staged.join(name), MAX_NOTICE)?;
            Ok(NoticeChange { path, previous, next })
        })
        .collect()
}
fn restore_notices(previous: &[(PathBuf, Option<Vec<u8>>)]) -> Result<(), String> {
    for (path, bytes) in previous {
        match bytes {
            Some(bytes) => io::atomic_write(path, bytes, 0o644)?,
            None if path.exists() => fs::remove_file(path).map_err(|e| e.to_string())?,
            None => {}
        }
    }
    Ok(())
}

fn installation_lock(binary: &Path) -> Result<File, String> {
    let path = binary.with_file_name(".mira-update.lock");
    if path.exists() && !fs::symlink_metadata(&path).map_err(|e| e.to_string())?.is_file() {
        return Err("Invalid update lock".into());
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.try_lock().map_err(|e| format!("Another Mira instance is updating: {e}"))?;
    Ok(file)
}

pub(super) fn apply(prepared: &Prepared, cancel: &AtomicBool) -> Result<Installed, String> {
    validate_binary(&prepared.path, &prepared.release, cancel, true)?;
    commit_prepared(prepared, cancel, |path, bytes| io::atomic_write(path, bytes, 0o600))
}

fn commit_prepared(
    prepared: &Prepared,
    cancel: &AtomicBool,
    write_record: impl FnOnce(&Path, &[u8]) -> Result<(), String>,
) -> Result<Installed, String> {
    let target = &prepared.target;
    let _lock = installation_lock(&target.binary)?;
    target.revalidate(cancel)?;
    validate_binary(&prepared.path, &prepared.release, cancel, false)?;
    let previous_record = io::read_bounded(&target.marker, 4096)?;
    let notices = notice_changes(&target.binary, &notices_path(&prepared.path))?;
    let previous_notices: Vec<_> = notices.iter().map(|n| (n.path.clone(), n.previous.clone())).collect();
    let backup = target.binary.with_file_name(format!(".mira-previous-{}", target.old_hash));
    if backup.exists() {
        if io::sha256(&backup, cancel)? != target.old_hash
            || !fs::symlink_metadata(&backup).map_err(|e| e.to_string())?.is_file()
        {
            return Err("Previous-version backup does not match this installation".into());
        }
    } else {
        fs::hard_link(&target.binary, &backup).map_err(|e| format!("Could not preserve the current Mira: {e}"))?;
    }
    fs::set_permissions(&prepared.path, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    fs::rename(&prepared.path, &target.binary).map_err(|e| e.to_string())?;
    let record = Record {
        format: 1,
        binary_sha256: prepared.release.manifest.binary_sha256.clone(),
        previous_binary_sha256: Some(target.old_hash.clone()),
    };
    let bytes = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
    let record_result = (|| {
        for notice in &notices {
            io::atomic_write(&notice.path, &notice.next, 0o644)?;
        }
        write_record(&target.marker, &bytes)
    })();
    if let Err(e) = record_result {
        let notice_result = restore_notices(&previous_notices);
        let e = match notice_result {
            Ok(()) => e,
            Err(notice) => format!("{e}; notice recovery: {notice}"),
        };
        return match fs::rename(&backup, &target.binary) {
            Ok(()) => match io::atomic_write(&target.marker, &previous_record, 0o600) {
                Ok(()) => Err(format!("Update could not be recorded; previous Mira restored: {e}")),
                Err(record) => Err(format!(
                    "Previous Mira restored, but reinstall to repair its record: {record}. Original failure: {e}"
                )),
            },
            Err(restore) => {
                Err(format!("Update record failed: {e}. Previous binary is at {} ({restore})", backup.display()))
            }
        };
    }
    File::open(target.binary.parent().ok_or("Missing binary directory")?)
        .and_then(|d| d.sync_all())
        .map_err(|e| e.to_string())?;
    let old_record: Record = serde_json::from_slice(&previous_record).map_err(|e| e.to_string())?;
    let obsolete_backup = old_record
        .previous_binary_sha256
        .filter(|h| super::valid_hash(h))
        .map(|h| target.binary.with_file_name(format!(".mira-previous-{h}")));
    Ok(Installed {
        previous_notices,
        obsolete_backup,
        binary: target.binary.clone(),
        backup,
        marker: target.marker.clone(),
        previous_record,
        new_hash: record.binary_sha256,
        version: prepared.release.manifest.version.clone(),
    })
}

impl Installed {
    fn rollback(&self) -> Result<(), String> {
        let _lock = installation_lock(&self.binary)?;
        if io::sha256(&self.binary, &AtomicBool::new(false))? != self.new_hash {
            return Err("Installation changed after updating; backup retained".into());
        }
        fs::rename(&self.backup, &self.binary).map_err(|e| e.to_string())?;
        let marker = io::atomic_write(&self.marker, &self.previous_record, 0o600);
        let notices = restore_notices(&self.previous_notices);
        marker.and(notices)
    }
}

static HEALTH_TOKEN: OnceLock<String> = OnceLock::new();
pub fn valid_health_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= 96 && token.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}
pub fn register_health(token: String) -> Result<(), String> {
    if !valid_health_token(&token) {
        return Err("Invalid update startup token".into());
    }
    HEALTH_TOKEN.set(token).map_err(|_| "Duplicate startup token".into())
}
pub fn has_health() -> bool {
    HEALTH_TOKEN.get().is_some()
}

pub(super) fn startup_health(token: &str) -> Result<(), String> {
    let path = super::state_dir()?.join(format!("ready-{token}"));
    let mut file = io::new_file(&path, 0o600)?;
    writeln!(file, "{}:{}", std::process::id(), env!("CARGO_PKG_VERSION"))
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())
}
pub fn acknowledge() -> Result<(), String> {
    match HEALTH_TOKEN.get() {
        Some(token) => startup_health(token),
        None => Ok(()),
    }
}

pub(super) fn restart(installed: &Installed, root: &Path, follow: bool) -> Result<(), String> {
    let token = io::nonce();
    let ready = super::state_dir()?.join(format!("ready-{token}"));
    if ready.exists() {
        return Err("Update startup token already exists".into());
    }
    let mut command = Command::new(&installed.binary);
    if follow {
        command.current_dir(std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| root.to_owned()));
    } else {
        command.current_dir(root);
    }
    command.arg(format!("--update-health={token}"));
    for (key, _) in std::env::vars_os().filter(|(k, _)| k.as_encoded_bytes().starts_with(b"HERDR_")) {
        command.env_remove(key);
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(super::state_dir()?.join("restart.log"))
        .map_err(|e| e.to_string())?;
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            installed.rollback()?;
            return Err(format!("Could not start the update; previous Mira restored: {e}"));
        }
    };
    let expected = format!("{}:{}", child.id(), installed.version);
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        if let Ok(bytes) = io::read_bounded(&ready, 256)
            && String::from_utf8_lossy(&bytes).trim() == expected
        {
            let _ = fs::remove_file(&ready); // Completed one-shot handshake; no state or document data.
            if let Some(old) = &installed.obsolete_backup {
                // Prune only the previous installation record's backup after the new GUI is healthy.
                if let Some(expected) =
                    old.file_name().and_then(|n| n.to_str()).and_then(|s| s.strip_prefix(".mira-previous-"))
                    && old != &installed.backup
                    && fs::symlink_metadata(old).is_ok_and(|m| m.is_file() && m.uid() == io::uid().unwrap_or(u32::MAX))
                    && io::sha256(old, &AtomicBool::new(false)).is_ok_and(|h| h == expected)
                {
                    let _ = fs::remove_file(old);
                }
            }
            let staged = installed
                .binary
                .with_file_name(format!(".mira-update-{}-{}.pending", installed.version, installed.new_hash));
            let _ = fs::remove_dir_all(notices_path(&staged)); // This completed update's notice staging only.
            // The new GUI is ready; the caller may now close the old window.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            installed.rollback()?;
            return Err(format!("Updated Mira exited during startup ({status}); previous version restored"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // Do not kill a potentially interactive new window, or discard the still-running old one.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Err("The new version has not confirmed its window is ready. This window stays open; close it only when the new one is usable.".into())
}

#[cfg(test)]
mod tests {
    use super::super::{Signed, name};
    use super::*;

    struct Fixture {
        dir: PathBuf,
        prepared: Prepared,
        original: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = crate::files::test_dir("update-install");
            let bin = dir.join("bin");
            fs::create_dir(&bin).unwrap();
            let original = b"old Mira remains usable".to_vec();
            fs::write(bin.join("mira"), &original).unwrap();
            let old_hash = io::sha256(&bin.join("mira"), &AtomicBool::new(false)).unwrap();
            let marker = dir.join("share/mira/install.json");
            io::atomic_write(
                &marker,
                &serde_json::to_vec(&Record { format: 1, binary_sha256: old_hash, previous_binary_sha256: None })
                    .unwrap(),
                0o600,
            )
            .unwrap();
            let target = Target::detect(&bin.join("mira")).unwrap();
            let mut elf = vec![0_u8; 128];
            elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
            elf[18] = if std::env::consts::ARCH == "x86_64" { 62 } else { 183 };
            let path = bin.join("staged");
            io::private_dir(&notices_path(&path)).unwrap();
            for name in NOTICES {
                fs::write(notices_path(&path).join(name), b"new dependency notices").unwrap();
            }
            fs::write(&path, &elf).unwrap();
            let hash = io::sha256(&path, &AtomicBool::new(false)).unwrap();
            let version = env!("CARGO_PKG_VERSION").to_string();
            let manifest = Manifest {
                format: 1,
                version: version.clone(),
                arch: std::env::consts::ARCH.into(),
                archive: format!("{}.tar.gz", name(&version, std::env::consts::ARCH)),
                archive_sha256: "a".repeat(64),
                binary_sha256: hash,
                archive_size: 100,
                binary_size: 128,
            };
            let release = Release {
                manifest,
                signed: Signed { json: String::new(), signature: vec![] },
                label: String::new(),
                notes: String::new(),
            };
            Self { dir, prepared: Prepared { target, release, path }, original }
        }
        fn commit(&self) -> Installed {
            commit_prepared(&self.prepared, &AtomicBool::new(false), |p, b| io::atomic_write(p, b, 0o600)).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.dir).unwrap();
        }
    }
    #[test]
    fn swaps_atomically_preserves_backup_and_rolls_back_binary_and_record() {
        let fixture = Fixture::new();
        let installed = fixture.commit();
        assert_eq!(fs::read(&installed.backup).unwrap(), fixture.original);
        assert!(Target::detect(&installed.binary).is_ok());
        assert_ne!(fs::read(&installed.binary).unwrap(), fixture.original);
        installed.rollback().unwrap();
        assert_eq!(fs::read(&installed.binary).unwrap(), fixture.original);
        assert!(Target::detect(&installed.binary).is_ok());
    }
    #[test]
    fn failed_record_write_restores_original_executable() {
        let fixture = Fixture::new();
        let result =
            commit_prepared(&fixture.prepared, &AtomicBool::new(false), |_, _| Err("simulated full disk".into()));
        assert!(result.unwrap_err().contains("previous Mira restored"));
        assert_eq!(fs::read(&fixture.prepared.target.binary).unwrap(), fixture.original);
        assert!(Target::detect(&fixture.prepared.target.binary).is_ok());
    }
    #[test]
    fn rejects_changed_installation_corrupt_staging_unmanaged_paths_and_concurrent_updates() {
        let fixture = Fixture::new();
        let lock = installation_lock(&fixture.prepared.target.binary).unwrap();
        assert!(installation_lock(&fixture.prepared.target.binary).is_err());
        drop(lock);
        fs::write(&fixture.prepared.path, b"tampered").unwrap();
        assert!(commit_prepared(&fixture.prepared, &AtomicBool::new(false), |_, _| Ok(())).is_err());
        assert_eq!(fs::read(&fixture.prepared.target.binary).unwrap(), fixture.original);
        fs::write(&fixture.prepared.target.binary, b"external update").unwrap();
        assert!(fixture.prepared.target.revalidate(&AtomicBool::new(false)).is_err());
        let external = fixture.dir.join("external");
        fs::write(&external, b"unmanaged").unwrap();
        assert!(Target::detect(&external).is_err());
        let link = fixture.dir.join("mira");
        std::os::unix::fs::symlink(&external, &link).unwrap();
        assert!(Target::detect(&link).is_err());
    }
    #[test]
    fn repairs_interrupted_binary_swap_only_with_matching_backup_and_authenticated_release() {
        let fixture = Fixture::new();
        let binary = &fixture.prepared.target.binary;
        let backup = binary.with_file_name(format!(".mira-previous-{}", fixture.prepared.target.old_hash));
        fs::hard_link(binary, &backup).unwrap();
        let pending = fixture.prepared.target.pending_path(&fixture.prepared.release.manifest);
        fs::rename(notices_path(&fixture.prepared.path), notices_path(&pending)).unwrap();
        fs::rename(&fixture.prepared.path, binary).unwrap();
        assert!(Target::detect(binary).is_err());
        assert!(repair_record(binary, &fixture.prepared.release, &AtomicBool::new(false)).is_ok());
        assert!(Target::detect(binary).is_ok());
    }
    #[test]
    fn rollback_refuses_to_overwrite_an_external_replacement() {
        let fixture = Fixture::new();
        let installed = fixture.commit();
        fs::write(&installed.binary, b"external replacement").unwrap();
        assert!(installed.rollback().is_err());
        assert_eq!(fs::read(&installed.binary).unwrap(), b"external replacement");
        assert_eq!(fs::read(&installed.backup).unwrap(), fixture.original);
    }
}
