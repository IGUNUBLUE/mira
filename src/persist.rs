//! Session persistence: what was open in a folder, so closing and reopening
//! mira brings the agents back (resumed via their own CLI flags) together with
//! the turns mira recorded.
//!
//! State lives in `$XDG_STATE_HOME/mira/<hash of folder>.json` (0600). Prompts
//! may be sensitive, so it never goes inside the project.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub root: PathBuf,
    pub active: usize,
    pub sessions: Vec<SavedSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub name: String,
    pub agent: Option<String>,
    pub resume_id: Option<String>,
    /// Folder the agent runs in, when it is below the opened one.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    pub base: Option<String>,
    pub latest: Option<String>,
    pub started_at: u64,
    pub turns: Vec<SavedTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedTurn {
    pub label: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub started_at: u64,
}

pub fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn from_unix(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

fn state_dir() -> Option<PathBuf> {
    match std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        Some(d) => Some(PathBuf::from(d)),
        None => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")),
    }
    .map(|d| d.join("mira"))
}

pub fn state_path(root: &Path) -> Option<PathBuf> {
    let key = fnv1a(root.as_os_str().as_encoded_bytes());
    state_dir().map(|d| d.join(format!("{key:016x}.json")))
}

pub fn load(root: &Path) -> Option<State> {
    let text = std::fs::read_to_string(state_path(root)?).ok()?;
    let state: State = serde_json::from_str(&text).ok()?;
    (state.root == root).then_some(state)
}

pub fn save(state: &State) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = state_path(&state.root).ok_or("no state directory")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    // Write then rename, so a crash never leaves half a file.
    let tmp = path.with_extension("json.tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    f.write_all(&json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

// ------------------------------------------------------------------ config

/// `$XDG_CONFIG_HOME/mira/config`: `key = value` lines, `#` comments.
fn config_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|d| d.join("mira/config"))
}

pub fn config_get(key: &str) -> Option<String> {
    let text = std::fs::read_to_string(config_path()?).ok()?;
    text.lines().filter(|l| !l.trim_start().starts_with('#')).find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// Set `key`, keeping every other line (and comments) as they are.
pub fn config_set(key: &str, value: &str) -> Result<(), String> {
    let path = config_path().ok_or("no config directory")?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
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
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, lines.join("\n") + "\n").map_err(|e| e.to_string())
}

// ------------------------------------------------------- agent conversations

/// One conversation an agent CLI recorded on disk.
struct Conversation {
    id: String,
    cwd: PathBuf,
    created: u64,
    modified: u64,
}

/// Agents whose conversation ids mira can discover.
pub fn discoverable(agent: &str) -> bool {
    matches!(agent, "kiro" | "codex")
}

/// Find the conversation `agent` is running in `root` for a session started
/// at `since`: the first one created after it, or else the one most recently
/// active since then (a resumed conversation). Conversations in folders below
/// `root` count too. `claimed` ids are skipped. Returns `(id, cwd)`.
pub fn find_conversation(agent: &str, root: &Path, since: u64, claimed: &[String]) -> Option<(String, PathBuf)> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let all = match agent {
        "kiro" => kiro_conversations(&home, root),
        "codex" => codex_conversations(&home, root, since),
        _ => return None,
    };
    let free: Vec<Conversation> = all
        .into_iter()
        .filter(|c| crate::session::valid_id(&c.id) && !claimed.contains(&c.id))
        .collect();
    let slack = since.saturating_sub(3);
    let pick = |c: &Conversation| (c.id.clone(), c.cwd.clone());
    if let Some(c) = free.iter().filter(|c| c.created >= slack).min_by_key(|c| c.created) {
        return Some(pick(c));
    }
    free.iter().filter(|c| c.modified >= slack).max_by_key(|c| c.modified).map(pick)
}

