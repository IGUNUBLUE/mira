//! Application state and update logic. Views live in `view.rs`.
//!
//! Model: the center terminal runs herdr, where the user talks to agents.
//! mira polls herdr's socket for the agents working in this folder; every
//! working period is a *run* (`runs.rs`) with before/after snapshots of the
//! working tree, listed in the Review inspector as an inbox.
//!
//! `update` dispatches each `Msg` to the child module that owns it; each one
//! adds an `impl App` block: `layout`, `terminal`, `follow` (herdr), `review`,
//! `documents`, `scm` (git and commit messages), `find` (Search), `navigate`
//! (find in file, go to line, quick open) and `keys`.

mod documents;
mod files;
mod find;
mod follow;
mod keys;
mod layout;
mod navigate;
mod review;
mod scm;
mod settings;
mod terminal;

pub use documents::{AutoSave, DiffSource, DiffTab, EDITOR_PADDING_TOP, FileTab, ImageData, ImageTab, Tab};
pub use files::{EditKind, ExplorerMenu, MenuAction, NAME_INPUT, NameEdit};
pub use find::SearchState;
pub use layout::{COMMIT_INPUT, CenterPane, Divider, Inspector, SEARCH_INPUT};
pub use navigate::{EDITOR, FIND_INPUT, FindBar, PALETTE_INPUT, QuickOpen};
pub use scm::{BusyOp, CommitGen, GitOutcome, HunkOp, SPINNER, SignIn, SignInEvent, SignInStage};
pub use settings::{Dialog, Drafts, Field, MenuItem, REPO_URL, SHORTCUTS, Setting, WindowControls};
pub use terminal::TermKind;

use crate::diff::FileDiff;
use crate::explorer::Explorer;
use crate::git::{Change, Snapshot};
use crate::herdr::{self, Agent, PaneId};
use crate::persist;
use crate::runs::{RunId, Tracker};
use crate::search::FileMatches;
use crate::theme;
use follow::focused_root;
use layout::MIN_HISTORY;

use iced::keyboard;
use iced::widget::pane_grid;
use iced::widget::text_editor::{self, Content};
use iced::{Subscription, Task};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const CODE_SIZE: u32 = 14;

pub struct App {
    pub root: PathBuf,
    pub center: pane_grid::State<CenterPane>,
    pub doc_ratio: f32,
    pub show_inspector: bool,
    pub focus_mode: bool,
    pub inspector: Inspector,
    pub window_focused: bool,
    autosave: AutoSave,
    // terminal
    pub term: Option<crate::term::Terminal>,
    pub term_kind: TermKind,
    next_term_id: u64,

