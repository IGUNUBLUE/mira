//! Application state and update logic. Views live in `view.rs`.
//!
//! Model: the user talks to agents in terminal *sessions*. Every prompt sent
//! to a session opens a *turn*; when the agent goes quiet mira snapshots the
//! working tree, so each turn has a reviewable before/after diff.

use crate::diff::{self, FileDiff};
use crate::explorer::Explorer;
use crate::git::{self, Change, Snapshot};
use crate::search::{self, FileMatches, Options};
use crate::persist;
use crate::session::{self, Resume, Session, Status, Turn};
use crate::theme;

use iced::keyboard;
use iced::widget::pane_grid::{self, Axis, Configuration};
use iced::widget::text_editor::{self, Content};
use iced::{Subscription, Task};
use iced_term::{AlacrittyEvent, BackendCommand};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

pub const CODE_SIZE: u32 = 14;
const MAX_OPEN_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inspector {
    Review,
    Git,
    Search,
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CenterPane {
    Document,
    Terminal,
}

/// What the Review inspector shows for the active session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewTarget {
    /// Everything since the session started.
    All,
    Turn(usize),
}

pub struct FileTab {
    pub path: PathBuf,
    pub content: Content,
    pub dirty: bool,
    pub mtime: Option<SystemTime>,
    pub syntax: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffSource {
    /// `staged`: HEAD ⇄ index, otherwise index ⇄ working tree.
    Index { staged: bool },
    /// Between two snapshot trees (a turn or a whole session).
    Trees { from: String, to: String, label: String },
}

pub struct DiffTab {
    /// Repo-relative path.
    pub rel: String,
    pub source: DiffSource,
    pub full: bool,
    pub diff: Option<FileDiff>,
    /// Line clicked in the diff (new-side number preferred), for comments.
    pub anchor: Option<usize>,
}

pub enum Tab {
    File(FileTab),
    Diff(DiffTab),
}

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub replace: String,
    pub opts: Options,
    pub generation: u64,
    pub running: bool,
    pub results: Vec<FileMatches>,
    pub total: usize,
    pub error: Option<String>,
}

pub struct App {
    pub root: PathBuf,
    pub center: pane_grid::State<CenterPane>,
    pub doc_ratio: f32,
    pub show_inspector: bool,
    pub focus_mode: bool,
    pub inspector: Inspector,
    pub window_focused: bool,

    // sessions
    pub sessions: Vec<Session>,
    pub active: usize,
    pub next_id: u64,
    pub presets: Vec<(&'static str, &'static str)>,
    pub show_presets: bool,
    pub review: ReviewTarget,
    pub session_files: Option<Vec<Change>>,
    pub confirm_revert: Option<(u64, usize)>,
    pub comment: String,

    // documents
    pub tabs: Vec<Tab>,
    pub active_tab: Option<usize>,
    pub pending_close: Option<usize>,

    // inspectors
    pub explorer: Explorer,
    pub git: Snapshot,
    pub repo_prefix: String,
    pub refreshing: bool,
    pub git_busy: bool,
    pub commit_msg: String,
    pub search: SearchState,

    pub status: Option<(String, bool)>,
    status_at: Instant,
    /// Session id whose close is awaiting a second click.
    pub confirm_close: Option<u64>,

    pub theme: theme::Choice,

    // persistence
    state_dirty: bool,
    discovering: bool,
    last_discovery: Instant,
}

#[derive(Debug, Clone, Copy)]
pub enum SnapshotFor {
    Base,
    Before(usize),
    After(usize),
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Shortcut: show this inspector, or hide it if it is already shown.
    Inspector(Inspector),
    /// Tab click: just switch.
    InspectorTab(Inspector),
    ToggleInspector,
    CycleTheme,
    FocusMode,
    CenterResized(pane_grid::ResizeEvent),
    Key(keyboard::Event, bool),
    WindowFocus(bool),
    Tick,
    StatusTick,

    // sessions
    TogglePresets,
    NewSession(Option<&'static str>),
    SelectSession(usize),
    CloseSession(usize),
    Term(iced_term::Event),
    Snapshot(u64, Vec<SnapshotFor>, Result<String, String>),
    TurnFiles(u64, usize, Vec<Change>),
    Review(ReviewTarget),
    SessionFiles(u64, Vec<Change>),
    OpenTurnDiff(String),
    /// Previous/next file of the review list.
    ReviewStep(isize),
    RevertReview,
    Comment(String),
    SendComment,
    SendToAgent(String),
    /// `(session id, agent conversation id, its folder)` found on disk.
    Discovered(Vec<(u64, String, PathBuf)>),

    // documents
    OpenFile(PathBuf, Option<usize>),
    SelectTab(usize),
    CloseTab(usize),
    Edit(text_editor::Action),
    Save,
    DiffLoaded(String, DiffSource, bool, FileDiff),
    DiffFull(bool),
    DiffAnchor(usize),

    // git
    ExplorerClick(PathBuf, bool),
    GitLoaded(Result<Snapshot, String>),
    OpenDiff(String, bool),
    Stage(Vec<String>),
    Unstage(Vec<String>),
    CommitInput(String),
    Commit,
    Push,
    Pull,
    GitDone(Result<String, String>),

    // search
    SearchQuery(String),
    SearchReplace(String),
    SearchToggle(u8),
    SearchRun,
    SearchDone(u64, Result<(Vec<FileMatches>, usize), String>),
    ReplaceAll,
    ReplaceDone(Result<usize, String>),

}

pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("background task panicked")
}

/// The `Instant` corresponding to wall-clock time `t` (clamped to now).
fn instant_at(t: SystemTime) -> Instant {
    let ago = SystemTime::now().duration_since(t).unwrap_or_default();
    Instant::now().checked_sub(ago).unwrap_or_else(Instant::now)
}

pub fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_OPEN_BYTES {
        return Err("file too large".into());
    }
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return Err("binary file".into());
    }
    String::from_utf8(bytes).map_err(|_| "not valid UTF-8".into())
}

