//! Per-folder state (the agent runs mira recorded) and the user config file.
//!
//! State lives in `$XDG_STATE_HOME/mira/<hash of folder>.json` (0600): prompts
//! may be sensitive, so it never goes inside the project. herdr keeps the
//! agents themselves alive across mira restarts.

use crate::runs::Run;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub root: PathBuf,
    /// Older state files (agent sessions) have no runs; they load empty.
    #[serde(default)]
    pub runs: Vec<Run>,
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

pub fn save(state: &State) -> Result<(), String> {
    save_to(&state_path(&state.root).ok_or("no state directory")?, state)
}

fn save_to(path: &Path, state: &State) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    write_private(path, &json)
}

/// Write `bytes` to `path` readable only by the user (0600), atomically.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = bytes;
    // Write then rename, so a crash never leaves half a file.
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    f.write_all(json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

// ------------------------------------------------------------------ config

/// `$XDG_CONFIG_HOME/mira/config`: `key = value` lines, `#` comments.
fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config"))
}

pub fn config_get(key: &str) -> Option<String> {
    get_in(&std::fs::read_to_string(config_path()?).ok()?, key)
}

/// Set `key`, keeping every other line (and comments) as they are.
pub fn config_set(key: &str, value: &str) -> Result<(), String> {
    let path = config_path().ok_or("no config directory")?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, set_in(&text, key, value)).map_err(|e| e.to_string())
}

fn get_in(text: &str, key: &str) -> Option<String> {
    text.lines().filter(|l| !l.trim_start().starts_with('#')).find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

fn set_in(text: &str, key: &str, value: &str) -> String {
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
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::PaneId;
    use crate::runs::RunId;

    #[test]
    fn config_set_keeps_other_lines() {
        let text = set_in("# mine\nfont = x\ntheme = auto\n", "theme", "LAGC Calm Dark");
        assert_eq!(text, "# mine\nfont = x\ntheme = \"LAGC Calm Dark\"\n");
        assert_eq!(get_in(&text, "theme").as_deref(), Some("LAGC Calm Dark"));
        assert_eq!(get_in("# theme = x\n", "theme"), None);
        assert_eq!(set_in("", "a", "1"), "a = \"1\"\n");
    }

    #[test]
    fn state_round_trips_is_keyed_by_folder_and_reads_old_files() {
        let dir = std::env::temp_dir().join(format!("mira-state-{}", std::process::id()));
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
        let state = State { root: PathBuf::from("/tmp/p"), runs: vec![run] };
        save_to(&path, &state).expect("save");
        let back = load_from(&path, Path::new("/tmp/p")).expect("load");
        assert_eq!(back.runs[0].id, RunId(3));
        assert_eq!(back.runs[0].prompt.as_deref(), Some("hola"));
        assert!(load_from(&path, Path::new("/tmp/other")).is_none());

        // A state file from the sessions era loads with no runs.
        std::fs::write(&path, r#"{"root":"/tmp/p","active":0,"sessions":[{"name":"kiro"}]}"#).expect("write");
        assert!(load_from(&path, Path::new("/tmp/p")).expect("old file").runs.is_empty());

        std::fs::write(&path, "{not json").expect("write");
        assert!(load_from(&path, Path::new("/tmp/p")).is_none());
        assert!(path.with_extension("json.corrupt").exists(), "corrupt file kept aside");
        let _ = std::fs::remove_dir_all(dir);
    }
}
