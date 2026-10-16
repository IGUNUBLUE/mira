//! File operations of the Files panel: new file or folder, rename, and
//! delete to the desktop's trash (the freedesktop.org home trash, as file
//! managers and VSCodium do), so a delete can be undone.

use std::io::Write;
use std::path::{Path, PathBuf};

/// A file or folder moved to the trash, and how to put it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trashed {
    pub original: PathBuf,
    /// Where it is now, in the trash's `files/`.
    pub stored: PathBuf,
    /// Its `.trashinfo`, removed on restore.
    pub info: PathBuf,
}

/// The home trash: `$XDG_DATA_HOME/Trash`, else `~/.local/share/Trash`.
pub fn home_trash() -> Result<PathBuf, String> {
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(d) => Ok(PathBuf::from(d).join("Trash")),
        None => std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".local/share/Trash"))
            .ok_or_else(|| "no HOME for the trash".to_string()),
    }
}

/// A name for one entry of a folder: not empty, no `/`, not `.` or `..`.
pub fn check_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A name is needed".into());
    }
    if name.contains('/') || name.contains('\0') {
        return Err("A name can't contain '/'".into());
    }
    if name == "." || name == ".." {
        return Err(format!("'{name}' is not a valid name"));
    }
    Ok(name)
}

/// Create an empty file or a folder `name` in `dir`; never replaces anything.
pub fn create(dir: &Path, name: &str, folder: bool) -> Result<PathBuf, String> {
    let path = dir.join(check_name(name)?);
    let result = if folder {
        std::fs::create_dir(&path)
    } else {
        std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map(drop)
    };
    match result {
        Ok(()) => Ok(path),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(format!("'{}' already exists", check_name(name).unwrap_or(name)))
        }
        Err(e) => Err(format!("{name}: {e}")),
    }
}