fn load_diff(root: &Path, rel: &str, source: &DiffSource, full: bool) -> FileDiff {
    let (old, new) = match source {
        DiffSource::Index { staged: true } => (git::show(root, "HEAD", rel), git::show(root, "", rel)),
        DiffSource::Index { staged: false } => {
            let disk = std::fs::read(root.join(rel))
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            (git::show(root, "", rel), disk)
        }
        DiffSource::Trees { from, to, .. } => (git::show(root, from, rel), git::show(root, to, rel)),
    };
    diff::compute(&old, &new, if full { None } else { Some(3) })
}

fn center_layout(with_document: bool, ratio: f32) -> pane_grid::State<CenterPane> {
    if with_document {
        pane_grid::State::with_configuration(Configuration::Split {
            axis: Axis::Horizontal,
            ratio,
            a: Box::new(Configuration::Pane(CenterPane::Document)),
            b: Box::new(Configuration::Pane(CenterPane::Terminal)),
        })
    } else {
        pane_grid::State::with_configuration(Configuration::Pane(CenterPane::Terminal))
    }
}

fn notify(summary: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["--app-name=mira", summary, body])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

impl App {
    pub fn new(root: PathBuf, fresh: bool) -> (Self, Task<Msg>) {
        let repo_prefix = git::repo_root(&root)
            .and_then(|r| root.strip_prefix(&r).ok().map(|p| p.to_string_lossy().into_owned()))
            .map(|p| if p.is_empty() { p } else { format!("{p}/") })
            .unwrap_or_default();

        let theme_choice =
            persist::config_get("theme").and_then(|v| theme::Choice::parse(&v)).unwrap_or(theme::Choice::Auto);
        theme::set_current(theme_choice.resolve());
        let mut app = Self {
            theme: theme_choice,
            explorer: Explorer::new(root.clone()),
            root,
            center: center_layout(false, 0.55),
            doc_ratio: 0.55,
            show_inspector: true,
            focus_mode: false,
            inspector: Inspector::Review,
            window_focused: true,
            sessions: Vec::new(),
            active: 0,
            next_id: 0,
            presets: session::available_presets(),
            show_presets: false,
            review: ReviewTarget::All,
            session_files: None,
            confirm_revert: None,
            comment: String::new(),
            tabs: Vec::new(),
            active_tab: None,
            pending_close: None,
            git: Snapshot::default(),
            repo_prefix,
            refreshing: false,
            git_busy: false,
            commit_msg: String::new(),
            search: SearchState::default(),
            status: None,
            status_at: Instant::now(),
            confirm_close: None,
            state_dirty: false,
            discovering: false,
            last_discovery: Instant::now(),
        };
        let saved = if fresh { None } else { persist::load(&app.root).filter(|s| !s.sessions.is_empty()) };
        let shell = match saved {
            Some(state) => app.restore(state),
            None => app.new_session(None, Resume::New, None),
        };
        let refresh = app.refresh_git();
        (app, Task::batch([shell, refresh]))
    }

    pub fn title(&self) -> String {
        format!("{} — mira", file_name(&self.root))
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        let mut subs: Vec<Subscription<Msg>> =
            self.sessions.iter().map(|s| s.term.subscription().map(Msg::Term)).collect();
        subs.push(iced::time::every(Duration::from_secs(2)).map(|_| Msg::Tick));
        subs.push(iced::time::every(Duration::from_millis(700)).map(|_| Msg::StatusTick));
        subs.push(iced::event::listen_with(|event, status, _| match event {
            iced::Event::Keyboard(k @ keyboard::Event::KeyPressed { .. }) => {
                Some(Msg::Key(k, status == iced::event::Status::Captured))
            }
            iced::Event::Window(iced::window::Event::Focused) => Some(Msg::WindowFocus(true)),
            iced::Event::Window(iced::window::Event::Unfocused) => Some(Msg::WindowFocus(false)),
            _ => None,
        }));
        Subscription::batch(subs)
    }

    pub fn repo_root(&self) -> PathBuf {
        self.git.root.clone().unwrap_or_else(|| self.root.clone())
    }

    pub fn has_repo(&self) -> bool {
        self.git.root.is_some() || git::repo_root(&self.root).is_some()
    }

    fn set_status(&mut self, msg: impl Into<String>, error: bool) {
        self.status = Some((msg.into(), error));
        self.status_at = Instant::now();
    }

    pub fn session(&self) -> Option<&Session> {
        self.sessions.get(self.active)
    }

    fn session_index(&self, id: u64) -> Option<usize> {
        self.sessions.iter().position(|s| s.id == id)
    }

    // ---------------------------------------------------------------- sessions

