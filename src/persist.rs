//! Per-folder state (the agent runs mira recorded) and the user config file.
//!
//! State lives in `$XDG_STATE_HOME/mira/<hash of folder>.json` (0600): prompts
//! may be sensitive, so it never goes inside the project. herdr keeps the
//! agents themselves alive across mira restarts.

use crate::runs::Run;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct State {
    pub root: PathBuf,
    /// Older state files (agent sessions) have no runs; they load empty.
    #[serde(default)]
    pub runs: Vec<Run>,
}

/// [`State`] as written: borrowed, so saving never copies the runs.
#[derive(Serialize)]
struct StateRef<'a> {
    root: &'a Path,
    runs: &'a [Run],
}

pub fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn now() -> u64 {
    unix(SystemTime::now())
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

fn xdg(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
}

/// `$XDG_STATE_HOME/mira`.
pub fn state_dir() -> Option<PathBuf> {
    xdg("XDG_STATE_HOME", ".local/state").map(|d| d.join("mira"))
}

/// `$XDG_CONFIG_HOME/mira`.
pub fn config_dir() -> Option<PathBuf> {
    xdg("XDG_CONFIG_HOME", ".config").map(|d| d.join("mira"))
}

pub fn state_path(root: &Path) -> Option<PathBuf> {
    let key = fnv1a(root.as_os_str().as_encoded_bytes());
    state_dir().map(|d| d.join(format!("{key:016x}.json")))
}

pub fn load(root: &Path) -> Option<State> {
    load_from(&state_path(root)?, root)
}

fn load_from(path: &Path, root: &Path) -> Option<State> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<State>(&text) {
        Ok(state) => (state.root == root).then_some(state),
        Err(_) => {
            // Keep an unreadable file for inspection instead of overwriting it.
            let _ = std::fs::rename(path, path.with_extension("json.corrupt"));
            None
        }
    }
}

/// Save the runs of folder `root`.
pub fn save(root: &Path, runs: &[Run]) -> Result<(), String> {
    save_to(&state_path(root).ok_or("no state directory")?, &StateRef { root, runs })
}

fn save_to(path: &Path, state: &StateRef<'_>) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    write_private(path, &json)
}

/// Change the saved runs of folder `root` (one no longer shown) through `f`,
/// which says whether it changed anything to write.
pub fn edit_runs(root: &Path, f: impl FnOnce(&mut [Run]) -> bool) -> Result<(), String> {
    edit_runs_at(&state_path(root).ok_or("no state directory")?, root, f)
}

fn edit_runs_at(path: &Path, root: &Path, f: impl FnOnce(&mut [Run]) -> bool) -> Result<(), String> {
    let Some(mut state) = load_from(path, root) else { return Ok(()) };
    if f(&mut state.runs) { save_to(path, &StateRef { root, runs: &state.runs }) } else { Ok(()) }
}

/// Write `bytes` to `path` readable only by the user (0600), atomically.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_atomic(path, bytes, 0o600)
}

/// Write `bytes` to `path` with permissions `mode`, atomically: through a
/// temporary file in the same directory, then a rename, so a crash never
/// leaves half a file.
fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}-{n}.tmp", std::process::id()));
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(mode).open(&tmp)?;
        // `mode` above is reduced by the umask; set it exactly.
        f.set_permissions(std::fs::Permissions::from_mode(mode))?;
        f.write_all(bytes)?;
        std::fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        // Nothing to clean up if the temporary file was never created.
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// `$XDG_STATE_HOME/mira/<name>`, created if needed and kept 0700, for
/// scratch files other users must not read.
pub fn private_dir(name: &str) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = state_dir().ok_or("no state directory")?.join(name);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// The folder mira showed last, for a start without a folder or herdr.
pub fn last_root() -> Option<PathBuf> {
    let text = std::fs::read_to_string(state_dir()?.join("last_root")).ok()?;
    Some(PathBuf::from(text.trim_end_matches('\n'))).filter(|p| p.is_dir())
}

pub fn set_last_root(root: &Path) -> Result<(), String> {
    write_private(&state_dir().ok_or("no state directory")?.join("last_root"), root.as_os_str().as_encoded_bytes())
}

// ------------------------------------------------------------------ config

/// `$XDG_CONFIG_HOME/mira/config`: `key = value` lines, `#` comments.
fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config"))
}

pub fn config_get(key: &str) -> Option<String> {
    get_in(&std::fs::read_to_string(config_path()?).ok()?, key)
}

/// Set `key`, keeping every other line (and comments) as they are. The file
/// is replaced atomically; its mode is kept (0644 for a new one), and a
/// symlinked config (dotfiles managers) stays a symlink.
pub fn config_set(key: &str, value: &str) -> Result<(), String> {
    set_at(&config_path().ok_or("no config directory")?, key, value)
}

