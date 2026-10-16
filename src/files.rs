//! Reading the text files the editor and search show.

use std::io::Read;
use std::path::Path;

/// Leading bytes checked for a NUL to tell binary files apart (as git does).
const SNIFF_BYTES: usize = 8192;

/// The UTF-8 text of `path`, unless it is larger than `max` bytes, binary (a
/// NUL in its first 8 KiB) or not valid UTF-8.
pub fn read_text(path: &Path, max: u64) -> Result<String, String> {
    let file = open_regular(path)?;
    if file.metadata().is_ok_and(|m| m.len() > max) {
        return Err("file too large".into());
    }
    // Read one byte past the limit, so a file that grew meanwhile is caught too.
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err("file too large".into());
    }
    if bytes.iter().take(SNIFF_BYTES).any(|&b| b == 0) {
        return Err("binary file".into());
    }
    String::from_utf8(bytes).map_err(|_| "not valid UTF-8".into())
}

/// Open `path` for reading if it is a regular file (or a link to one). A FIFO
/// would block the open until something writes to it, and a device never ends.
pub fn open_regular(path: &Path) -> Result<std::fs::File, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    std::fs::File::open(path).map_err(|e| e.to_string())
}

/// Replace what the existing file `path` holds without ever leaving half of
/// it on disk (a full disk or a crash mid-write would): a temporary file next
/// to it, renamed over it, with its permissions. A link is followed, so the
/// file it points at is the one replaced. A file that can't be written stays
/// refused; in a folder mira can't create files in, or when it's gone, it is
/// written in place.
pub fn rewrite(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    // Opening it for writing (without truncating) asks the file's own permissions.
    let meta = match std::fs::OpenOptions::new().write(true).open(&target) {
        Ok(f) => f.metadata().map_err(|e| e.to_string())?,
        // Gone (deleted on disk): created anew, with the usual permissions.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return std::fs::write(&target, bytes).map_err(|e| e.to_string());
        }
        Err(e) => return Err(e.to_string()),
    };
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = target.with_file_name(format!(".{name}.mira-{}.tmp", std::process::id()));
    let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return std::fs::write(&target, bytes).map_err(|e| e.to_string());
        }
        Err(e) => return Err(e.to_string()),
    };
    let result = file
        .write_all(bytes)
        .and_then(|()| file.set_permissions(meta.permissions()))
        .and_then(|()| std::fs::rename(&tmp, &target));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())
}

/// A fresh, empty temporary directory, unique per call (process id and counter).
#[cfg(test)]
pub fn test_dir(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let dir =
        std::env::temp_dir().join(format!("mira-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_and_refuses_large_binary_and_invalid_files() {
        let dir = test_dir("files");
        let p = dir.join("f");
        std::fs::write(&p, "héllo\n").unwrap();
        assert_eq!(read_text(&p, 100).as_deref(), Ok("héllo\n"));
        assert_eq!(read_text(&p, 3).unwrap_err(), "file too large");
        std::fs::write(&p, b"a\0b").unwrap();
        assert_eq!(read_text(&p, 100).unwrap_err(), "binary file");
        std::fs::write(&p, b"\xff\xfe").unwrap();
        assert_eq!(read_text(&p, 100).unwrap_err(), "not valid UTF-8");
        assert!(read_text(&dir.join("missing"), 100).is_err());
        // A FIFO with no writer would block the open forever.
        let fifo = dir.join("pipe");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        assert_eq!(read_text(&fifo, 100).unwrap_err(), "not a regular file");
        assert!(read_text(&dir, 100).is_err());
        // A read-only file stays refused, even though its folder is writable.
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&p, "keep\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
        let refused = rewrite(&p, b"x");
        // (Root may write anyway.)
        if refused.is_err() {
            assert_eq!(std::fs::read_to_string(&p).unwrap(), "keep\n");
        }
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        rewrite(&p, b"new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new\n");
        let _ = std::fs::remove_dir_all(dir);
    }
}