    fn new_session(&mut self, agent: Option<&'static str>, resume: Resume, cwd: Option<PathBuf>) -> Task<Msg> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("TERM_PROGRAM".to_string(), "mira".to_string());
        let settings = iced_term::settings::Settings {
            backend: iced_term::settings::BackendSettings {
                program: shell,
                args: vec![],
                env,
                working_directory: Some(self.root.clone()),
            },
            theme: iced_term::settings::ThemeSettings::new(Box::new(theme::terminal_palette())),
            font: iced_term::settings::FontSettings {
                size: CODE_SIZE as f32,
                font_type: theme::mono_font(),
                ..Default::default()
            },
        };
        let mut term = match iced_term::Terminal::new(self.next_id, settings) {
            Ok(t) => t,
            Err(e) => {
                self.set_status(format!("terminal: {e}"), true);
                return Task::none();
            }
        };

        // App shortcuts must not leak into the shell.
        let swallow = |c: &str, m: keyboard::Modifiers| {
            (
                iced_term::bindings::Binding {
                    target: iced_term::bindings::InputKind::Char(c.to_string()),
                    modifiers: m,
                    terminal_mode_include: iced_term::TermMode::empty(),
                    terminal_mode_exclude: iced_term::TermMode::empty(),
                },
                iced_term::bindings::BindingAction::Esc(String::new()),
            )
        };
        let cs = keyboard::Modifiers::CTRL | keyboard::Modifiers::SHIFT;
        let mut bindings: Vec<_> =
            // Not "v"/"c": Ctrl+Shift+V/C are the terminal's paste/copy.
            ["r", "g", "f", "e", "m", "n", "b", "j", "k", "t"].iter().map(|c| swallow(c, cs)).collect();
        bindings.push(swallow("`", keyboard::Modifiers::CTRL));
        term.handle(iced_term::Command::AddBindings(bindings));

        // Launch the agent as if typed, so the shell stays when it exits.
        let preset = agent.and_then(|a| self.presets.iter().find(|(n, _)| *n == a));
        let (name, agent_name) = match preset.and_then(|(n, _)| Some((*n, session::launch_command(n, &resume)?))) {
            Some((name, cmd)) => {
                let cmd = match cwd.as_deref().filter(|d| *d != self.root && d.is_dir()) {
                    Some(d) => format!("cd {} && {cmd}", session::shell_quote(&d.to_string_lossy())),
                    None => cmd,
                };
                term.handle(iced_term::Command::ProxyToBackend(BackendCommand::Write(format!("{cmd}\r").into_bytes())));
                (name.to_string(), Some(name))
            }
            None => ("shell".to_string(), None),
        };

        let id = self.next_id;
        self.next_id += 1;
        let focus = iced_term::TerminalView::focus(term.widget_id().clone());
        let mut s = Session::new(id, name, agent_name, term);
        if agent_name.is_some() {
            s.status = Status::Working;
        }
        if let Resume::Id(rid) = resume {
            s.resume_id = Some(rid);
        }
        s.cwd = cwd;
        self.state_dirty = true;
        self.sessions.push(s);
        self.active = self.sessions.len() - 1;
        self.review = ReviewTarget::All;
        self.session_files = None;
        self.show_presets = false;
        Task::batch([focus, self.snapshot(id, vec![SnapshotFor::Base])])
    }

    /// Reopen the sessions saved for this folder, resuming each agent.
    fn restore(&mut self, state: persist::State) -> Task<Msg> {
        let mut tasks = Vec::new();
        let root = self.repo_root();
        let n = state.sessions.len();
        for saved in state.sessions {
            let agent = saved.agent.as_deref().and_then(|a| self.presets.iter().find(|(n, _)| *n == a)).map(|(n, _)| *n);
            let resume = match &saved.resume_id {
                Some(id) if session::valid_id(id) => Resume::Id(id.clone()),
                _ => Resume::Last,
            };
            let before = self.sessions.len();
            tasks.push(self.new_session(agent, resume, saved.cwd.clone()));
            if self.sessions.len() == before {
                continue;
            }
            let s = self.sessions.last_mut().expect("just pushed");
            if agent.is_some() || saved.agent.is_none() {
                s.name = saved.name;
            }
            s.base = saved.base;
            s.latest = saved.latest;
            s.started_at = persist::from_unix(saved.started_at);
            s.turns = saved
                .turns
                .into_iter()
                .map(|t| Turn {
                    label: t.label,
                    before: t.before,
                    after: t.after,
                    files: None,
                    started: instant_at(persist::from_unix(t.started_at)),
                    snapshotting: false,
                })
                .collect();
            let id = s.id;
            for (i, t) in s.turns.iter().enumerate() {
                if let (Some(b), Some(a)) = (t.before.clone(), t.after.clone()) {
                    let root = root.clone();
                    tasks.push(Task::perform(
                        blocking(move || git::diff_trees(&root, &b, &a).unwrap_or_default()),
                        move |files| Msg::TurnFiles(id, i, files),
                    ));
                }
            }
        }
        if !self.sessions.is_empty() {
            self.active = state.active.min(self.sessions.len() - 1);
            tasks.push(self.focus_terminal());
            tasks.push(self.load_session_files());
            self.set_status(format!("Restored {n} session(s) — start fresh with `mira --fresh`"), false);
        } else {
            tasks.push(self.new_session(None, Resume::New, None));
        }
        Task::batch(tasks)
    }

    fn save_state(&mut self) {
        if !self.state_dirty {
            return;
        }
        self.state_dirty = false;
        let state = persist::State {
            root: self.root.clone(),
            active: self.active,
            sessions: self
                .sessions
                .iter()
                .filter(|s| s.status != Status::Exited)
                .map(|s| persist::SavedSession {
                    name: s.name.clone(),
                    agent: s.agent.map(str::to_string),
                    resume_id: s.resume_id.clone(),
                    cwd: s.cwd.clone(),
                    base: s.base.clone(),
                    latest: s.latest.clone(),
                    started_at: persist::unix(s.started_at),
                    turns: s
                        .turns
                        .iter()
                        .map(|t| persist::SavedTurn {
                            label: t.label.clone(),
                            before: t.before.clone(),
                            after: t.after.clone(),
                            started_at: persist::unix(SystemTime::now() - t.started.elapsed()),
                        })
                        .collect(),
                })
                .collect(),
        };
        if let Err(e) = persist::save(&state) {
            self.set_status(format!("Could not save sessions: {e}"), true);
        }
    }

