//! Application state and update logic. Views live in `view.rs`.
//!
//! Model: the center terminal runs herdr, where the user talks to agents.
//! mira polls herdr's socket for the agents working in this folder; every
//! working period is a *run* (`runs.rs`) with before/after snapshots of the
//! working tree, listed in the Review inspector as an inbox.

use crate::diff::{self, FileDiff};
use crate::explorer::Explorer;
use crate::git::{self, Change, Snapshot};
use crate::herdr::{self, Agent, PaneId};
use crate::persist;
use crate::runs::{Effect, RunId, Seen, Tracker};
use crate::search::{self, FileMatches, Options};
use crate::termio;
use crate::theme;

use alacritty_terminal::index::{Column, Line, Point as GridPoint};
use alacritty_terminal::vte::ansi::Rgb;
use iced::keyboard;
use iced::widget::pane_grid::{self, Axis};
use iced::widget::text_editor::{self, Content};
use iced::{Subscription, Task};
use iced_term::{AlacrittyEvent, BackendCommand};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

pub const CODE_SIZE: u32 = 14;

/// Draggable divider between the center and the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    /// Vertical: between the center and the inspector.
    Inspector,
    /// Horizontal: between Changes and History in the Changes panel.
    History,
}

const MIN_HISTORY: f32 = 60.0;
/// Room the Changes list keeps above History.
const MIN_CHANGES_LIST: f32 = 120.0;

const MIN_SIDE: f32 = 220.0;
const MIN_CENTER: f32 = 360.0;
const DIVIDER: f32 = 5.0;
const MAX_OPEN_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inspector {
    /// Git changes, annotated with the agent runs that made them.
    Changes,
    Search,
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CenterPane {
    Document,
    Terminal,
}

/// What runs in the center terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKind {
    Herdr,
    /// herdr is not installed (or `terminal = "shell"` in the config).
    Shell,
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
    /// Between two snapshot trees (an agent run).
    Trees { from: String, to: String, label: String, run: RunId },
}

pub struct DiffTab {
    /// Repo-relative path.
    pub rel: String,
    pub source: DiffSource,
    pub full: bool,
    pub diff: Option<FileDiff>,
    /// Line clicked in the diff (new-side number preferred), for comments.
    pub anchor: Option<usize>,
    /// Working file's mtime when the diff was last computed (index diffs).
    pub loaded_mtime: Option<SystemTime>,
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

    // terminal
    pub term: Option<iced_term::Terminal>,
    pub term_kind: TermKind,
    /// Cell size the terminal measured (for mouse reports).
    pub term_cell: Option<iced::Size>,
    /// Bumped whenever mira focuses the terminal.
    pub focus_epoch: u64,
    next_term_id: u64,
    gate: termio::Gate,
    flush_scheduled: bool,

    // herdr
    herdr: Option<herdr::Client>,
    herdr_session: Option<String>,
    polling: bool,
    workspace: Option<String>,
    ensuring_workspace: bool,
    /// Earliest next `ensure_workspace` attempt after a failure.
    workspace_retry: Instant,
    /// Agents working in this folder, as of the last poll.
    pub agents: Vec<Agent>,
    pub herdr_error: Option<String>,

    // review
    pub runs: Tracker,
    /// Snapshots waiting for the one in flight (git is never run in parallel).
    snapshot_queue: std::collections::VecDeque<(RunId, bool)>,
    snapshot_running: bool,
    /// Set when a snapshot timed out: the tree is too large to track runs.
    snapshots_off: bool,
    pub selected_run: Option<RunId>,
    pub group_by_run: bool,
    pub confirm_revert: Option<RunId>,
    pub comment: String,
    state_dirty: bool,

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
    pub commit_msg: Content,
    /// Cancel flag of the running commit-message generation.
    pub commit_gen: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub search: SearchState,

    pub status: Option<(String, bool)>,
    status_at: Instant,
    pub theme: theme::Choice,

    // layout
    pub inspector_width: f32,
    pub window_width: f32,
    pub window_height: f32,
    /// Height of History in the Changes panel.
    pub history_height: f32,
    /// `(cursor y, height)` when a vertical drag started.
    drag_anchor: Option<(f32, f32)>,
    /// `Msg::Tick` counter, for slower refreshes in the background.
    ticks: u64,
    pub dragging: Option<Divider>,
    pub hovered_divider: Option<Divider>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Shortcut: show this inspector, or hide it if it is already shown.
    Inspector(Inspector),
    /// Tab click: just switch.
    InspectorTab(Inspector),
    ToggleInspector,
    CycleTheme,
    DividerHover(Option<Divider>),
    DividerPress(Divider),
    /// Cursor x while dragging a divider.
    DividerDrag(iced::Point),
    DividerRelease,
    WindowSize(iced::Size),
    FocusMode,
    CenterResized(pane_grid::ResizeEvent),
    Key(keyboard::Event, bool),
    WindowFocus(bool),
    Tick,
    StatusTick,

