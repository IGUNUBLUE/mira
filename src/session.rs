//! Agent sessions: a terminal running an agent CLI (or a plain shell), plus
//! the turns (prompt → resulting file changes) mira observed in it.

use crate::git::Change;
use std::time::{Duration, Instant};

/// Known agent CLIs. Only presets found on `$PATH` are offered.
pub const PRESETS: &[(&str, &str)] = &[
    ("codex", "codex"),
    ("kiro", "kiro-cli"),
    ("claude", "claude"),
    ("gemini", "gemini"),
    ("opencode", "opencode"),
    ("aider", "aider"),
];

pub fn available_presets() -> Vec<(&'static str, &'static str)> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<_> = std::env::split_paths(&path).collect();
    PRESETS
        .iter()
        .copied()
        .filter(|(_, bin)| dirs.iter().any(|d| d.join(bin).is_file()))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Producing output.
    Working,
    /// Quiet after working: probably waiting for you.
    Waiting,
    /// Quiet, nothing pending.
    Idle,
    Exited,
}

/// Silence that turns "working" into "waiting".
pub const QUIET_AFTER: Duration = Duration::from_millis(3500);

#[derive(Debug, Clone)]
pub struct Turn {
    pub label: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub files: Option<Vec<Change>>,
    pub started: Instant,
    /// An "after" snapshot is in flight.
    pub snapshotting: bool,
}

impl Turn {
    pub fn running(&self) -> bool {
        self.after.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Escape {
    None,
    Start,
    Csi,
    Ss3,
}

pub struct Session {
    pub id: u64,
    pub name: String,
    pub agent: Option<&'static str>,
    pub term: iced_term::Terminal,
    pub status: Status,
    pub last_output: Instant,
    pub last_input: Instant,
    pub attention: bool,
    /// What the user is typing (best effort; reset on Enter).
    pub input: String,
    escape: Escape,
    pub turns: Vec<Turn>,
    /// Tree at session start (for "all changes in this session").
    pub base: Option<String>,
    pub latest: Option<String>,
}

impl Session {
    pub fn new(id: u64, name: String, agent: Option<&'static str>, term: iced_term::Terminal) -> Self {
        Self {
            id,
            name,
            agent,
            term,
            status: Status::Idle,
            last_output: Instant::now(),
            last_input: Instant::now(),
            attention: false,
            input: String::new(),
            escape: Escape::None,
            turns: Vec::new(),
            base: None,
            latest: None,
        }
    }

    /// Feed bytes the user sent to the PTY. Returns `Some(line)` when Enter
    /// submits a line.
    pub fn track_input(&mut self, bytes: &[u8]) -> Option<String> {
        self.last_input = Instant::now();
        self.attention = false;
        let text = String::from_utf8_lossy(bytes);
        let mut submitted = None;
        for c in text.chars() {
            // Skip escape sequences: CSI (`ESC [ … final`), SS3 (`ESC O x`)
            // and Alt+key (`ESC x`).
            match self.escape {
                Escape::None => {}
                Escape::Start => {
                    self.escape = match c {
                        '[' => Escape::Csi,
                        'O' => Escape::Ss3,
                        _ => Escape::None,
                    };
                    continue;
                }
                Escape::Csi => {
                    if ('\x40'..='\x7e').contains(&c) {
                        self.escape = Escape::None;
                    }
                    continue;
                }
                Escape::Ss3 => {
                    self.escape = Escape::None;
                    continue;
                }
            }
            match c {
                '\x1b' => self.escape = Escape::Start,
                '\r' | '\n' => {
                    let line = self.input.trim().to_string();
                    self.input.clear();
                    if !line.is_empty() {
                        submitted = Some(line);
                    }
                }
                '\x7f' | '\x08' => {
                    self.input.pop();
                }
                '\x15' => self.input.clear(), // Ctrl+U
                c if c.is_control() => {}
                c => self.input.push(c),
            }
        }
        submitted
    }

    pub fn running_turn(&self) -> Option<usize> {
        self.turns.iter().rposition(Turn::running)
    }

    pub fn changed_files(&self) -> usize {
        let mut paths: Vec<&str> = self
            .turns
            .iter()
            .filter_map(|t| t.files.as_ref())
            .flatten()
            .map(|c| c.path.as_str())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let term = iced_term::Terminal::new(0, Default::default()).expect("pty");
        Session::new(0, "t".into(), None, term)
    }

    #[test]
    fn tracks_typed_line_with_edits_and_escapes() {
        let mut s = session();
        assert_eq!(s.track_input(b"agrega testz"), None);
        assert_eq!(s.track_input(b"\x7fs\x1b[D"), None);
        assert_eq!(s.track_input(b"\r").as_deref(), Some("agrega tests"));
        assert_eq!(s.track_input(b"\r"), None);
    }

    #[test]
    fn skips_ss3_alt_and_bracketed_paste_markers() {
        let mut s = session();
        // SS3 arrow, Alt+b, bracketed paste around "hola".
        assert_eq!(s.track_input(b"a\x1bOAb\x1bbc\x1b[200~hola\x1b[201~\r").as_deref(), Some("abchola"));
    }
}