    /// Look up the conversation ids of agents that do not have one yet.
    fn discover_conversations(&mut self) -> Task<Msg> {
        if self.discovering || self.last_discovery.elapsed() < Duration::from_secs(6) {
            return Task::none();
        }
        let wanted: Vec<(u64, &'static str, u64)> = self
            .sessions
            .iter()
            .filter(|s| s.resume_id.is_none() && s.status != Status::Exited)
            .filter_map(|s| Some((s.id, s.agent.filter(|a| persist::discoverable(a))?, persist::unix(s.started_at))))
            .collect();
        if wanted.is_empty() {
            return Task::none();
        }
        self.discovering = true;
        self.last_discovery = Instant::now();
        let mut claimed: Vec<String> = self.sessions.iter().filter_map(|s| s.resume_id.clone()).collect();
        let root = self.root.clone();
        Task::perform(
            blocking(move || {
                let mut found = Vec::new();
                for (id, agent, since) in wanted {
                    if let Some((conv, cwd)) = persist::find_conversation(agent, &root, since, &claimed) {
                        claimed.push(conv.clone());
                        found.push((id, conv, cwd));
                    }
                }
                found
            }),
            Msg::Discovered,
        )
    }

    /// Activate the palette of the current choice; recolor terminals if it changed.
    fn apply_theme(&mut self) {
        if theme::set_current(self.theme.resolve()) {
            for s in &mut self.sessions {
                s.term.handle(iced_term::Command::ChangeTheme(Box::new(theme::terminal_palette())));
            }
        }
    }

    fn cycle_session(&mut self, step: isize) -> Task<Msg> {
        let n = self.sessions.len() as isize;
        if n < 2 {
            return Task::none();
        }
        let next = (self.active as isize + step).rem_euclid(n) as usize;
        self.update(Msg::SelectSession(next))
    }

    fn snapshot(&self, id: u64, purposes: Vec<SnapshotFor>) -> Task<Msg> {
        if !self.has_repo() {
            return Task::none();
        }
        let root = self.repo_root();
        Task::perform(blocking(move || git::snapshot_tree(&root)), move |r| {
            Msg::Snapshot(id, purposes.clone(), r)
        })
    }

    fn focus_terminal(&self) -> Task<Msg> {
        self.session()
            .map(|s| iced_term::TerminalView::focus(s.term.widget_id().clone()))
            .unwrap_or_else(Task::none)
    }

    fn write_to_session(&mut self, text: &str) -> Task<Msg> {
        let Some(s) = self.sessions.get_mut(self.active) else {
            self.set_status("No active session", true);
            return Task::none();
        };
        s.term.handle(iced_term::Command::ProxyToBackend(BackendCommand::Write(text.as_bytes().to_vec())));
        // Count it as typed so the next Enter opens a turn labelled with it.
        let _ = s.track_input(text.as_bytes());
        let name = s.name.clone();
        self.set_status(format!("Sent to {name} — review and press Enter"), false);
        self.focus_terminal()
    }

    /// The user submitted a line in session `idx`.
    fn on_prompt(&mut self, idx: usize, line: String) -> Task<Msg> {
        let id = self.sessions[idx].id;
        self.state_dirty = true;
        let s = &mut self.sessions[idx];
        s.status = Status::Working;
        s.last_output = Instant::now();

        // An agent started by hand in a plain shell: adopt it, so it gets
        // status, notifications and is resumed next time.
        if s.agent.is_none()
            && let Some((name, rid)) = session::detect_launch(&line)
        {
            s.agent = Some(name);
            s.name = name.to_string();
            s.started_at = SystemTime::now();
            s.resume_id = rid;
            return Task::none();
        }

        // Short replies ("y", "1") continue the previous turn (approvals, menus).
        if line.chars().count() <= 2
            && let Some(last) = s.turns.last_mut()
                && last.started.elapsed() < Duration::from_secs(1800) && last.before.is_some() {
                    last.after = None;
                    last.files = None;
                    last.snapshotting = false;
                    return Task::none();
                }

        let new_idx = s.turns.len();
        let mut purposes = vec![SnapshotFor::Before(new_idx)];
        if let Some(prev) = s.running_turn() {
            s.turns[prev].snapshotting = true;
            purposes.push(SnapshotFor::After(prev));
        }
        s.turns.push(Turn {
            label: line,
            before: None,
            after: None,
            files: None,
            started: Instant::now(),
            snapshotting: false,
        });
        self.snapshot(id, purposes)
    }

    fn on_term_event(&mut self, id: u64, cmd: &BackendCommand) -> Task<Msg> {
        let Some(idx) = self.session_index(id) else { return Task::none() };
        match cmd {
            BackendCommand::Write(bytes) => {
                if let Some(line) = self.sessions[idx].track_input(bytes) {
                    return self.on_prompt(idx, line);
                }
            }
            BackendCommand::ProcessAlacrittyEvent(AlacrittyEvent::Wakeup) => {
                let s = &mut self.sessions[idx];
                s.last_output = Instant::now();
                // Echo of the user's own typing is not agent activity.
                if s.last_input.elapsed() > Duration::from_millis(400) && s.status != Status::Exited {
                    s.status = Status::Working;
                }
            }
            BackendCommand::ProcessAlacrittyEvent(AlacrittyEvent::Bell) => {
                let s = &mut self.sessions[idx];
                if s.agent.is_some() && (idx != self.active || !self.window_focused) && !s.attention {
                    s.attention = true;
                    notify(&format!("{} needs you", s.name), "The agent rang the bell");
                }
            }
            _ => {}
        }
        Task::none()
    }

    fn status_tick(&mut self) -> Task<Msg> {
        let mut tasks = Vec::new();
        let focused = self.window_focused;
        for i in 0..self.sessions.len() {
            let is_active = i == self.active;
            let s = &mut self.sessions[i];
            if s.status != Status::Working || s.last_output.elapsed() < session::QUIET_AFTER {
                continue;
            }
            let id = s.id;
            let running = s.running_turn();
            // Plain shells just finish commands; only agents "wait for you".
            s.status = if s.agent.is_some() { Status::Waiting } else { Status::Idle };
            if s.status == Status::Waiting && (!focused || !is_active) && !s.attention {
                s.attention = true;
                notify(&format!("{} is waiting", s.name), "The agent went quiet — review its changes");
            }
            if let Some(t) = running
                && s.turns[t].before.is_some() && !s.turns[t].snapshotting {
                    s.turns[t].snapshotting = true;
                    tasks.push((id, t));
                }
        }
        Task::batch(tasks.into_iter().map(|(id, t)| self.snapshot(id, vec![SnapshotFor::After(t)])))
    }

    fn on_snapshot(&mut self, id: u64, purposes: Vec<SnapshotFor>, tree: String) -> Task<Msg> {
        let Some(idx) = self.session_index(id) else { return Task::none() };
        let root = self.repo_root();
        self.state_dirty = true;
        let s = &mut self.sessions[idx];
        s.latest = Some(tree.clone());
        let mut tasks = Vec::new();
        for p in purposes {
            let t = match p {
                SnapshotFor::Base => {
                    s.base.get_or_insert(tree.clone());
                    continue;
                }
                SnapshotFor::Before(t) => {
                    if let Some(turn) = s.turns.get_mut(t) {
                        turn.before = Some(tree.clone());
                    }
                    t
                }
                SnapshotFor::After(t) => {
                    if let Some(turn) = s.turns.get_mut(t) {
                        turn.snapshotting = false;
                        // A newer prompt may have reopened this turn meanwhile.
                        if turn.before.is_some() {
                            turn.after = Some(tree.clone());
                        }
                    }
                    t
                }
            };
            if let Some(Turn { before: Some(b), after: Some(a), .. }) = s.turns.get(t) {
                let (root, b, a) = (root.clone(), b.clone(), a.clone());
                tasks.push(Task::perform(
                    blocking(move || git::diff_trees(&root, &b, &a).unwrap_or_default()),
                    move |files| Msg::TurnFiles(id, t, files),
                ));
            }
        }
        if idx == self.active && self.review == ReviewTarget::All {
            tasks.push(self.load_session_files());
        }
        Task::batch(tasks)
    }

    fn load_session_files(&self) -> Task<Msg> {
        let Some(s) = self.session() else { return Task::none() };
        let (Some(base), Some(latest)) = (s.base.clone(), s.latest.clone()) else {
            return Task::none();
        };
        let (root, id) = (self.repo_root(), s.id);
        Task::perform(
            blocking(move || git::diff_trees(&root, &base, &latest).unwrap_or_default()),
            move |files| Msg::SessionFiles(id, files),
        )
    }

    /// Trees `(from, to, label)` of the current review target.
    pub fn review_trees(&self) -> Option<(String, String, String)> {
        let s = self.session()?;
        match self.review {
            ReviewTarget::All => Some((s.base.clone()?, s.latest.clone()?, "Session".into())),
            ReviewTarget::Turn(t) => {
                let turn = s.turns.get(t)?;
                Some((turn.before.clone()?, turn.after.clone()?, format!("Turn {}", t + 1)))
            }
        }
    }

    pub fn review_files(&self) -> Option<&Vec<Change>> {
        match self.review {
            ReviewTarget::All => self.session_files.as_ref(),
            ReviewTarget::Turn(t) => self.session()?.turns.get(t)?.files.as_ref(),
        }
    }

    /// `(index, total)` of `rel` in the current review list.
    pub fn review_position(&self, rel: &str) -> Option<(usize, usize)> {
        let files = self.review_files()?;
        files.iter().position(|c| c.path == rel).map(|i| (i, files.len()))
    }

    // --------------------------------------------------------------- documents

    fn sync_center(&mut self) {
        let want_doc = !self.tabs.is_empty() && !self.focus_mode;
        let has_doc = self.center.iter().any(|(_, k)| *k == CenterPane::Document);
        if want_doc != has_doc {
            self.center = center_layout(want_doc, self.doc_ratio);
        }
    }

    fn open_tab(&mut self, tab: Tab) -> usize {
        self.tabs.push(tab);
        self.tabs.len() - 1
    }

    fn open_file(&mut self, path: PathBuf, line: Option<usize>) {
        let idx = match self.tabs.iter().position(|t| matches!(t, Tab::File(f) if f.path == path)) {
            Some(i) => i,
            None => match read_text(&path) {
                Ok(text) => {
                    let name = file_name(&path);
                    let syntax = if name == ".env" || name.starts_with(".env.") || name.ends_with(".env") {
                        // KEY=value with # comments reads well as shell.
                        "sh".to_string()
                    } else {
                        path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or(name)
                    };
                    self.open_tab(Tab::File(FileTab {
                        mtime: mtime(&path),
                        content: Content::with_text(&text),
                        path,
                        dirty: false,
                        syntax,
                    }))
                }
                Err(e) => {
                    self.set_status(format!("{}: {e}", file_name(&path)), true);
                    return;
                }
            },
        };
        if let (Some(line), Tab::File(t)) = (line, &mut self.tabs[idx]) {
            let line = line.min(t.content.line_count().saturating_sub(1));
            t.content.move_to(text_editor::Cursor {
                position: text_editor::Position { line, column: 0 },
                selection: None,
            });
        }
        self.active_tab = Some(idx);
        self.focus_mode = false;
        self.sync_center();
    }

    fn open_diff(&mut self, rel: String, source: DiffSource) -> Task<Msg> {
        let idx = self
            .tabs
            .iter()
            .position(|t| matches!(t, Tab::Diff(d) if d.rel == rel && d.source == source));
        let tab = |rel, source| Tab::Diff(DiffTab { rel, source, full: false, diff: None, anchor: None });
        let idx = match idx {
            Some(i) => i,
            // Review diffs share one preview tab instead of piling up.
            None if matches!(source, DiffSource::Trees { .. }) => {
                match self.tabs.iter().position(|t| matches!(t, Tab::Diff(d) if matches!(d.source, DiffSource::Trees { .. }))) {
                    Some(i) => {
                        self.tabs[i] = tab(rel, source);
                        i
                    }
                    None => self.open_tab(tab(rel, source)),
                }
            }
            None => self.open_tab(tab(rel, source)),
        };
        self.active_tab = Some(idx);
        self.focus_mode = false;
        self.sync_center();
        self.reload_active_diff()
    }

    fn reload_active_diff(&self) -> Task<Msg> {
        let Some(Tab::Diff(d)) = self.active_tab.and_then(|i| self.tabs.get(i)) else {
            return Task::none();
        };
        let (root, rel, source, full) = (self.repo_root(), d.rel.clone(), d.source.clone(), d.full);
        Task::perform(
            blocking(move || {
                let diff = load_diff(&root, &rel, &source, full);
                (rel, source, full, diff)
            }),
            |(rel, source, full, diff)| Msg::DiffLoaded(rel, source, full, diff),
        )
    }

    fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        if matches!(&self.tabs[i], Tab::File(t) if t.dirty) && self.pending_close != Some(i) {
            self.pending_close = Some(i);
            self.set_status("Unsaved changes — close again to discard them (Ctrl+S to save)", true);
            return;
        }
        self.pending_close = None;
        self.tabs.remove(i);
        self.active_tab = match self.active_tab {
            _ if self.tabs.is_empty() => None,
            Some(a) if a > i => Some(a - 1),
            Some(a) if a == i => Some(i.min(self.tabs.len() - 1)),
            other => other,
        };
        self.sync_center();
    }