    // terminal
    Term(iced_term::Event),
    TermInput(termio::Input),
    RestartTerminal,
    /// Flush terminal commands queued by the gate after a quiet moment.
    TermFlush,

    // herdr and runs
    HerdrPolled(Result<Vec<Agent>, herdr::Error>),
    WorkspaceReady(Result<String, herdr::Error>),
    RunSnapshot(RunId, bool, Result<String, String>),
    RunFiles(RunId, Vec<Change>),
    RunPrompt(RunId, Option<String>),
    /// Select a run in the "by run" view (for its actions); again to clear.
    SelectRun(RunId),
    MarkAllSeen,
    /// Show Changes grouped by agent run instead of by file.
    GroupByRun(bool),
    OpenRunDiff(RunId, String),
    /// Previous/next file of the selected run.
    ReviewStep(isize),
    RevertRun(RunId),
    StageRun(RunId),
    Comment(String),
    SendComment,
    SendToAgent(String),
    /// Text typed into an agent's pane through herdr.
    SentToPane(Result<(), herdr::Error>),
    /// Show this agent's pane in herdr (switching workspace if needed).
    FocusAgent(PaneId),
    AgentFocused(Result<(), herdr::Error>),

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
    CommitEdit(text_editor::Action),
    Commit,
    /// Generate a commit message, or cancel the running generation.
    GenerateCommit,
    CommitGenerated(Result<String, String>),
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

/// Run blocking work (git, search, sockets) off the UI thread.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    match tokio::task::spawn_blocking(f).await {
        Ok(v) => v,
        // A panic in a worker is a bug; surface it instead of hiding it.
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
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
            let disk =
                std::fs::read(root.join(rel)).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            (git::show(root, "", rel), disk)
        }
        DiffSource::Trees { from, to, .. } => (git::show(root, from, rel), git::show(root, to, rel)),
    };
    diff::compute(&old, &new, if full { None } else { Some(3) })
}

fn swallow(
    c: &str,
    m: keyboard::Modifiers,
) -> (iced_term::bindings::Binding<iced_term::bindings::InputKind>, iced_term::bindings::BindingAction) {
    (
        iced_term::bindings::Binding {
            target: iced_term::bindings::InputKind::Char(c.to_string()),
            modifiers: m,
            terminal_mode_include: iced_term::TermMode::empty(),
            terminal_mode_exclude: iced_term::TermMode::empty(),
        },
        iced_term::bindings::BindingAction::Esc(String::new()),
    )
}

/// mira's `Ctrl+Shift+<letter>` shortcuts; swallowed by the terminal so they
/// work while it has focus. Never `c`/`v` (copy/paste).
const SHORTCUT_LETTERS: [&str; 6] = ["g", "f", "e", "m", "b", "t"];

impl App {
    pub fn new(root: PathBuf, fresh: bool) -> (Self, Task<Msg>) {
        let repo_prefix = git::repo_root(&root)
            .and_then(|r| root.strip_prefix(&r).ok().map(|p| p.to_string_lossy().into_owned()))
            .map(|p| if p.is_empty() { p } else { format!("{p}/") })
            .unwrap_or_default();

        let theme_choice =
            persist::config_get("theme").and_then(|v| theme::Choice::parse(&v)).unwrap_or(theme::Choice::Auto);
        theme::set_current(theme_choice.resolve());
        let width = |key: &str, default: f32| {
            persist::config_get(key).and_then(|v| v.parse::<f32>().ok()).filter(|w| w.is_finite()).unwrap_or(default)
        };
        let herdr_session = persist::config_get("herdr_session").filter(|s| herdr::valid_session(s));
        let use_herdr = herdr::installed() && persist::config_get("terminal").as_deref() != Some("shell");
        let runs = if fresh { Vec::new() } else { persist::load(&root).map(|s| s.runs).unwrap_or_default() };

        let mut app = Self {
            inspector_width: width("inspector_width", 400.0),
            window_width: 1400.0,
            window_height: 900.0,
            history_height: width("history_height", 180.0).max(MIN_HISTORY),
            drag_anchor: None,
            ticks: 0,
            dragging: None,
            hovered_divider: None,
            theme: theme_choice,
            explorer: Explorer::new(root.clone()),
            root,
            center: pane_grid::State::new(CenterPane::Terminal).0,
            doc_ratio: 0.55,
            show_inspector: true,
            focus_mode: false,
            inspector: Inspector::Changes,
            window_focused: true,
            term: None,
            term_kind: if use_herdr { TermKind::Herdr } else { TermKind::Shell },
            term_cell: None,
            focus_epoch: 0,
            next_term_id: 0,
            gate: termio::Gate::default(),
            flush_scheduled: false,
            herdr: if use_herdr { herdr::Client::for_session(herdr_session.as_deref()) } else { None },
            herdr_session,
            polling: false,
            workspace: None,
            ensuring_workspace: false,
            workspace_retry: Instant::now(),
            agents: Vec::new(),
            herdr_error: None,
            runs: Tracker::from_runs(runs),
            snapshot_queue: std::collections::VecDeque::new(),
            snapshot_running: false,
            snapshots_off: false,
            selected_run: None,
            group_by_run: persist::config_get("changes_view").as_deref() == Some("runs"),
            confirm_revert: None,
            comment: String::new(),
            state_dirty: false,
            tabs: Vec::new(),
            active_tab: None,
            pending_close: None,
            git: Snapshot::default(),
            repo_prefix,
            refreshing: false,
            git_busy: false,
            commit_msg: Content::new(),
            commit_gen: None,
            search: SearchState::default(),
            status: None,
            status_at: Instant::now(),
        };
        if !use_herdr {
            app.set_status("herdr not found: running a plain shell (install herdr for agent tracking)", false);
        }
        let term = app.spawn_terminal();
        let refresh = app.refresh_git();
        let files = app.load_all_run_files();
        (app, Task::batch([term, refresh, files]))
    }