    // herdr
    herdr: Option<herdr::Client>,
    herdr_session: Option<String>,
    /// herdr isn't installed (and the config didn't ask for a plain shell).
    pub herdr_missing: bool,
    polling: bool,
    workspace: Option<String>,
    /// Started without a folder before herdr was up: take the folder of
    /// herdr's focused workspace as soon as it answers.
    adopt_focused: bool,
    /// Bumped when the folder changes; async results of an older folder are dropped.
    root_epoch: u64,
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
    /// "Replace all" was pressed once; the next press replaces.
    pub confirm_replace: bool,
    /// The account button was pressed once; the next press signs out.
    pub confirm_signout: bool,
    /// "Discard all" waits for its second click.
    pub confirm_discard_all: bool,
    /// The last discard: the snapshot taken just before it and the files, for Undo.
    undo_discard: Option<(String, Vec<String>)>,
    /// The Changes row under the mouse (section, index): its buttons show.
    pub hovered_change: Option<(u8, usize)>,
    /// A button next to the status message (e.g. Undo after a discard).
    pub status_action: Option<(&'static str, Msg)>,
    pub comment: String,
    state_dirty: bool,

    // documents
    pub tabs: Vec<Tab>,
    /// Height of the file editor's text area, once laid out (for the line numbers).
    pub editor_view: Option<f32>,
    pub active_tab: Option<usize>,
    /// The dirty file tab a first close attempt warned about.
    pub pending_close: Option<PathBuf>,

    // inspectors
    pub explorer: Explorer,
    /// The open dialog (Settings, Shortcuts, About), and the gear menu.
    pub dialog: Option<Dialog>,
    pub gear_menu: bool,
    /// Settings typed but not saved yet, and why the last save failed.
    pub drafts: Drafts,
    pub settings_error: Option<String>,
    /// `herdr --version`, asked when About first opens.
    pub herdr_version: Option<String>,
    pub window_controls: WindowControls,
    /// The row a right click was on, until the panel reports where (see `rightclick`).
    explorer_target: Option<(PathBuf, bool)>,
    pub explorer_menu: Option<ExplorerMenu>,
    /// A name being typed in Files (new file or folder, rename).
    pub name_edit: Option<NameEdit>,
    /// The last delete, for Undo.
    undo_trash: Option<Vec<crate::fileops::Trashed>>,
    pub git: Snapshot,
    /// The opened folder relative to its repository ("" at the top), from the last snapshot.
    pub repo_prefix: String,
    pub refreshing: bool,
    /// Bumped per git refresh; an older refresh's snapshot is dropped.
    git_gen: u64,
    /// What git refreshes of this folder reuse (repository, log); replaced on a folder switch.
    git_cache: std::sync::Arc<std::sync::Mutex<crate::git::Cache>>,
    /// The latest refresh lists ignored files (only while Files is visible).
    git_with_ignored: bool,
    /// A snapshot of this folder arrived, so `git.root` says whether it is a repository.
    pub git_loaded: bool,
    pub git_busy: bool,
    /// The running git action whose button spins (commit, push, pull).
    pub busy_op: Option<BusyOp>,
    pub commit_msg: Content,
    commit_history: crate::history::History,
    /// The running commit-message generation.
    pub commit_gen: Option<CommitGen>,
    commit_gen_seq: u64,
    /// The ChatGPT sign-in dialog, while open.
    pub signin: Option<SignIn>,
    signin_seq: u64,
    /// Spinner frame while generating.
    pub spin: usize,
    /// Signed-in ChatGPT account (email, maybe empty), for the sign-out button.
    pub chatgpt_account: Option<String>,
    pub search: SearchState,
    /// The quick-open palette (Ctrl+P, Ctrl+G), while open.
    pub palette: Option<QuickOpen>,
    palette_seq: u64,
    /// Keyboard modifiers held now (Shift+Enter in the find bar).
    modifiers: keyboard::Modifiers,

    pub status: Option<(String, bool)>,
    /// The whole text behind a shortened status (e.g. git's stderr), for its tooltip.
    pub status_detail: Option<String>,
    status_at: Instant,
    /// How long the current status stays.
    status_ttl: Duration,
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
    /// `Msg::StatusTick` counter, for slower herdr polling in the background.
    status_ticks: u64,
    pub dragging: Option<Divider>,
    pub hovered_divider: Option<Divider>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Shortcut: show this inspector and focus its input (Search's query, the
    /// commit box). Never hides it.
    Inspector(Inspector),
    /// Tab click: just switch.
    InspectorTab(Inspector),
    ToggleInspector,
    CycleTheme,
    /// The status message was clicked: hide it.
    DismissStatus,
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
    CloseRequested,
    Tick,
    StatusTick,

    // terminal
    /// A batch of the terminal's events.
    Term(Vec<crate::term::Event>),
    /// Input from the terminal widget (keys, mouse, selection, resize).
    TermAction(crate::term::Action),
    RestartTerminal,
    /// Tell herdr light/dark once its client has started (see `report_color_scheme`).
    ReportColorScheme,

    // herdr and runs
    HerdrPolled(Result<(Vec<Agent>, Option<Focus>), herdr::Error>),
    /// A result computed for folder epoch `.0`; dropped if the folder changed since.
    ForRoot(u64, Box<Msg>),
    WorkspaceReady(Result<String, herdr::Error>),
    RunSnapshot(RunId, bool, Result<String, String>),
    RunFiles(RunId, Result<Vec<Change>, String>),
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
    /// Show this SVG (open as text) rendered, in an image tab.
    OpenImage(PathBuf),
    SelectTab(usize),
    CloseTab(usize),
    Edit(text_editor::Action),
    /// The file editor got this size (its line numbers need the height).
    EditorViewport(iced::Size),
    Save,
    Undo,
    Redo,
    /// Save the tabs whose edits paused long enough (`AutoSave::AfterDelay`).
    AutoSaveTick,
    DiffLoaded(String, DiffSource, bool, std::sync::Arc<FileDiff>),
    DiffFull(bool),
    DiffAnchor(usize),
    /// Open the active diff's file at the anchored line.
    DiffOpenFile,
    /// Send the active file's `path:line` (cursor line) to the agent.
    SendLine,

    // settings
    /// Open (`true`) or close the gear menu.
    GearMenu(bool),
    Gear(MenuItem),
    OpenDialog(Dialog),
    CloseDialog,
    HerdrVersion(Option<String>),
    SettingChoice(Setting),
    SettingText(Field, String),
    /// Enter in a typed setting: save the typed ones.
    SettingSubmit,
    OpenConfigFile,
    OpenLink(String),
    CopyAbout,
    WindowMinimize,
    WindowMaximize,
    WindowDrag,

    // files
    ExplorerClick(PathBuf, bool),
    /// Right press on a Files row: what (`path`, `is_dir`); [`Msg::ExplorerMenuAt`] follows.
    ExplorerTarget(PathBuf, bool),
    /// Right press in Files, in window coordinates: open the context menu there.
    ExplorerMenuAt(iced::Point),
    ExplorerMenuClose,
    ExplorerMenu(MenuAction),
    ExplorerName(String),
    ExplorerNameSubmit,
    ExplorerNameCancel,
    TrashDone(Result<files::TrashOutcome, String>),
    UndoTrash,

    // git
    /// A snapshot from refresh `.0` (see `git_gen`).
    GitLoaded(u64, Result<Snapshot, String>),
    OpenDiff(String, bool),
    Stage(Vec<String>),
    Unstage(Vec<String>),
    /// Throw away the unstaged changes of these files (a snapshot keeps an Undo).
    Discard(Vec<String>),
    /// Discard every listed change (asks for a second click).
    DiscardAll,
    UndoDiscard,
    /// A per-change action in the active diff: hunk `.2` of the diff with id `.1`
    /// (ignored if the diff was reloaded since it was drawn).
    Hunk(HunkOp, u64, usize),
    /// Review a section's changes file by file: open the first diff (`true` = staged).
    OpenChanges(bool),
    OpenChangedFile(String),
    /// The mouse entered (`true`) or left a Changes row (section, index).
    HoverChange((u8, usize), bool),
    CommitEdit(text_editor::Action),
    CommitUndo,
    CommitRedo,
    Commit,
    /// Make the opened folder a git repository.
    GitInit,
    /// Generate a commit message, or cancel the running generation.
    GenerateCommit,
    CommitGen(u64, crate::commitgen::Event),
    Spin,
    ChatGptSignOut,
    /// The sign-in dialog: open the browser, stop, copy the link, progress.
    SignInStart,
    SignInCancel,
    SignInCopyLink,
    SignInEvent(u64, SignInEvent),
    Push,
    Pull,
    /// A git action started in folder epoch `.0` finished.
    GitDone(u64, Result<GitOutcome, String>),

    // search
    SearchQuery(String),
    SearchReplace(String),
    SearchToggle(u8),
    SearchRun,
    /// Typing paused: search for generation `.0` if nothing newer was typed.
    SearchDebounced(u64),
    SearchDone(u64, Result<(Vec<FileMatches>, usize), String>),
    /// Open result `(results generation, file, line)`; `None` line: the file's top.
    SearchOpen(u64, usize, Option<usize>),
    /// Send a result's `path:line` to the agent `(results generation, file, line)`.
    SearchSend(u64, usize, usize),
    ReplaceAll,
    /// `.0`: the results were capped, so files past the cap were not replaced.
    ReplaceDone(bool, Result<usize, String>),

    // navigate
    /// The keyboard modifiers changed.
    Modifiers(keyboard::Modifiers),
    /// Ctrl+F: open the active file's find bar (or focus it), seeded with the selection.
    FindOpen,
    FindQuery(String),
    FindToggleCase,
    /// Next (`true`) or previous match from the cursor.
    FindStep(bool),
    /// Enter in the find input: next, or previous with Shift.
    FindSubmit,
    FindClose,
    /// Open the palette: quick open, or go to line (`true`, prefilled with `:`).
    PaletteOpen(bool),
    /// Whether the terminal had the keyboard when the palette opened.
    PaletteFrom(bool),
    /// The folder's files for palette `.0`, and whether the list was capped.
    PaletteFiles(u64, (Vec<String>, bool)),
    PaletteQuery(String),
    /// Move the selected result by this many rows (wrapping).
    PaletteMove(isize),
    /// Open result `.0` (a click).
    PalettePick(usize),
    PaletteSubmit,
    PaletteClose,
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

impl App {
    pub fn new(start: Start, fresh: bool) -> (Self, Task<Msg>) {
        let herdr_session = persist::config_get("herdr_session").filter(|s| herdr::valid_session(s));
        let shell_chosen = persist::config_get("terminal").as_deref() == Some("shell");
        let herdr_missing = !shell_chosen && !herdr::installed();
        let use_herdr = !shell_chosen && !herdr_missing;
        let herdr_client = if use_herdr { herdr::Client::for_session(herdr_session.as_deref()) } else { None };
        // Without a folder, show herdr's focused workspace; if herdr isn't up
        // yet, the last folder until it is (see `adopt_focused`).
        let (root, workspace, adopt_focused) = match start {
            Start::Folder(root) => (root, None, false),
            Start::Follow => match herdr_client.as_ref().and_then(|c| c.panes().ok()).and_then(|p| focused_root(&p)) {
                Some((ws, root)) => (root, Some(ws), false),
                None => {
                    let fallback = persist::last_root()
                        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
                        .unwrap_or_else(|| PathBuf::from("/"));
                    (fallback, None, herdr_client.is_some())
                }
            },
        };
        let _ = persist::set_last_root(&root);

        let theme_choice =
            persist::config_get("theme").and_then(|v| theme::Choice::parse(&v)).unwrap_or(theme::Choice::Auto);
        theme::set_current(theme_choice.resolve());
        let width = |key: &str, default: f32| {
            persist::config_get(key).and_then(|v| v.parse::<f32>().ok()).filter(|w| w.is_finite()).unwrap_or(default)
        };
        let runs = if fresh { Vec::new() } else { persist::load(&root).map(|s| s.runs).unwrap_or_default() };

        let mut app = Self {
            inspector_width: width("inspector_width", 400.0),
            window_width: 1400.0,
            window_height: 900.0,
            history_height: width("history_height", 180.0).max(MIN_HISTORY),
            drag_anchor: None,
            ticks: 0,
            status_ticks: 0,
            dragging: None,
            hovered_divider: None,
            theme: theme_choice,
            explorer: Explorer::new(root.clone()),
            dialog: None,
            gear_menu: false,
            drafts: Drafts::default(),
            settings_error: None,
            herdr_version: None,
            window_controls: WindowControls::from_config(),
            explorer_target: None,
            explorer_menu: None,
            name_edit: None,
            undo_trash: None,
            root,
            center: pane_grid::State::new(CenterPane::Terminal).0,
            doc_ratio: 0.55,
            show_inspector: true,
            focus_mode: false,
            inspector: Inspector::Changes,
            window_focused: true,
            autosave: AutoSave::from_config(),
            term: None,
            term_kind: if use_herdr { TermKind::Herdr } else { TermKind::Shell },
            next_term_id: 0,
            herdr: herdr_client,
            herdr_session,
            herdr_missing,
            polling: false,
            workspace,
            adopt_focused,
            root_epoch: 0,
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
            confirm_replace: false,
            confirm_signout: false,
            confirm_discard_all: false,
            undo_discard: None,
            hovered_change: None,
            status_action: None,
            comment: String::new(),
            state_dirty: false,
            tabs: Vec::new(),
            editor_view: None,
            active_tab: None,
            pending_close: None,
            git: Snapshot::default(),
            repo_prefix: String::new(),
            refreshing: false,
            git_gen: 0,
            git_cache: Default::default(),
            git_with_ignored: false,
            git_loaded: false,
            git_busy: false,
            busy_op: None,
            commit_msg: Content::new(),
            commit_history: crate::history::History::default(),
            commit_gen: None,
            commit_gen_seq: 0,
            signin: None,
            signin_seq: 0,
            spin: 0,
            chatgpt_account: crate::openai::account(),
            search: SearchState::default(),
            palette: None,
            palette_seq: 0,
            modifiers: keyboard::Modifiers::default(),
            status: None,
            status_detail: None,
            status_at: Instant::now(),
            status_ttl: Duration::ZERO,
        };
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
        // Spinners: generating a message, the sign-in dialog waiting, a commit, push or pull.
        let waiting = matches!(&self.signin, Some(s) if s.stage != SignInStage::Ask);
        if self.commit_gen.is_some() || waiting || self.busy_op.is_some() {
            subs.push(iced::time::every(Duration::from_millis(80)).map(|_| Msg::Spin));
        }
        // Autosave timer, only while some tab has unsaved edits.
        if matches!(self.autosave, AutoSave::AfterDelay(_))
            && self.tabs.iter().any(|t| matches!(t, Tab::File(f) if f.dirty && !f.conflict))
        {
            subs.push(iced::time::every(Duration::from_millis(250)).map(|_| Msg::AutoSaveTick));
        }
        subs.push(iced::event::listen_with(|event, status, _| match event {
            iced::Event::Keyboard(k @ keyboard::Event::KeyPressed { .. }) => {
                Some(Msg::Key(k, status == iced::event::Status::Captured))
            }
            iced::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => Some(Msg::Modifiers(m)),
            iced::Event::Window(iced::window::Event::Focused) => Some(Msg::WindowFocus(true)),
            iced::Event::Window(iced::window::Event::Unfocused) => Some(Msg::WindowFocus(false)),
            iced::Event::Window(iced::window::Event::CloseRequested) => Some(Msg::CloseRequested),
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

    /// The Files inspector is on screen (the tree and ignored files are only needed then).
    pub(super) fn files_visible(&self) -> bool {
        self.show_inspector && !self.focus_mode && self.inspector == Inspector::Files
    }

    /// The folder is in a git repository, as of the last snapshot (never runs git).
    pub fn has_repo(&self) -> bool {
        self.git.root.is_some()
    }

    /// Files in Changes that an agent changed and you haven't opened since.
    /// (Files of older turns that were committed since don't count.)
    pub fn fresh_changes(&self) -> usize {
        let unstaged = self.git.unstaged.iter().filter(|c| self.runs.is_new(&c.path)).count();
        let staged_only = self
            .git
            .staged
            .iter()
            .filter(|c| self.runs.is_new(&c.path) && !self.git.unstaged.iter().any(|u| u.path == c.path))
            .count();
        unstaged + staged_only
    }

    /// Show `msg` in the status bar: 6 s, errors 30 s (or until clicked or Escape).
    fn set_status(&mut self, msg: impl Into<String>, error: bool) {
        self.show_status(msg.into(), error, if error { 30 } else { 6 });
    }

    /// A two-step action waits for its second click: shown briefly, and the
    /// pending confirmation goes with it (see `StatusTick`).
    fn set_confirm(&mut self, msg: &str) {
        self.show_status(msg.to_string(), true, 10);
    }

    /// An error shortened to `summary`; the whole text goes in the tooltip.
    fn set_error_detail(&mut self, summary: String, detail: String) {
        self.set_status(summary, true);
        self.status_detail = (detail.trim() != self.status.as_ref().map_or("", |(s, _)| s.as_str())).then_some(detail);
    }

    /// Show `msg` with a button that sends `action`, for 20 s.
    fn set_status_action(&mut self, msg: impl Into<String>, label: &'static str, action: Msg) {
        self.show_status(msg.into(), false, 20);
        self.status_action = Some((label, action));
    }

    fn show_status(&mut self, msg: String, error: bool, secs: u64) {
        self.status = Some((msg, error));
        self.status_detail = None;
        self.status_action = None;
        self.status_at = Instant::now();
        self.status_ttl = Duration::from_secs(secs);
    }

    pub(super) fn clear_status(&mut self) {
        self.status = None;
        self.status_detail = None;
        self.status_action = None;
        self.clear_confirms();
    }

    /// Drop every pending two-step confirmation (revert, replace, sign out, discard tab).
    /// Returns whether one was pending.
    pub(super) fn clear_confirms(&mut self) -> bool {
        let any = self.confirm_revert.is_some()
            || self.confirm_replace
            || self.confirm_signout
            || self.confirm_discard_all
            || self.pending_close.is_some();
        self.confirm_revert = None;
        self.confirm_replace = false;
        self.confirm_signout = false;
        self.confirm_discard_all = false;
        self.pending_close = None;
        any
    }

    /// The status expired.
    pub(super) fn status_expired(&self) -> bool {
        self.status.is_some() && self.status_at.elapsed() > self.status_ttl
    }

    // ------------------------------------------------------------------ update

    /// Hand each message to the module that owns it. No wildcard: a new `Msg`
    /// variant must be routed here.
    pub fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::CloseRequested => {
                if self.autosave != AutoSave::Off {
                    self.save_dirty_tabs();
                }
                self.save_state();
                iced::exit()
            }
            Msg::Tick => self.on_tick(),
            Msg::Key(event, captured) => self.on_key(event, captured),
            m @ (Msg::Inspector(_)
            | Msg::InspectorTab(_)
            | Msg::ToggleInspector
            | Msg::FocusMode
            | Msg::CenterResized(_)
            | Msg::WindowFocus(_)
            | Msg::DividerHover(_)
            | Msg::DividerPress(_)
            | Msg::DividerDrag(_)
            | Msg::DividerRelease
            | Msg::WindowSize(_)
            | Msg::DismissStatus
            | Msg::CycleTheme) => self.update_layout(m),
            m @ (Msg::Term(_) | Msg::TermAction(_) | Msg::ReportColorScheme | Msg::RestartTerminal) => {
                self.update_terminal(m)
            }
            m @ (Msg::StatusTick
            | Msg::HerdrPolled(_)
            | Msg::ForRoot(..)
            | Msg::WorkspaceReady(_)
            | Msg::SendToAgent(_)
            | Msg::FocusAgent(_)
            | Msg::AgentFocused(_)
            | Msg::SentToPane(_)) => self.update_follow(m),
            m @ (Msg::RunSnapshot(..)
            | Msg::RunFiles(..)
            | Msg::RunPrompt(..)
            | Msg::SelectRun(_)
            | Msg::MarkAllSeen
            | Msg::GroupByRun(_)
            | Msg::OpenRunDiff(..)
            | Msg::ReviewStep(_)
            | Msg::RevertRun(_)
            | Msg::StageRun(_)
            | Msg::Comment(_)
            | Msg::SendComment) => self.update_review(m),
            m @ (Msg::OpenFile(..)
            | Msg::OpenImage(_)
            | Msg::SelectTab(_)
            | Msg::CloseTab(_)
            | Msg::Edit(_)
            | Msg::EditorViewport(_)
            | Msg::Save
            | Msg::Undo
            | Msg::Redo
            | Msg::AutoSaveTick
            | Msg::DiffLoaded(..)
            | Msg::DiffFull(_)
            | Msg::DiffAnchor(_)
            | Msg::DiffOpenFile
            | Msg::SendLine
            | Msg::ExplorerClick(..)) => self.update_documents(m),
            m @ (Msg::GearMenu(_)
            | Msg::Gear(_)
            | Msg::OpenDialog(_)
            | Msg::CloseDialog
            | Msg::HerdrVersion(_)
            | Msg::SettingChoice(_)
            | Msg::SettingText(..)
            | Msg::SettingSubmit
            | Msg::OpenConfigFile
            | Msg::OpenLink(_)
            | Msg::CopyAbout
            | Msg::WindowMinimize
            | Msg::WindowMaximize
            | Msg::WindowDrag) => self.update_settings(m),
            m @ (Msg::ExplorerTarget(..)
            | Msg::ExplorerMenuAt(_)
            | Msg::ExplorerMenuClose
            | Msg::ExplorerMenu(_)
            | Msg::ExplorerName(_)
            | Msg::ExplorerNameSubmit
            | Msg::ExplorerNameCancel
            | Msg::TrashDone(_)
            | Msg::UndoTrash) => self.update_files(m),
            m @ (Msg::GitLoaded(..)
            | Msg::OpenDiff(..)
            | Msg::Stage(_)
            | Msg::Unstage(_)
            | Msg::Discard(_)
            | Msg::DiscardAll
            | Msg::UndoDiscard
            | Msg::Hunk(..)
            | Msg::OpenChanges(_)
            | Msg::OpenChangedFile(_)
            | Msg::HoverChange(..)
            | Msg::CommitEdit(_)
            | Msg::CommitUndo
            | Msg::CommitRedo
            | Msg::Commit
            | Msg::GitInit
            | Msg::GenerateCommit
            | Msg::CommitGen(..)
            | Msg::Spin
            | Msg::ChatGptSignOut
            | Msg::SignInStart
            | Msg::SignInCancel
            | Msg::SignInCopyLink
            | Msg::SignInEvent(..)
            | Msg::Push
            | Msg::Pull
            | Msg::GitDone(..)) => self.update_scm(m),
            m @ (Msg::SearchQuery(_)
            | Msg::SearchReplace(_)
            | Msg::SearchToggle(_)
            | Msg::SearchRun
            | Msg::SearchDebounced(_)
            | Msg::SearchDone(..)
            | Msg::SearchOpen(..)
            | Msg::SearchSend(..)
            | Msg::ReplaceAll
            | Msg::ReplaceDone(..)) => self.update_find(m),
            m @ (Msg::Modifiers(_)
            | Msg::FindOpen
            | Msg::FindQuery(_)
            | Msg::FindToggleCase
            | Msg::FindStep(_)
            | Msg::FindSubmit
            | Msg::FindClose
            | Msg::PaletteOpen(_)
            | Msg::PaletteFrom(_)
            | Msg::PaletteFiles(..)
            | Msg::PaletteQuery(_)
            | Msg::PaletteMove(_)
            | Msg::PalettePick(_)
            | Msg::PaletteSubmit
            | Msg::PaletteClose) => self.update_navigate(m),
        }
    }

    fn on_tick(&mut self) -> Task<Msg> {
        self.ticks = self.ticks.wrapping_add(1);
        if self.theme == theme::Choice::Auto {
            self.apply_theme();
        }
        // In the background, refresh every 10 s instead of every 2 s.
        if !self.window_focused && !self.ticks.is_multiple_of(5) {
            return Task::none();
        }
        // The tree is only on screen in Files.
        if self.files_visible() {
            self.explorer.refresh();
        }
        self.reload_changed_files();
        let diff = if self.active_diff_stale() { self.reload_active_diff() } else { Task::none() };
        Task::batch([self.refresh_git(), diff])
    }
}

pub fn first_line(s: &str) -> Option<&str> {
    s.lines().map(str::trim).find(|l| !l.is_empty())
}

/// What mira opens at start.
#[derive(Debug, Clone)]
pub enum Start {
    /// This folder (its herdr workspace gets focused, or created).
    Folder(PathBuf),
    /// The folder of herdr's focused workspace.
    Follow,
}

/// herdr's focused workspace moved to another one.
#[derive(Debug, Clone)]
pub struct Focus {
    workspace: String,
    root: PathBuf,
    /// The workspace mira knew when it polled; a stale reply is ignored.
    since: Option<String>,
}