    fn save(&mut self) {
        let Some(Tab::File(t)) = self.active_tab.and_then(|i| self.tabs.get_mut(i)) else { return };
        match std::fs::write(&t.path, t.content.text()) {
            Ok(()) => {
                t.dirty = false;
                t.mtime = mtime(&t.path);
                let name = file_name(&t.path);
                self.set_status(format!("Saved {name}"), false);
            }
            Err(e) => self.set_status(format!("Save failed: {e}"), true),
        }
    }

    fn reload_changed_files(&mut self) {
        for tab in &mut self.tabs {
            let Tab::File(t) = tab else { continue };
            let now = mtime(&t.path);
            if t.dirty || now == t.mtime {
                continue;
            }
            t.mtime = now;
            if let Ok(text) = read_text(&t.path) {
                let pos = t.content.cursor().position;
                t.content = Content::with_text(&text);
                let line = pos.line.min(t.content.line_count().saturating_sub(1));
                t.content.move_to(text_editor::Cursor {
                    position: text_editor::Position { line, column: 0 },
                    selection: None,
                });
            }
        }
    }

    // ---------------------------------------------------------- git / search

    fn refresh_git(&mut self) -> Task<Msg> {
        if self.refreshing {
            return Task::none();
        }
        self.refreshing = true;
        let root = self.root.clone();
        Task::perform(blocking(move || git::snapshot(&root)), Msg::GitLoaded)
    }