    pub fn title(&self) -> String {
        format!("{} — mira", file_name(&self.root))
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        let mut subs: Vec<Subscription<Msg>> = Vec::new();
        if let Some(t) = &self.term {
            subs.push(t.subscription().map(Msg::Term));
        }
        subs.push(iced::time::every(Duration::from_secs(2)).map(|_| Msg::Tick));
        subs.push(iced::time::every(Duration::from_millis(700)).map(|_| Msg::StatusTick));
        subs.push(iced::event::listen_with(|event, status, _| match event {
            iced::Event::Keyboard(k @ keyboard::Event::KeyPressed { .. }) => {
                Some(Msg::Key(k, status == iced::event::Status::Captured))
            }
            iced::Event::Window(iced::window::Event::Focused) => Some(Msg::WindowFocus(true)),
            iced::Event::Window(iced::window::Event::Unfocused) => Some(Msg::WindowFocus(false)),
            iced::Event::Window(iced::window::Event::Opened { size, .. } | iced::window::Event::Resized(size)) => {
                Some(Msg::WindowSize(size))
            }
            _ => None,
        }));
        // Only follow the mouse while a divider is being dragged.
        if self.dragging.is_some() {
            subs.push(iced::event::listen_with(|event, _, _| match event {
                iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => Some(Msg::DividerDrag(position)),
                iced::Event::Mouse(
                    iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left) | iced::mouse::Event::CursorLeft,
                ) => Some(Msg::DividerRelease),
                _ => None,
            }));
        }
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

    // ---------------------------------------------------------------- terminal