/// Rename `path` to `name` in the same folder; never replaces anything.
pub fn rename(path: &Path, name: &str) -> Result<PathBuf, String> {
    let name = check_name(name)?;
    let to = path.with_file_name(name);
    if to == path {
        return Ok(to);
    }
    // Something else is there; on case-insensitive disks `A.txt` may be this very entry.
    if let Ok(there) = to.symlink_metadata() {
        use std::os::unix::fs::MetadataExt;
        let here = path.symlink_metadata().map_err(|e| e.to_string())?;
        if (there.dev(), there.ino()) != (here.dev(), here.ino()) {
            return Err(format!("'{name}' already exists"));
        }
    }
    std::fs::rename(path, &to).map_err(|e| format!("{}: {e}", file_name(path)))?;
    Ok(to)
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `Path=` of a `.trashinfo`: the absolute path, percent-encoded but for `/`
/// and the unreserved characters (as RFC 3986 and the spec ask).
fn encode_path(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Local time as `YYYY-MM-DDThh:mm:ss`, for `DeletionDate` (coreutils
/// `date`; the spec asks for local time, which std can't give).
fn deletion_date() -> String {
    std::process::Command::new("date")
        .arg("+%Y-%m-%dT%H:%M:%S")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Move `path` into `trash` (with `files/` and `info/`, created if needed).
/// Fails, leaving the file where it is, when the trash is on another disk.
pub fn trash(path: &Path, trash: &Path) -> Result<Trashed, String> {
    let original = std::path::absolute(path).map_err(|e| e.to_string())?;
    let name = file_name(&original);
    if name.is_empty() {
        return Err("nothing to delete".into());
    }
    let (files, info_dir) = (trash.join("files"), trash.join("info"));
    for d in [&files, &info_dir] {
        std::fs::create_dir_all(d).map_err(|e| format!("trash: {e}"))?;
    }
    // A free name: `x`, then `x.2`, `x.3`… The info file is claimed first
    // (create_new), so two deletes of the same name never collide.
    let date = deletion_date();
    for n in 1..10_000 {
        let stored_name = if n == 1 { name.clone() } else { format!("{name}.{n}") };
        let info = info_dir.join(format!("{stored_name}.trashinfo"));
        let stored = files.join(&stored_name);
        if stored.symlink_metadata().is_ok() {
            continue;
        }
        let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).open(&info) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("trash: {e}")),
        };
        let text = format!("[Trash Info]\nPath={}\nDeletionDate={date}\n", encode_path(&original));
        if let Err(e) = f.write_all(text.as_bytes()).and_then(|()| std::fs::rename(&original, &stored)) {
            // Cleanup of the claim: the file was not moved.
            let _ = std::fs::remove_file(&info);
            return Err(if e.raw_os_error() == Some(18) {
                format!("{name}: on another disk than the trash, not deleted")
            } else {
                format!("{name}: {e}")
            });
        }
        return Ok(Trashed { original, stored, info });
    }
    Err(format!("{name}: the trash has too many items with this name"))
}

/// Put a trashed entry back; refuses to replace something created since.
pub fn restore(t: &Trashed) -> Result<(), String> {
    if t.original.symlink_metadata().is_ok() {
        return Err(format!("'{}' exists again, not restored", file_name(&t.original)));
    }
    std::fs::rename(&t.stored, &t.original).map_err(|e| format!("{}: {e}", file_name(&t.original)))?;
    // Gone already only if the user emptied that entry: nothing left to clean.
    let _ = std::fs::remove_file(&t.info);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked() {
        assert_eq!(check_name("  a.txt "), Ok("a.txt"));
        assert!(check_name("").is_err());
        assert!(check_name("a/b").is_err());
        assert!(check_name("..").is_err());
        assert_eq!(encode_path(Path::new("/home/x/my file%ñ.txt")), "/home/x/my%20file%25%C3%B1.txt");
    }

    #[test]
    fn creates_and_renames_without_replacing() {
        let dir = crate::files::test_dir("fileops");
        let f = create(&dir, "a.txt", false).unwrap();
        assert!(f.is_file());
        assert!(create(&dir, "a.txt", false).unwrap_err().contains("already exists"));
        let d = create(&dir, "sub", true).unwrap();
        assert!(d.is_dir());
        assert!(rename(&f, "sub").unwrap_err().contains("already exists"));
        let g = rename(&f, "b.txt").unwrap();
        assert!(g.is_file() && !f.exists());
        // Changing only the case is allowed.
        let h = rename(&g, "B.txt").unwrap();
        assert!(h.is_file());
        // …but never over another file whose name differs only in case.
        let c = create(&dir, "c.txt", false).unwrap();
        create(&dir, "C.txt", false).unwrap();
        assert!(rename(&c, "C.txt").unwrap_err().contains("already exists"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn trash_and_restore_files_and_folders() {
        let dir = crate::files::test_dir("fileops-trash");
        let trash_dir = dir.join("Trash");
        std::fs::create_dir_all(dir.join("w/sub")).unwrap();
        std::fs::write(dir.join("w/sub/x.txt"), "x").unwrap();
        std::fs::write(dir.join("w/a.txt"), "1").unwrap();

        let t = trash(&dir.join("w/a.txt"), &trash_dir).unwrap();
        assert!(!dir.join("w/a.txt").exists());
        assert_eq!(std::fs::read_to_string(&t.stored).unwrap(), "1");
        let info = std::fs::read_to_string(&t.info).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath=/") && info.contains("/w/a.txt\nDeletionDate=20"), "{info}");

        // Same name again: stored next to it.
        std::fs::write(dir.join("w/a.txt"), "2").unwrap();
        let t2 = trash(&dir.join("w/a.txt"), &trash_dir).unwrap();
        assert!(t2.stored.ends_with("files/a.txt.2"));

        let folder = trash(&dir.join("w/sub"), &trash_dir).unwrap();
        assert!(!dir.join("w/sub").exists());
        restore(&folder).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("w/sub/x.txt")).unwrap(), "x");
        assert!(!folder.info.exists());

        restore(&t2).unwrap();
        // The first one can't come back over the second.
        assert!(restore(&t).unwrap_err().contains("exists again"));
        assert_eq!(std::fs::read_to_string(dir.join("w/a.txt")).unwrap(), "2");
        let _ = std::fs::remove_dir_all(dir);
    }
}