    fn git_op(&mut self, op: impl FnOnce(&Path) -> Result<String, String> + Send + 'static) -> Task<Msg> {
        if self.git_busy {
            return Task::none();
        }
        self.git_busy = true;
        let root = self.repo_root();
        Task::perform(blocking(move || op(&root)), Msg::GitDone)
    }

    fn run_search(&mut self) -> Task<Msg> {
        self.search.generation += 1;
        self.search.error = None;
        if self.search.query.is_empty() {
            self.search.results.clear();
            self.search.total = 0;
            self.search.running = false;
            return Task::none();
        }
        self.search.running = true;
        let (generation_id, root, query, opts) =
            (self.search.generation, self.root.clone(), self.search.query.clone(), self.search.opts.clone());
        Task::perform(blocking(move || search::search(&root, &query, &opts)), move |r| {
            Msg::SearchDone(generation_id, r)
        })
    }

    // ------------------------------------------------------------------ update

    pub fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Inspector(i) => {
                if self.inspector == i && self.show_inspector {
                    self.show_inspector = false;
                } else {
                    self.inspector = i;
                    self.show_inspector = true;
                    self.focus_mode = false;
                    self.sync_center();
                }
            }
            Msg::FocusMode => {
                self.focus_mode = !self.focus_mode;
                self.sync_center();
                return self.focus_terminal();
            }
            Msg::CenterResized(e) => {
                self.center.resize(e.split, e.ratio);
                self.doc_ratio = e.ratio;
            }
            Msg::Key(event, captured) => return self.on_key(event, captured),
            Msg::WindowFocus(f) => {
                self.window_focused = f;
                if f && let Some(s) = self.sessions.get_mut(self.active) {
                    s.attention = false;
                }
            }
            Msg::Tick => {
                if self.theme == theme::Choice::Auto {
                    self.apply_theme();
                }
                self.explorer.refresh();
                self.reload_changed_files();
                let tasks = vec![self.refresh_git(), self.reload_active_diff(), self.discover_conversations()];
                return Task::batch(tasks);
            }
            Msg::StatusTick => {
                let ttl = if self.status.as_ref().is_some_and(|(_, e)| *e) { 12 } else { 6 };
                if self.status.is_some() && self.status_at.elapsed() > Duration::from_secs(ttl) {
                    self.status = None;
                    self.confirm_revert = None;
                    self.confirm_close = None;
                }
                self.save_state();
                return self.status_tick();
            }
            Msg::CycleTheme => {
                self.theme = self.theme.next();
                self.apply_theme();
                let name = theme::p().name;
                let label = match self.theme {
                    theme::Choice::Auto => format!("Theme: auto (following HyDE · {name})"),
                    theme::Choice::Fixed(_) => format!("Theme: {name}"),
                };
                self.set_status(label, false);
                if let Err(e) = persist::config_set("theme", &self.theme.label()) {
                    self.set_status(format!("Could not save theme: {e}"), true);
                }
            }
            Msg::ToggleInspector => {
                self.show_inspector = !self.show_inspector;
                self.focus_mode = false;
                self.sync_center();
            }
            Msg::InspectorTab(i) => {
                self.inspector = i;
            }