    fn spawn_terminal(&mut self) -> Task<Msg> {
        let (program, args) = match self.term_kind {
            TermKind::Herdr => herdr::launch_command(self.herdr_session.as_deref()),
            TermKind::Shell => (std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()), Vec::new()),
        };
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("TERM_PROGRAM".to_string(), "mira".to_string());
        let settings = iced_term::settings::Settings {
            backend: iced_term::settings::BackendSettings {
                program,
                args,
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
        let id = self.next_term_id;
        self.next_term_id += 1;
        let mut term = match iced_term::Terminal::new(id, settings) {
            Ok(t) => t,
            Err(e) => {
                self.set_status(format!("terminal: {e}"), true);
                return Task::none();
            }
        };
        let cs = keyboard::Modifiers::CTRL | keyboard::Modifiers::SHIFT;
        let mut bindings: Vec<_> = SHORTCUT_LETTERS.iter().map(|c| swallow(c, cs)).collect();
        bindings.push(swallow("`", keyboard::Modifiers::CTRL));
        term.handle(iced_term::Command::AddBindings(bindings));
        self.term = Some(term);
        self.gate = termio::Gate::default();
        self.term_cell = None;
        self.workspace = None;
        self.focus_terminal()
    }

    fn focus_terminal(&mut self) -> Task<Msg> {
        self.focus_epoch += 1;
        self.term.as_ref().map_or_else(Task::none, |t| iced_term::TerminalView::focus(t.widget_id().clone()))
    }

    fn term_write(&mut self, bytes: Vec<u8>) {
        self.term_send(iced_term::Command::ProxyToBackend(BackendCommand::Write(bytes)));
    }

    /// Every command for the terminal goes through the gate (see `termio::Gate`).
    fn term_send(&mut self, cmd: iced_term::Command) {
        if let termio::Gated::Forward(cmds) = self.gate.command(cmd) {
            self.term_apply(cmds);
        }
    }

    fn term_apply(&mut self, cmds: Vec<iced_term::Command>) {
        let Some(t) = &mut self.term else { return };
        let mut shutdown = false;
        for cmd in cmds {
            shutdown |= t.handle(cmd) == iced_term::actions::Action::Shutdown;
        }
        if shutdown {
            self.term = None;
            self.gate = termio::Gate::default();
            let what = if self.term_kind == TermKind::Herdr { "herdr detached" } else { "The shell exited" };
            self.set_status(format!("{what} — agents keep running in herdr"), false);
        }
    }

    /// Make sure a queued batch gets flushed even if it never sees `Wakeup`.
    fn schedule_term_flush(&mut self) -> Task<Msg> {
        if !self.gate.busy() || self.flush_scheduled {
            return Task::none();
        }
        self.flush_scheduled = true;
        Task::perform(blocking(|| std::thread::sleep(termio::GATE_QUIET)), |()| Msg::TermFlush)
    }

    fn on_term_call(&mut self, cmd: BackendCommand) -> Task<Msg> {
        match cmd {
            BackendCommand::ProcessAlacrittyEvent(event) => {
                // OSC 52 store from apps in the terminal. Loads are not answered:
                // letting programs read the clipboard is a data leak.
                if let AlacrittyEvent::ClipboardStore(_, text) = &event {
                    return iced::clipboard::write(text.clone());
                }
                let color = |i| {
                    let (r, g, b) = termio::color_reply_rgb(i);
                    Rgb { r, g, b }
                };
                match self.gate.event(event, Instant::now(), color) {
                    termio::Gated::Forward(cmds) => self.term_apply(cmds),
                    termio::Gated::Queued => return self.schedule_term_flush(),
                }
            }
            cmd => {
                // The backend truncates the cell size to whole pixels; match it.
                if let BackendCommand::Resize(_, Some(cell)) = &cmd {
                    self.term_cell = Some(iced::Size::new(cell.width.floor().max(1.0), cell.height.floor().max(1.0)));
                }
                self.term_send(iced_term::Command::ProxyToBackend(cmd));
                return self.schedule_term_flush();
            }
        }
        Task::none()
    }

    fn on_term_input(&mut self, input: termio::Input) -> Task<Msg> {
        match input {
            termio::Input::Mouse { button, pressed, line, col, mods } => {
                let button = match button {
                    termio::Button::Middle => iced_term::MouseButton::MiddleButton,
                    termio::Button::Right => iced_term::MouseButton::RightButton,
                    termio::Button::WheelUp => iced_term::MouseButton::ScrollUp,
                    termio::Button::WheelDown => iced_term::MouseButton::ScrollDown,
                };
                let point = GridPoint::new(Line(i32::try_from(line).unwrap_or(i32::MAX)), Column(col));
                self.term_send(iced_term::Command::ProxyToBackend(BackendCommand::MouseReport(
                    button, mods, point, pressed,
                )));
            }
            termio::Input::Bytes(b) => self.term_write(b),
            termio::Input::Paste(text) => self.term_write(termio::bracketed(&text)),
        }
        self.schedule_term_flush()
    }

    // ------------------------------------------------------------------- herdr

    fn poll_herdr(&mut self) -> Task<Msg> {
        let Some(client) = self.herdr.clone() else { return Task::none() };
        if self.polling {
            return Task::none();
        }
        self.polling = true;
        Task::perform(blocking(move || client.agents()), Msg::HerdrPolled)
    }

    fn on_agents(&mut self, agents: Vec<Agent>) -> Task<Msg> {
        let root = self.root.clone();
        let here: Vec<Agent> = agents.into_iter().filter(|a| a.dir().is_some_and(|d| d.starts_with(&root))).collect();
        let seen: Vec<Seen> = here
            .iter()
            .map(|a| Seen {
                pane: a.pane_id.clone(),
                status: a.agent_status,
                seq: a.completion_seq,
                agent: a.label(),
                kind: a.agent.clone(),
                dir: a.dir().map_or_else(|| root.clone(), Path::to_path_buf),
            })
            .collect();
        self.agents = here;
        let effects = self.runs.observe(&seen, persist::now());
        if !effects.is_empty() {
            self.state_dirty = true;
        }
        let mut tasks: Vec<Task<Msg>> = effects.into_iter().map(|e| self.run_effect(e)).collect();
        if self.workspace.is_none() && !self.ensuring_workspace && Instant::now() >= self.workspace_retry {
            tasks.push(self.ensure_workspace());
        }
        Task::batch(tasks)
    }

    fn ensure_workspace(&mut self) -> Task<Msg> {
        let Some(client) = self.herdr.clone() else { return Task::none() };
        self.ensuring_workspace = true;
        let (root, label) = (self.root.clone(), file_name(&self.root));
        Task::perform(blocking(move || client.ensure_workspace(&root, &label)), Msg::WorkspaceReady)
    }

    fn run_effect(&mut self, effect: Effect) -> Task<Msg> {
        let (id, after) = match effect {
            Effect::Before(id) => (id, false),
            Effect::After(id) => (id, true),
        };
        let mut tasks = Vec::new();
        if self.has_repo() && !self.snapshots_off {
            self.snapshot_queue.push_back((id, after));
            tasks.push(self.next_snapshot());
        }
        if after
            && let Some(run) = self.runs.get(id)
            && let (Some(kind), Some(home)) = (run.kind.clone(), std::env::var_os("HOME"))
        {
            let (dir, start, end) = (run.dir.clone(), run.started, run.finished.unwrap_or(run.started));
            let home = PathBuf::from(home);
            tasks.push(Task::perform(
                blocking(move || crate::agentlog::last_prompt(&home, &kind, &dir, start, end)),
                move |p| Msg::RunPrompt(id, p),
            ));
        }
        Task::batch(tasks)
    }

    /// Start the next queued snapshot unless one is running.
    fn next_snapshot(&mut self) -> Task<Msg> {
        if self.snapshot_running || self.snapshots_off {
            return Task::none();
        }
        let Some((id, after)) = self.snapshot_queue.pop_front() else { return Task::none() };
        self.snapshot_running = true;
        let root = self.repo_root();
        Task::perform(blocking(move || git::snapshot_tree(&root)), move |r| Msg::RunSnapshot(id, after, r))
    }

    /// Changed files of every run whose trees are known (after a restore).
    fn load_all_run_files(&self) -> Task<Msg> {
        let ids: Vec<RunId> = self.runs.runs.iter().filter(|r| r.trees().is_some()).map(|r| r.id).collect();
        Task::batch(ids.into_iter().map(|id| self.load_run_files(id)))
    }

    fn load_run_files(&self, id: RunId) -> Task<Msg> {
        let Some((from, to)) = self.runs.get(id).and_then(|r| r.trees()) else { return Task::none() };
        let (root, from, to) = (self.repo_root(), from.to_string(), to.to_string());
        Task::perform(blocking(move || git::diff_trees(&root, &from, &to).unwrap_or_default()), move |f| {
            Msg::RunFiles(id, f)
        })
    }

    fn save_state(&mut self) {
        if !self.state_dirty {
            return;
        }
        self.state_dirty = false;
        let state = persist::State { root: self.root.clone(), runs: self.runs.runs.clone() };
        if let Err(e) = persist::save(&state) {
            self.set_status(format!("Could not save review state: {e}"), true);
        }
    }

    /// Activate the palette of the current choice; recolor the terminal if it changed.
    fn apply_theme(&mut self) {
        if theme::set_current(self.theme.resolve()) {
            self.term_send(iced_term::Command::ChangeTheme(Box::new(theme::terminal_palette())));
        }
    }

    // ------------------------------------------------------------------ review

    /// Diff source of run `id`.
    pub fn run_source(&self, id: RunId) -> Option<DiffSource> {
        let run = self.runs.get(id)?;
        let (from, to) = run.trees()?;
        Some(DiffSource::Trees { from: from.to_string(), to: to.to_string(), label: run.agent.clone(), run: id })
    }

    fn mark_seen(&mut self, path: &str) {
        if self.runs.mark_seen(path) {
            self.state_dirty = true;
        }
    }

    /// Type `text` without pressing Enter: into the pane of run `target`
    /// through herdr when that agent is still there, else into the terminal
    /// (herdr's focused pane) as a paste.
    fn send_to_agent(&mut self, text: String, target: Option<RunId>) -> Task<Msg> {
        let pane: Option<PaneId> = target
            .and_then(|id| self.runs.get(id))
            .map(|r| r.pane.clone())
            .filter(|p| self.agents.iter().any(|a| &a.pane_id == p));
        let focus = self.focus_terminal();
        match (pane, self.herdr.clone()) {
            (Some(pane), Some(client)) => {
                let task = Task::perform(
                    blocking(move || {
                        client.send_text(&pane, &text)?;
                        client.focus_pane(&pane)
                    }),
                    Msg::SentToPane,
                );
                Task::batch([task, focus])
            }
            _ => {
                let bytes =
                    if self.term_kind == TermKind::Herdr { termio::bracketed(&text) } else { text.into_bytes() };
                self.term_write(bytes);
                let msg = if target.is_some() {
                    "That agent is gone — typed into the focused pane instead"
                } else {
                    "Typed into the focused pane — review and press Enter"
                };
                self.set_status(msg, target.is_some());
                focus
            }
        }
    }

    // --------------------------------------------------------------- documents

    /// Add or remove the document pane. The terminal pane is split or
    /// restored in place, never rebuilt, so its widget state (focus) survives.
    fn sync_center(&mut self) {
        let want_doc = !self.tabs.is_empty() && !self.focus_mode;
        let find = |kind| self.center.iter().find(|(_, k)| **k == kind).map(|(p, _)| *p);
        match (want_doc, find(CenterPane::Document), find(CenterPane::Terminal)) {
            (true, None, Some(term)) => {
                if let Some((doc, split)) = self.center.split(Axis::Horizontal, term, CenterPane::Document) {
                    // The new pane lands below; documents go on top.
                    self.center.swap(doc, term);
                    self.center.resize(split, self.doc_ratio);
                }
            }
            (false, Some(doc), _) => {
                let _ = self.center.close(doc);
            }
            _ => {}
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
            t.content
                .move_to(text_editor::Cursor { position: text_editor::Position { line, column: 0 }, selection: None });
        }
        self.active_tab = Some(idx);
        self.focus_mode = false;
        self.sync_center();
    }

    fn open_diff(&mut self, rel: String, source: DiffSource) -> Task<Msg> {
        let idx = self.tabs.iter().position(|t| matches!(t, Tab::Diff(d) if d.rel == rel && d.source == source));
        let tab =
            |rel, source| Tab::Diff(DiffTab { rel, source, full: false, diff: None, anchor: None, loaded_mtime: None });
        let idx = match idx {
            Some(i) => i,
            // Review diffs share one preview tab instead of piling up.
            None if matches!(source, DiffSource::Trees { .. }) => {
                match self
                    .tabs
                    .iter()
                    .position(|t| matches!(t, Tab::Diff(d) if matches!(d.source, DiffSource::Trees { .. })))
                {
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

    fn reload_active_diff(&mut self) -> Task<Msg> {
        let root = self.repo_root();
        let Some(Tab::Diff(d)) = self.active_tab.and_then(|i| self.tabs.get_mut(i)) else {
            return Task::none();
        };
        d.loaded_mtime = mtime(&root.join(&d.rel));
        let (rel, source, full) = (d.rel.clone(), d.source.clone(), d.full);
        Task::perform(
            blocking(move || {
                let diff = load_diff(&root, &rel, &source, full);
                (rel, source, full, diff)
            }),
            |(rel, source, full, diff)| Msg::DiffLoaded(rel, source, full, diff),
        )
    }

    /// The active diff compares against the working tree and the file changed
    /// since it was computed. Run diffs (between snapshots) never go stale.
    fn active_diff_stale(&self) -> bool {
        let root = self.repo_root();
        matches!(
            self.active_tab.and_then(|i| self.tabs.get(i)),
            Some(Tab::Diff(d)) if matches!(d.source, DiffSource::Index { .. }) && mtime(&root.join(&d.rel)) != d.loaded_mtime
        )
    }

    fn active_diff_is_index(&self) -> bool {
        matches!(self.active_tab.and_then(|i| self.tabs.get(i)), Some(Tab::Diff(d)) if matches!(d.source, DiffSource::Index { .. }))
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
            Msg::InspectorTab(i) => self.inspector = i,
            Msg::ToggleInspector => {
                self.show_inspector = !self.show_inspector;
                self.focus_mode = false;
                self.sync_center();
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
                // herdr enables focus reporting (mode 1004) and redraws on focus.
                if self.term_kind == TermKind::Herdr {
                    self.term_write(if f { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
                }
            }
            Msg::Tick => {
                self.ticks = self.ticks.wrapping_add(1);
                if self.theme == theme::Choice::Auto {
                    self.apply_theme();
                }
                // In the background, refresh every 10 s instead of every 2 s.
                if !self.window_focused && !self.ticks.is_multiple_of(5) {
                    return Task::none();
                }
                // The tree is only on screen in Files.
                if self.show_inspector && self.inspector == Inspector::Files {
                    self.explorer.refresh();
                }
                self.reload_changed_files();
                let diff = if self.active_diff_stale() { self.reload_active_diff() } else { Task::none() };
                return Task::batch([self.refresh_git(), diff]);
            }
            Msg::StatusTick => {
                let ttl = if self.status.as_ref().is_some_and(|(_, e)| *e) { 12 } else { 6 };
                if self.status.is_some() && self.status_at.elapsed() > Duration::from_secs(ttl) {
                    self.status = None;
                    self.confirm_revert = None;
                }
                self.save_state();
                return self.poll_herdr();
            }
            Msg::DividerHover(d) => self.hovered_divider = d,
            Msg::DividerPress(d) => {
                self.dragging = Some(d);
                self.drag_anchor = None;
            }
            Msg::DividerDrag(pos) => match self.dragging {
                Some(Divider::Inspector) => {
                    let max = (self.window_width - MIN_CENTER).max(MIN_SIDE);
                    self.inspector_width = (self.window_width - pos.x - DIVIDER / 2.0).clamp(MIN_SIDE, max);
                }
                // Relative to where the drag started: the panel's top edge isn't known here.
                Some(Divider::History) => {
                    let (start_y, start_h) = *self.drag_anchor.get_or_insert((pos.y, self.history_height));
                    let max = (self.window_height - 280.0 - MIN_CHANGES_LIST).max(MIN_HISTORY);
                    self.history_height = (start_h - (pos.y - start_y)).clamp(MIN_HISTORY, max);
                }
                None => {}
            },
            Msg::DividerRelease => {
                let saved = match self.dragging.take() {
                    Some(Divider::Inspector) => {
                        persist::config_set("inspector_width", &format!("{:.0}", self.inspector_width))
                    }
                    Some(Divider::History) => {
                        persist::config_set("history_height", &format!("{:.0}", self.history_height))
                    }
                    None => Ok(()),
                };
                if let Err(e) = saved {
                    self.set_status(format!("Could not save layout: {e}"), true);
                }
            }
            Msg::WindowSize(size) => {
                self.window_width = size.width;
                self.window_height = size.height;
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

            Msg::Term(iced_term::Event::BackendCall(_, cmd)) => return self.on_term_call(cmd),
            Msg::TermInput(input) => return self.on_term_input(input),
            Msg::TermFlush => {
                self.flush_scheduled = false;
                if self.gate.flush_due(Instant::now()) {
                    let cmds = self.gate.flush();
                    self.term_apply(cmds);
                }
                return self.schedule_term_flush();
            }
            Msg::RestartTerminal => return self.spawn_terminal(),

            Msg::HerdrPolled(result) => {
                self.polling = false;
                match result {
                    Ok(agents) => {
                        self.herdr_error = None;
                        return self.on_agents(agents);
                    }
                    // No server: its agents are gone (or the client in the
                    // terminal is still starting it). Close what was open.
                    Err(herdr::Error::NotRunning) => {
                        self.herdr_error = None;
                        return self.on_agents(Vec::new());
                    }
                    Err(e) => self.herdr_error = Some(e.to_string()),
                }
            }
            Msg::WorkspaceReady(result) => {
                self.ensuring_workspace = false;
                match result {
                    Ok(ws) => self.workspace = Some(ws),
                    Err(e) => {
                        self.workspace_retry = Instant::now() + Duration::from_secs(30);
                        self.set_status(format!("herdr workspace: {e}"), true);
                    }
                }
            }
            Msg::RunSnapshot(id, after, result) => {
                self.snapshot_running = false;
                let tree = match result {
                    Ok(t) => t,
                    Err(e) if e.starts_with(git::SNAPSHOT_TIMED_OUT) => {
                        // Too big to track: stop instead of piling up git processes.
                        self.snapshots_off = true;
                        self.snapshot_queue.clear();
                        self.set_status(format!("{e}: agent change tracking is off for this folder"), true);
                        return Task::none();
                    }
                    Err(e) => {
                        self.set_status(format!("snapshot: {e}"), true);
                        return self.next_snapshot();
                    }
                };
                let next = self.next_snapshot();
                let Some(run) = self.runs.get_mut(id) else { return next };
                // A late "after" of a run that reopened (quick follow-up) is stale.
                if after && run.running() {
                    return next;
                }
                if after {
                    run.after = Some(tree)
                } else {
                    run.before = Some(tree)
                }
                self.state_dirty = true;
                // The two snapshots race; whichever lands second computes the files.
                if (after || run.files.is_none()) && run.trees().is_some() {
                    return Task::batch([next, self.load_run_files(id)]);
                }
                return next;
            }
            Msg::RunFiles(id, files) => {
                if let Some(run) = self.runs.get_mut(id)
                    && !run.running()
                {
                    run.files = Some(files);
                }
            }
            Msg::RunPrompt(id, prompt) => {
                // The first title wins: a reopened run keeps the prompt that started it.
                if let Some(run) = self.runs.get_mut(id)
                    && prompt.is_some()
                    && run.prompt.is_none()
                {
                    run.prompt = prompt;
                    self.state_dirty = true;
                }
            }
            Msg::SelectRun(id) => {
                self.selected_run = if self.selected_run == Some(id) { None } else { Some(id) };
                self.confirm_revert = None;
            }
            Msg::MarkAllSeen => {
                self.runs.mark_all_seen();
                self.state_dirty = true;
            }
            Msg::GroupByRun(on) => {
                self.group_by_run = on;
                if let Err(e) = persist::config_set("changes_view", if on { "runs" } else { "files" }) {
                    self.set_status(format!("Could not save view: {e}"), true);
                }
            }
            Msg::OpenRunDiff(id, rel) => {
                self.selected_run = Some(id);
                self.mark_seen(&rel);
                if let Some(source) = self.run_source(id) {
                    return self.open_diff(rel, source);
                }
            }
            Msg::ReviewStep(step) => {
                let Some(Tab::Diff(d)) = self.active_tab.and_then(|i| self.tabs.get(i)) else {
                    return Task::none();
                };
                let DiffSource::Trees { run: id, .. } = d.source else { return Task::none() };
                let files = self.runs.get(id).and_then(|r| r.files.as_ref());
                if let Some((i, files)) = files.and_then(|f| f.iter().position(|c| c.path == d.rel).map(|i| (i, f))) {
                    let next = (i as isize + step).rem_euclid(files.len() as isize) as usize;
                    if let Some(c) = files.get(next) {
                        return self.update(Msg::OpenRunDiff(id, c.path.clone()));
                    }
                }
            }
            Msg::RevertRun(id) => {
                if self.confirm_revert != Some(id) {
                    self.confirm_revert = Some(id);
                    self.set_status("Click Revert again to restore these files (later edits to them are lost)", true);
                    return Task::none();
                }
                self.confirm_revert = None;
                let Some(run) = self.runs.get(id) else { return Task::none() };
                let (Some((from, _)), Some(files)) = (run.trees(), run.files.as_ref()) else { return Task::none() };
                let from = from.to_string();
                let paths: Vec<String> = files.iter().map(|c| c.path.clone()).collect();
                for p in &paths {
                    self.mark_seen(p);
                }
                return self.git_op(move |r| git::restore_from_tree(r, &from, &paths));
            }
            Msg::StageRun(id) => {
                let Some(paths) = self
                    .runs
                    .get(id)
                    .and_then(|r| r.files.as_ref())
                    .map(|f| f.iter().map(|c| c.path.clone()).collect::<Vec<_>>())
                else {
                    return Task::none();
                };
                for p in &paths {
                    self.mark_seen(p);
                }
                return self.git_op(move |r| git::stage(r, &paths));
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
                // A run's diff goes to the agent that made it; others to the focused pane.
                let target = match &d.source {
                    DiffSource::Trees { run, .. } => Some(*run),
                    DiffSource::Index { .. } => self.runs.origin(&d.rel).map(|r| r.id),
                };
                self.comment.clear();
                return self.send_to_agent(format!("En {place} — {comment}"), target);
            }
            Msg::SendToAgent(text) => return self.send_to_agent(text, None),
            Msg::FocusAgent(pane) => {
                let focus = self.focus_terminal();
                let Some(client) = self.herdr.clone() else { return focus };
                let task = Task::perform(blocking(move || client.focus_pane(&pane)), Msg::AgentFocused);
                return Task::batch([task, focus]);
            }
            Msg::AgentFocused(result) => {
                if let Err(e) = result {
                    self.set_status(e.to_string(), true);
                }
            }
            Msg::SentToPane(result) => match result {
                Ok(()) => self.set_status("Sent to the agent — review and press Enter", false),
                Err(e) => self.set_status(e.to_string(), true),
            },

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
                        && d.rel == rel
                        && d.source == source
                        && d.full == full
                    {
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
                    // Only a real change re-renders the panel and recomputes the diff.
                    Ok(snap) if snap != self.git => {
                        self.git = snap;
                        if self.active_diff_is_index() {
                            return self.reload_active_diff();
                        }
                    }
                    Ok(_) => {}
                    Err(e) => self.set_status(e, true),
                }
            }
            Msg::OpenDiff(rel, staged) => {
                self.mark_seen(&rel);
                return self.open_diff(rel, DiffSource::Index { staged });
            }
            Msg::Stage(paths) => return self.git_op(move |r| git::stage(r, &paths)),
            Msg::Unstage(paths) => return self.git_op(move |r| git::unstage(r, &paths)),
            Msg::CommitEdit(action) => self.commit_msg.perform(action),
            Msg::Commit => {
                let message = self.commit_msg.text().trim().to_string();
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
                    git::commit(r, &message).map(|out| format!("COMMIT:{}", first_line(&out).unwrap_or("Committed")))
                });
            }
            Msg::GenerateCommit => {
                if let Some(cancel) = self.commit_gen.take() {
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    self.set_status("Commit message generation cancelled", false);
                    return Task::none();
                }
                if self.git.root.is_none() {
                    return Task::none();
                }
                let staged = !self.git.staged.is_empty();
                if !staged && self.git.unstaged.is_empty() {
                    self.set_status("Nothing to commit", true);
                    return Task::none();
                }
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                self.commit_gen = Some(cancel.clone());
                let what = if staged { "staged changes" } else { "all changes" };
                self.set_status(format!("Generating a commit message for {what}…"), false);
                let root = self.repo_root();
                return Task::perform(
                    blocking(move || crate::commitgen::generate(&root, staged, cancel)),
                    Msg::CommitGenerated,
                );
            }
            Msg::CommitGenerated(result) => {
                // A cancelled run already cleared `commit_gen`; drop its result.
                if self.commit_gen.take().is_none() {
                    return Task::none();
                }
                match result {
                    Ok(message) => {
                        self.commit_msg = Content::with_text(&message);
                        self.set_status("Commit message ready — review it, then Commit", false);
                    }
                    Err(e) => self.set_status(format!("Commit message: {e}"), true),
                }
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
                            self.commit_msg = Content::new();
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
        let backquote = matches!(physical_key, keyboard::key::Physical::Code(keyboard::key::Code::Backquote));
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
            // Plain Ctrl+B is herdr's prefix.
            (Some('b'), true) => return self.update(Msg::ToggleInspector),
            (Some('g'), true) => return self.update(Msg::Inspector(Inspector::Changes)),
            (Some('f'), true) => return self.update(Msg::Inspector(Inspector::Search)),
            (Some('e'), true) => return self.update(Msg::Inspector(Inspector::Files)),
            (Some('t'), true) => return self.update(Msg::CycleTheme),
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
