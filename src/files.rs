//! Reading the text files the editor and search show.

use std::io::Read;
use std::path::Path;

/// Leading bytes checked for a NUL to tell binary files apart (as git does).
const SNIFF_BYTES: usize = 8192;

/// The UTF-8 text of `path`, unless it is larger than `max` bytes, binary (a
/// NUL in its first 8 KiB) or not valid UTF-8.
pub fn read_text(path: &Path, max: u64) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
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
        let _ = std::fs::remove_dir_all(dir);
    }
}