            Msg::TogglePresets => self.show_presets = !self.show_presets,
            Msg::NewSession(agent) => return self.new_session(agent, Resume::New, None),
            Msg::SelectSession(i) => {
                if i < self.sessions.len() {
                    self.active = i;
                    self.state_dirty = true;
                    self.sessions[i].attention = false;
                    self.review = ReviewTarget::All;
                    self.session_files = None;
                    return Task::batch([self.focus_terminal(), self.load_session_files()]);
                }
            }
            Msg::CloseSession(i) => {
                if let Some(s) = self.sessions.get(i)
                    && s.agent.is_some()
                    && s.status != Status::Exited
                    && self.confirm_close != Some(s.id)
                {
                    self.confirm_close = Some(s.id);
                    let name = s.name.clone();
                    self.set_status(format!("{name} is still running — click × again to kill it"), true);
                    return Task::none();
                }
                self.confirm_close = None;
                if i < self.sessions.len() {
                    self.sessions.remove(i);
                    self.state_dirty = true;
                    self.active = self.active.min(self.sessions.len().saturating_sub(1));
                    self.session_files = None;
                    return self.load_session_files();
                }
            }
            Msg::Term(iced_term::Event::BackendCall(id, cmd)) => {
                let task = self.on_term_event(id, &cmd);
                if let Some(i) = self.session_index(id) {
                    let action = self.sessions[i].term.handle(iced_term::Command::ProxyToBackend(cmd));
                    if action == iced_term::actions::Action::Shutdown {
                        self.sessions[i].status = Status::Exited;
                        return self.update(Msg::CloseSession(i));
                    }
                }
                return task;
            }
            Msg::Snapshot(id, purposes, result) => match result {
                Ok(tree) => return self.on_snapshot(id, purposes, tree),
                Err(e) => self.set_status(format!("snapshot: {e}"), true),
            },
            Msg::TurnFiles(id, t, files) => {
                if let Some(turn) = self.session_index(id).and_then(|i| self.sessions[i].turns.get_mut(t)) {
                    turn.files = Some(files);
                }
            }
            Msg::Review(target) => {
                self.review = target;
                self.confirm_revert = None;
                self.inspector = Inspector::Review;
                self.show_inspector = true;
                if target == ReviewTarget::All {
                    return self.load_session_files();
                }
                // Jump straight to the first changed file of the turn.
                if let Some(first) = self.review_files().and_then(|f| f.first()).map(|c| c.path.clone()) {
                    return self.update(Msg::OpenTurnDiff(first));
                }
            }
            Msg::SessionFiles(id, files) => {
                if self.session().is_some_and(|s| s.id == id) {
                    self.session_files = Some(files);
                }
            }
            Msg::OpenTurnDiff(rel) => {
                if let Some((from, to, label)) = self.review_trees() {
                    return self.open_diff(rel, DiffSource::Trees { from, to, label });
                }
            }
            Msg::ReviewStep(step) => {
                let Some(Tab::Diff(d)) = self.active_tab.and_then(|i| self.tabs.get(i)) else {
                    return Task::none();
                };
                if let Some((i, n)) = self.review_position(&d.rel) {
                    let next = (i as isize + step).rem_euclid(n as isize) as usize;
                    if let Some(c) = self.review_files().and_then(|f| f.get(next)) {
                        return self.update(Msg::OpenTurnDiff(c.path.clone()));
                    }
                }
            }
            Msg::RevertReview => {
                let Some(s) = self.session() else { return Task::none() };
                let key = (s.id, match self.review { ReviewTarget::All => usize::MAX, ReviewTarget::Turn(t) => t });
                if self.confirm_revert != Some(key) {
                    self.confirm_revert = Some(key);
                    self.set_status("Click Revert again to restore these files (later edits to them are lost)", true);
                    return Task::none();
                }
                self.confirm_revert = None;
                let (Some((from, _, _)), Some(files)) = (self.review_trees(), self.review_files().cloned()) else {
                    return Task::none();
                };
                let paths: Vec<String> = files.into_iter().map(|c| c.path).collect();
                return self.git_op(move |r| git::restore_from_tree(r, &from, &paths));
            }
            Msg::Comment(c) => self.comment = c,
            Msg::SendComment => {
                let comment = self.comment.trim().to_string();
                let Some(Tab::Diff(d)) = self.active_tab.and_then(|i| self.tabs.get(i)) else {
                    return Task::none();
                };
                if comment.is_empty() {
                    return Task::none();
                }
                let place = match d.anchor {
                    Some(line) => format!("{}:{line}", d.rel),
                    None => d.rel.clone(),
                };
                self.comment.clear();
                return self.write_to_session(&format!("En {place} — {comment}"));
            }
            Msg::SendToAgent(text) => return self.write_to_session(&text),
            Msg::Discovered(found) => {
                self.discovering = false;
                for (id, conv, cwd) in found {
                    if let Some(i) = self.session_index(id) {
                        self.sessions[i].resume_id = Some(conv);
                        self.sessions[i].cwd = Some(cwd);
                        self.state_dirty = true;
                    }
                }
            }

            Msg::OpenFile(path, line) => self.open_file(path, line),
            Msg::SelectTab(i) => {
                if i < self.tabs.len() {
                    self.active_tab = Some(i);
                    return self.reload_active_diff();
                }
            }
            Msg::CloseTab(i) => self.close_tab(i),
            Msg::Edit(action) => {
                if let Some(Tab::File(t)) = self.active_tab.and_then(|i| self.tabs.get_mut(i)) {
                    if action.is_edit() {
                        t.dirty = true;
                    }
                    t.content.perform(action);
                }
            }
            Msg::Save => self.save(),
            Msg::DiffLoaded(rel, source, full, diff) => {
                for tab in &mut self.tabs {
                    if let Tab::Diff(d) = tab
                        && d.rel == rel && d.source == source && d.full == full {
                            d.diff = Some(diff.clone());
                        }
                }
            }
            Msg::DiffFull(full) => {
                if let Some(Tab::Diff(d)) = self.active_tab.and_then(|i| self.tabs.get_mut(i)) {
                    d.full = full;
                    return self.reload_active_diff();
                }
            }
            Msg::DiffAnchor(line) => {
                if let Some(Tab::Diff(d)) = self.active_tab.and_then(|i| self.tabs.get_mut(i)) {
                    d.anchor = if d.anchor == Some(line) { None } else { Some(line) };
                }
            }