/// `~/.kiro/sessions/<hash>/sess_*/session.json`.
fn kiro_conversations(home: &Path, root: &Path) -> Vec<Conversation> {
    let mut out = Vec::new();
    let Ok(groups) = std::fs::read_dir(home.join(".kiro/sessions")) else { return out };
    for group in groups.flatten() {
        let Ok(sessions) = std::fs::read_dir(group.path()) else { continue };
        for s in sessions.flatten() {
            let Ok(text) = std::fs::read_to_string(s.path().join("session.json")) else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            let cwd = v["workspacePaths"]
                .as_array()
                .and_then(|ps| ps.iter().filter_map(|p| p.as_str()).map(Path::new).find(|p| p.starts_with(root)));
            let (Some(id), Some(cwd)) = (v["id"].as_str(), cwd) else { continue };
            out.push(Conversation {
                id: id.to_string(),
                cwd: cwd.to_path_buf(),
                created: v["createdAt"].as_str().and_then(parse_iso_utc).unwrap_or(0),
                modified: v["lastModifiedAt"].as_str().and_then(parse_iso_utc).unwrap_or(0),
            });
        }
    }
    out
}

/// `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`; the first line is the
/// session meta with `id`, `cwd` and `timestamp`.
fn codex_conversations(home: &Path, root: &Path, since: u64) -> Vec<Conversation> {
    use std::io::BufRead;
    let mut out = Vec::new();
    let mut stack = vec![(home.join(".codex/sessions"), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if depth < 3 {
                if path.is_dir() {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let modified = e.metadata().and_then(|m| m.modified()).map(unix).unwrap_or(0);
            if modified < since.saturating_sub(3) || path.extension().is_none_or(|x| x != "jsonl") {
                continue;
            }
            let Ok(f) = std::fs::File::open(&path) else { continue };
            let mut first = String::new();
            if std::io::BufReader::new(f).read_line(&mut first).is_err() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&first) else { continue };
            let p = &v["payload"];
            let Some(cwd) = p["cwd"].as_str().map(Path::new).filter(|c| c.starts_with(root)) else { continue };
            if let Some(id) = p["id"].as_str() {
                out.push(Conversation {
                    id: id.to_string(),
                    cwd: cwd.to_path_buf(),
                    created: p["timestamp"].as_str().and_then(parse_iso_utc).unwrap_or(modified),
                    modified,
                });
            }
        }
    }
    out
}

/// `2026-10-14T17:13:27.934Z` → unix seconds (UTC only).
pub fn parse_iso_utc(s: &str) -> Option<u64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    let mut t = time.split(':');
    let (h, min) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let sec = t.next()?.split('.').next()?.parse::<i64>().ok()?;
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    u64::try_from(days * 86400 + h * 3600 + min * 60 + sec).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_set_keeps_other_lines() {
        let dir = std::env::temp_dir().join(format!("mira-config-{}", std::process::id()));
        // SAFETY: only this test uses XDG_CONFIG_HOME.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        std::fs::create_dir_all(dir.join("mira")).unwrap();
        std::fs::write(dir.join("mira/config"), "# mine\nfont = x\ntheme = auto\n").unwrap();
        config_set("theme", "LAGC Calm Dark").unwrap();
        assert_eq!(config_get("theme").as_deref(), Some("LAGC Calm Dark"));
        assert_eq!(std::fs::read_to_string(dir.join("mira/config")).unwrap(), "# mine\nfont = x\ntheme = \"LAGC Calm Dark\"\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parses_iso_timestamps() {
        assert_eq!(parse_iso_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_utc("2026-10-14T17:13:27.934Z"), Some(1_791_998_007));
        assert_eq!(parse_iso_utc("garbage"), None);
    }

    #[test]
    fn state_round_trips_and_is_keyed_by_folder() {
        let dir = std::env::temp_dir().join(format!("mira-state-{}", std::process::id()));
        // SAFETY: tests in this module are the only users of XDG_STATE_HOME.
        unsafe { std::env::set_var("XDG_STATE_HOME", &dir) };
        let state = State {
            root: PathBuf::from("/tmp/some/project"),
            active: 0,
            sessions: vec![SavedSession {
                name: "kiro".into(),
                agent: Some("kiro".into()),
                resume_id: Some("sess_1".into()),
                cwd: None,
                base: Some("abc".into()),
                latest: None,
                started_at: 10,
                turns: vec![SavedTurn { label: "hola".into(), before: Some("abc".into()), after: None, started_at: 11 }],
            }],
        };
        save(&state).unwrap();
        let back = load(Path::new("/tmp/some/project")).unwrap();
        assert_eq!(back.sessions[0].resume_id.as_deref(), Some("sess_1"));
        assert_eq!(back.sessions[0].turns[0].label, "hola");
        assert!(load(Path::new("/tmp/other")).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
