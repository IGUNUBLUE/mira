//! Agent sessions: a terminal running an agent CLI (or a plain shell), plus
//! the turns (prompt → resulting file changes) mira observed in it.

use crate::git::Change;
use std::time::{Duration, Instant, SystemTime};

/// Known agent CLIs. Only presets found on `$PATH` are offered.
pub const PRESETS: &[(&str, &str)] = &[
    ("codex", "codex"),
    ("kiro", "kiro-cli"),
    ("claude", "claude"),
    ("gemini", "gemini"),
    ("opencode", "opencode"),
    ("aider", "aider"),
];

/// How to reopen an agent's conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    New,
    /// Most recent conversation in this directory.
    Last,
    Id(String),
}

/// Ids are typed into a shell, so only allow a conservative alphabet.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Shell command line that starts (or resumes) `agent`.
pub fn launch_command(agent: &str, resume: &Resume) -> Option<String> {
    let bin = PRESETS.iter().find(|(n, _)| *n == agent)?.1;
    let resume = match resume {
        Resume::Id(id) if !valid_id(id) => &Resume::Last,
        r => r,
    };
    Some(match (agent, resume) {
        (_, Resume::New) => bin.to_string(),
        ("codex", Resume::Last) => "codex resume --last".into(),
        ("codex", Resume::Id(id)) => format!("codex resume {id}"),
        ("kiro", Resume::Last) => "kiro-cli --resume".into(),
        ("kiro", Resume::Id(id)) => format!("kiro-cli --resume-id {id}"),
        ("claude", Resume::Last) => "claude --continue".into(),
        ("claude", Resume::Id(id)) => format!("claude --resume {id}"),
        ("opencode", Resume::Last) => "opencode --continue".into(),
        ("opencode", Resume::Id(id)) => format!("opencode --session {id}"),
        ("gemini", _) => "gemini --resume".into(),
        ("aider", _) => "aider --restore-chat-history".into(),
        _ => bin.to_string(),
    })
}

/// If `line` launches a known agent CLI, its preset name and the resume id
/// given on the command line, if any.
pub fn detect_launch(line: &str) -> Option<(&'static str, Option<String>)> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let bin = words.first()?.rsplit('/').next()?;
    let name = PRESETS.iter().find(|(_, b)| *b == bin)?.0;
    let flag = match name {
        "kiro" => "--resume-id",
        "claude" => "--resume",
        "opencode" => "--session",
        _ => "",
    };
    let id = match name {
        "codex" if words.get(1) == Some(&"resume") => words.get(2).filter(|w| !w.starts_with('-')),
        _ => words.iter().position(|w| *w == flag).and_then(|i| words.get(i + 1)),
    }
    .filter(|id| valid_id(id))
    .map(|id| id.to_string());
    Some((name, id))
}

/// Single-quote `s` for a POSIX shell.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

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
    /// The agent's own conversation id, once discovered (for resuming).
    pub resume_id: Option<String>,
    /// Folder the agent runs in, if not the opened one.
    pub cwd: Option<std::path::PathBuf>,
    /// Wall-clock start, to find the agent's conversation created after it.
    pub started_at: SystemTime,
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
            resume_id: None,
            cwd: None,
            started_at: SystemTime::now(),
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
    fn detects_agents_launched_by_hand() {
        assert_eq!(detect_launch("kiro-cli"), Some(("kiro", None)));
        assert_eq!(detect_launch("kiro-cli --resume-id sess_1"), Some(("kiro", Some("sess_1".into()))));
        assert_eq!(detect_launch("~/.local/bin/codex resume abc-1"), Some(("codex", Some("abc-1".into()))));
        assert_eq!(detect_launch("codex resume --last"), Some(("codex", None)));
        assert_eq!(detect_launch("ls -la"), None);
    }

    #[test]
    fn quotes_paths_for_the_shell() {
        assert_eq!(shell_quote("/a b/it's"), "'/a b/it'\\''s'");
    }

    #[test]
    fn resume_commands_reject_unsafe_ids() {
        assert_eq!(launch_command("kiro", &Resume::Id("sess_ab-12".into())).as_deref(), Some("kiro-cli --resume-id sess_ab-12"));
        assert_eq!(launch_command("codex", &Resume::Id("x; rm -rf ~".into())).as_deref(), Some("codex resume --last"));
        assert_eq!(launch_command("codex", &Resume::New).as_deref(), Some("codex"));
        assert_eq!(launch_command("nope", &Resume::Last), None);
    }

    #[test]
    fn skips_ss3_alt_and_bracketed_paste_markers() {
        let mut s = session();
        // SS3 arrow, Alt+b, bracketed paste around "hola".
        assert_eq!(s.track_input(b"a\x1bOAb\x1bbc\x1b[200~hola\x1b[201~\r").as_deref(), Some("abchola"));
    }
}