            Msg::ExplorerClick(path, is_dir) => {
                if is_dir {
                    self.explorer.toggle(&path);
                } else {
                    self.open_file(path, None);
                }
            }
            Msg::GitLoaded(result) => {
                self.refreshing = false;
                match result {
                    Ok(snap) => self.git = snap,
                    Err(e) => self.set_status(e, true),
                }
            }
            Msg::OpenDiff(rel, staged) => return self.open_diff(rel, DiffSource::Index { staged }),
            Msg::Stage(paths) => return self.git_op(move |r| git::stage(r, &paths)),
            Msg::Unstage(paths) => return self.git_op(move |r| git::unstage(r, &paths)),
            Msg::CommitInput(m) => self.commit_msg = m,
            Msg::Commit => {
                let message = self.commit_msg.trim().to_string();
                if message.is_empty() {
                    self.set_status("Write a commit message first", true);
                    return Task::none();
                }
                let stage_all: Vec<String> = if self.git.staged.is_empty() {
                    self.git.unstaged.iter().map(|c| c.path.clone()).collect()
                } else {
                    Vec::new()
                };
                if self.git.staged.is_empty() && stage_all.is_empty() {
                    self.set_status("Nothing to commit", true);
                    return Task::none();
                }
                return self.git_op(move |r| {
                    if !stage_all.is_empty() {
                        git::stage(r, &stage_all)?;
                    }
                    git::commit(r, &message)
                        .map(|out| format!("COMMIT:{}", first_line(&out).unwrap_or("Committed")))
                });
            }
            Msg::Push => {
                self.set_status("Pushing…", false);
                return self.git_op(|r| git::push(r).map(|_| "Pushed".into()));
            }
            Msg::Pull => {
                self.set_status("Pulling…", false);
                return self.git_op(|r| git::pull(r).map(|o| first_line(&o).unwrap_or("Pulled").into()));
            }
            Msg::GitDone(result) => {
                self.git_busy = false;
                match result {
                    Ok(out) => {
                        if let Some(summary) = out.strip_prefix("COMMIT:") {
                            self.commit_msg.clear();
                            self.set_status(summary.to_string(), false);
                        } else if !out.trim().is_empty() {
                            self.set_status(first_line(&out).unwrap_or_default().to_string(), false);
                        }
                    }
                    Err(e) => self.set_status(first_line(&e).unwrap_or(&e).to_string(), true),
                }
                self.refreshing = false;
                self.reload_changed_files();
                return Task::batch([self.refresh_git(), self.reload_active_diff()]);
            }

            Msg::SearchQuery(q) => {
                self.search.query = q;
                return self.run_search();
            }
            Msg::SearchReplace(r) => self.search.replace = r,
            Msg::SearchToggle(which) => {
                let o = &mut self.search.opts;
                match which {
                    0 => o.case_sensitive = !o.case_sensitive,
                    1 => o.whole_word = !o.whole_word,
                    _ => o.regex = !o.regex,
                }
                return self.run_search();
            }
            Msg::SearchRun => return self.run_search(),
            Msg::SearchDone(generation_id, result) => {
                if generation_id == self.search.generation {
                    self.search.running = false;
                    match result {
                        Ok((results, total)) => {
                            self.search.results = results;
                            self.search.total = total;
                        }
                        Err(e) => {
                            self.search.results.clear();
                            self.search.total = 0;
                            self.search.error = Some(e);
                        }
                    }
                }
            }
            Msg::ReplaceAll => {
                if self.search.results.is_empty() {
                    return Task::none();
                }
                let files: Vec<PathBuf> = self.search.results.iter().map(|f| f.path.clone()).collect();
                let (query, replacement, opts) =
                    (self.search.query.clone(), self.search.replace.clone(), self.search.opts.clone());
                return Task::perform(
                    blocking(move || search::replace_all(&files, &query, &replacement, &opts)),
                    Msg::ReplaceDone,
                );
            }
            Msg::ReplaceDone(result) => {
                match result {
                    Ok(n) => self.set_status(format!("Replaced in {n} file(s)"), false),
                    Err(e) => self.set_status(e, true),
                }
                self.reload_changed_files();
                return self.run_search();
            }

        }
        Task::none()
    }

    fn on_key(&mut self, event: keyboard::Event, captured: bool) -> Task<Msg> {
        let keyboard::Event::KeyPressed { key, modifiers, physical_key, .. } = event else {
            return Task::none();
        };
        if !modifiers.command() {
            return Task::none();
        }
        let shift = modifiers.shift();
        let latin = key.to_latin(physical_key).map(|c| c.to_ascii_lowercase());
        let backquote =
            matches!(physical_key, keyboard::key::Physical::Code(keyboard::key::Code::Backquote));
        // While a widget (terminal, editor, input) owns the key, only
        // Ctrl+Shift+… and Ctrl+` act globally.
        if captured && !shift && !backquote {
            return Task::none();
        }
        match (latin, shift) {
            (Some('s'), false) => self.save(),
            (Some('w'), false) => {
                if let Some(i) = self.active_tab {
                    self.close_tab(i);
                }
            }
            (Some('b'), _) => return self.update(Msg::ToggleInspector),
            (Some('r'), true) => return self.update(Msg::Inspector(Inspector::Review)),
            (Some('g'), true) => return self.update(Msg::Inspector(Inspector::Git)),
            (Some('t'), true) => return self.update(Msg::CycleTheme),
            (Some('j'), true) => return self.cycle_session(1),
            (Some('k'), true) => return self.cycle_session(-1),
            (Some('f'), true) => return self.update(Msg::Inspector(Inspector::Search)),
            (Some('e'), true) => return self.update(Msg::Inspector(Inspector::Files)),
            (Some('n'), true) => self.show_presets = !self.show_presets,
            (Some('m'), true) => return self.update(Msg::FocusMode),
            _ if backquote => return self.focus_terminal(),
            _ => {}
        }
        Task::none()
    }
}

pub fn first_line(s: &str) -> Option<&str> {
    s.lines().map(str::trim).find(|l| !l.is_empty())
}