/// `config_set` on the file at `path`. Only a missing file starts empty: a
/// file mira can't read (permissions, bytes that aren't UTF-8) is left alone,
/// or writing would replace everything in it with this one key.
fn set_at(path: &Path, key: &str, value: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = match std::fs::read(&path) {
        Ok(bytes) => {
            String::from_utf8(bytes).map_err(|_| format!("{} is not valid UTF-8: not changed", path.display()))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mode = std::fs::metadata(&path).map_or(0o644, |m| m.permissions().mode() & 0o7777);
    write_atomic(&path, set_in(&text, key, value)?.as_bytes(), mode)
}

fn get_in(text: &str, key: &str) -> Option<String> {
    text.lines().filter(|l| !l.trim_start().starts_with('#')).find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// `text` with `key = "value"`. Values are written in quotes with no escaping
/// (and read back by stripping them), so a quote or line break is refused.
fn set_in(text: &str, key: &str, value: &str) -> Result<String, String> {
    if value.contains(['"', '\n', '\r']) {
        return Err(format!("{key}: the value can't contain quotes or line breaks"));
    }
    let line = format!("{key} = \"{value}\"");
    let mut found = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|l| match l.split_once('=') {
            Some((k, _)) if !l.trim_start().starts_with('#') && k.trim() == key && !found => {
                found = true;
                line.clone()
            }
            _ => l.to_string(),
        })
        .collect();
    if !found {
        lines.push(line);
    }
    Ok(lines.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::PaneId;
    use crate::runs::RunId;

    #[test]
    fn an_unreadable_config_is_never_replaced() {
        let dir = crate::files::test_dir("persist-config");
        let path = dir.join("config");
        // A Latin-1 comment: not UTF-8.
        let latin1 = b"# caf\xe9\ntheme = \"auto\"\n".to_vec();
        std::fs::write(&path, &latin1).unwrap();
        assert!(set_at(&path, "theme", "x").unwrap_err().contains("not valid UTF-8"));
        assert_eq!(std::fs::read(&path).unwrap(), latin1);
        // Missing: created with the key.
        let new = dir.join("new");
        set_at(&new, "theme", "x").unwrap();
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "theme = \"x\"\n");
        // Unreadable (a directory where the file should be): an error, nothing written.
        std::fs::create_dir(dir.join("adir")).unwrap();
        assert!(set_at(&dir.join("adir"), "theme", "x").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn config_set_keeps_other_lines() {
        let text = set_in("# mine\nfont = x\ntheme = auto\n", "theme", "LAGC Calm Dark").unwrap();
        assert_eq!(text, "# mine\nfont = x\ntheme = \"LAGC Calm Dark\"\n");
        assert_eq!(get_in(&text, "theme").as_deref(), Some("LAGC Calm Dark"));
        assert_eq!(get_in("# theme = x\n", "theme"), None);
        assert_eq!(set_in("", "a", "1").unwrap(), "a = \"1\"\n");
        assert!(set_in("", "a", "say \"hi\"").is_err());
        assert!(set_in("", "a", "x\ny = 1").is_err());
    }

    #[test]
    fn atomic_writes_set_the_mode_and_leave_no_temp_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::files::test_dir("persist-write");
        let path = dir.join("config");
        write_atomic(&path, b"a = \"1\"\n", 0o644).unwrap();
        write_private(&dir.join("secret"), b"x").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&path), mode(&dir.join("secret"))), (0o644, 0o600));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = \"1\"\n");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "no temporary file left");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn state_round_trips_is_keyed_by_folder_and_reads_old_files() {
        let dir = crate::files::test_dir("persist-state");
        let path = dir.join("s.json");
        let run = Run {
            id: RunId(3),
            pane: PaneId("w1:p1".into()),
            agent: "codex".into(),
            kind: Some("codex".into()),
            dir: PathBuf::from("/tmp/p"),
            prompt: Some("hola".into()),
            before: Some("abc".into()),
            after: None,
            started: 10,
            finished: None,
            seen: Vec::new(),
            files: None,
        };
        let runs = [run];
        save_to(&path, &StateRef { root: Path::new("/tmp/p"), runs: &runs }).expect("save");
        let back = load_from(&path, Path::new("/tmp/p")).expect("load");
        assert_eq!(back.runs[0].id, RunId(3));
        assert_eq!(back.runs[0].prompt.as_deref(), Some("hola"));
        assert!(load_from(&path, Path::new("/tmp/other")).is_none());

        // A snapshot landing after the folder was left goes into its file.
        let mut finished = runs[0].clone();
        finished.finished = Some(20);
        save_to(&path, &StateRef { root: Path::new("/tmp/p"), runs: &[finished] }).expect("save");
        let store = |runs: &mut [Run]| {
            runs.iter_mut().find(|r| r.id == RunId(3)).is_some_and(|r| r.store_tree(true, "def".into()))
        };
        edit_runs_at(&path, Path::new("/tmp/p"), store).expect("edit");
        let back = load_from(&path, Path::new("/tmp/p")).expect("load");
        assert_eq!(back.runs[0].trees(), Some(("abc", "def")));
        // Nothing for another folder's file, or a file that isn't there.
        assert!(edit_runs_at(&path, Path::new("/tmp/other"), |_| panic!("not its runs")).is_ok());
        assert!(edit_runs_at(&dir.join("none.json"), Path::new("/tmp/p"), |_| true).is_ok());
        assert!(!dir.join("none.json").exists());

        // A state file from the sessions era loads with no runs.
        std::fs::write(&path, r#"{"root":"/tmp/p","active":0,"sessions":[{"name":"kiro"}]}"#).expect("write");
        assert!(load_from(&path, Path::new("/tmp/p")).expect("old file").runs.is_empty());

        std::fs::write(&path, "{not json").expect("write");
        assert!(load_from(&path, Path::new("/tmp/p")).is_none());
        assert!(path.with_extension("json.corrupt").exists(), "corrupt file kept aside");
        let _ = std::fs::remove_dir_all(dir);
    }
}
