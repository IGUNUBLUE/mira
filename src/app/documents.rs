//! Document tabs: files (edit, undo, save, autosave) and diffs, plus the explorer click.

use super::{App, Msg, blocking, file_name};
use crate::diff::{self, FileDiff};
use crate::git;
use crate::imagefile;
use crate::persist;
use crate::runs::RunId;

use iced::Task;
use iced::widget::text_editor::{self, Content};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// Largest file opened in a tab or diffed (bytes).
const MAX_OPEN_BYTES: u64 = 8 * 1024 * 1024;

/// The file editor's padding (view.rs) above and below its text.
pub const EDITOR_PADDING_TOP: f32 = 6.0;
const EDITOR_PADDING_Y: f32 = 2.0 * EDITOR_PADDING_TOP;

/// `autosave` in the config, named like VSCodium's `files.autoSave`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoSave {
    Off,
    /// Save once edits pause for `autosave_delay` ms (default 1000).
    AfterDelay(Duration),
    /// Save when switching tabs, closing a tab or leaving the window.
    OnFocusChange,
    /// Save when leaving the window.
    OnWindowChange,
}

impl AutoSave {
    fn parse(mode: Option<&str>, delay_ms: Option<&str>) -> Self {
        let delay = Duration::from_millis(delay_ms.and_then(|d| d.trim().parse().ok()).unwrap_or(1000).max(100));
        match mode.map(|m| m.trim().to_ascii_lowercase()).as_deref() {
            Some("off") => Self::Off,
            Some("onfocuschange") => Self::OnFocusChange,
            Some("onwindowchange") => Self::OnWindowChange,
            _ => Self::AfterDelay(delay),
        }
    }

    pub(super) fn from_config() -> Self {
        Self::parse(persist::config_get("autosave").as_deref(), persist::config_get("autosave_delay").as_deref())
    }
}

pub struct FileTab {
    pub path: PathBuf,
    /// `path` inside the opened folder as `a › b › c`, for the header.
    pub crumbs: String,
    pub content: Content,
    pub dirty: bool,
    pub mtime: Option<SystemTime>,
    pub syntax: String,
    pub history: crate::history::History,
    /// Last edit, for autosave after a delay.
    edited_at: Option<Instant>,
    /// The file changed on disk while this tab had unsaved edits: autosave
    /// stops for it until the user saves by hand.
    pub(super) conflict: bool,
    /// The last write failed (disk full, permissions): autosave stops
    /// retrying it; Ctrl+S tries again right away.
    pub(super) write_failed: bool,
    /// The editor's scroll, mirrored for the line numbers (see `gutter.rs`).
    pub scroll: crate::gutter::Scroll,
    /// Opened at a line and not touched since: keep that line in view while
    /// the editor's size settles (the panes are laid out after it opens).
    pub(super) reveal: bool,
    /// The find bar (Ctrl+F), while open; its matches follow the text.
    pub find: Option<Box<super::FindBar>>,
}

impl FileTab {
    /// Height of one editor row.
    pub fn row() -> f32 {
        crate::gutter::line_height(super::CODE_SIZE as f32)
    }

    /// After an editor action: the view follows the cursor, as the editor does.
    pub(super) fn follow(&mut self, view: Option<f32>) {
        self.scroll.settle(self.content.cursor().position, Self::row(), self.content.line_count(), view);
    }

    /// Scroll the cursor's line into view. The editor only scrolls when the
    /// cursor moves, and the size it first scrolled for may change right after
    /// a file opens: so move it off its line and back, then follow.
    fn reveal(&mut self, view: Option<f32>) {
        let cursor = self.content.cursor();
        let line = cursor.position.line;
        let other = if line + 1 < self.content.line_count() { line + 1 } else { line.saturating_sub(1) };
        if other != line {
            let away = text_editor::Position { line: other, column: 0 };
            self.content.move_to(text_editor::Cursor { position: away, selection: None });
            self.content.move_to(cursor);
        }
        self.scroll.force_follow();
        self.follow(view);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffSource {
    /// `staged`: HEAD ⇄ index, otherwise index ⇄ working tree.
    Index { staged: bool },
    /// Between two snapshot trees (an agent run).
    Trees { from: String, to: String, label: String, run: RunId },
    /// Immutable commit and first-parent comparison, independent of local edits.
    Commit { details: Arc<git::commits::Details> },
}

pub struct DiffTab {
    /// Repo-relative path.
    pub rel: String,
    pub source: DiffSource,
    pub full: bool,
    /// Shared with the view's cached rows (see `view_diff`).
    pub diff: Option<Arc<FileDiff>>,
    /// Line clicked in the diff (new-side number preferred), for comments.
    pub anchor: Option<usize>,
    /// Working file's mtime when the diff was last computed (index diffs).
    pub loaded_mtime: Option<SystemTime>,
    /// The file exists in the working tree (checked when the diff loads), for "Open file".
    pub on_disk: bool,
    /// The latest load asked for: an older one finishing later is dropped.
    pub load: u64,
}

/// An image, shown with zoom and pan (raster) or scaled to fit (SVG).
pub struct ImageTab {
    pub path: PathBuf,
    pub crumbs: String,
    pub image: ImageData,
    /// Pixel size (raster only).
    pub size: Option<(u32, u32)>,
    pub bytes: u64,
    mtime: Option<SystemTime>,
    /// The file was deleted on disk; the last image stays.
    pub gone: bool,
}

/// What iced draws. A new handle (new id) per read, so a changed file redraws.
pub enum ImageData {
    Raster(iced::widget::image::Handle),
    Svg(iced::widget::svg::Handle),
}

impl ImageTab {
    fn load(root: &Path, path: PathBuf, kind: imagefile::Kind) -> Result<Self, String> {
        let loaded = imagefile::load(&path, kind)?;
        let bytes = loaded.bytes.len() as u64;
        let image = match kind {
            imagefile::Kind::Raster => ImageData::Raster(iced::widget::image::Handle::from_bytes(loaded.bytes)),
            imagefile::Kind::Svg => ImageData::Svg(iced::widget::svg::Handle::from_memory(loaded.bytes)),
        };
        Ok(Self {
            crumbs: crumbs(root, &path),
            mtime: mtime(&path),
            path,
            image,
            size: loaded.size,
            bytes,
            gone: false,
        })
    }
}

pub enum Tab {
    File(FileTab),
    Diff(DiffTab),
    Image(ImageTab),
}

impl Tab {
    /// The file a file or image tab shows.
    pub fn file_path(&self) -> Option<&Path> {
        match self {
            Tab::File(t) => Some(&t.path),
            Tab::Image(t) => Some(&t.path),
            Tab::Diff(_) => None,
        }
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// `path` inside `root` as `a › b › c` (the whole path when outside).
pub(super) fn crumbs(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('/', " › ")
}

fn read_text(path: &Path) -> Result<String, String> {
    crate::files::read_text(path, MAX_OPEN_BYTES)
}

fn load_diff(root: &Path, rel: &str, source: &DiffSource, full: bool) -> FileDiff {
    // Sizes first: a huge log or dump is never read into memory.
    let blob = |rev: &str| git::blob_size(root, rev, rel).is_some_and(|n| n > MAX_OPEN_BYTES);
    let too_large = match source {
        DiffSource::Index { staged: true } => blob("HEAD") || blob(""),
        DiffSource::Index { staged: false } => {
            blob("") || std::fs::metadata(root.join(rel)).is_ok_and(|m| m.len() > MAX_OPEN_BYTES)
        }
        DiffSource::Trees { from, to, .. } => blob(from) || blob(to),
        DiffSource::Commit { .. } => false,
    };
    if too_large {
        return FileDiff { too_large: true, id: diff::next_id(), ..Default::default() };
    }
    let (old, new) = match source {
        DiffSource::Index { staged: true } => (git::show(root, "HEAD", rel), git::show(root, "", rel)),
        DiffSource::Index { staged: false } => {
            let disk =
                std::fs::read(root.join(rel)).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            let index = git::show(root, "", rel);
            // With CRLF checkouts (autocrlf, `eol=crlf`) git stores LF: compared as
            // git does, or every line would show as changed.
            let disk = if disk.contains("\r\n") && !index.contains('\r') && git::checks_out_crlf(root, rel) {
                disk.replace("\r\n", "\n")
            } else {
                disk
            };
            (index, disk)
        }
        DiffSource::Trees { from, to, .. } => (git::show(root, from, rel), git::show(root, to, rel)),
        DiffSource::Commit { details } => match git::commits::contents(root, details, rel, MAX_OPEN_BYTES) {
            Ok(Some(contents)) => contents,
            Ok(None) => return FileDiff { too_large: true, id: diff::next_id(), ..Default::default() },
            Err(error) => return FileDiff { error: Some(error), id: diff::next_id(), ..Default::default() },
        },
    };
    let mut d = diff::compute(&old, &new, if full { None } else { Some(3) });
    if let DiffSource::Index { staged } = *source
        && !d.binary
        && !d.items.is_empty()
    {
        d.patch = git::file_patch(root, rel, staged).ok().and_then(|p| crate::hunks::parse(&p, staged));
    }
    d
}

impl App {
    pub(super) fn update_documents(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::OpenFile(path, line) => self.open_file(path, line),
            Msg::OpenImage(path) => self.open_image(path),
            Msg::SelectTab(i) => {
                if i < self.tabs.len() {
                    self.activate(i);
                    return self.reload_active_diff();
                }
            }
            Msg::CloseTab(i) => self.close_tab(i),
            Msg::Edit(action) => {
                let edit = action.is_edit();
                let view = self.editor_view;
                if let Some(t) = self.active_file_mut() {
                    let wheel = match action {
                        text_editor::Action::Scroll { lines } => Some(lines),
                        _ => None,
                    };
                    t.reveal = false;
                    t.history.perform(&mut t.content, action);
                    match wheel {
                        Some(lines) => t.scroll.scrolled(lines, FileTab::row(), t.content.line_count(), view),
                        None => t.follow(view),
                    }
                    if edit {
                        t.dirty = true;
                        t.edited_at = Some(Instant::now());
                        t.refind();
                    } else {
                        t.sync_find();
                    }
                }
            }
            Msg::Save => {
                if let Some(i) = self.active_tab {
                    self.save_tab(i, true);
                }
            }
            Msg::SaveFile(path) => {
                if let Some(i) = self.tabs.iter().position(|t| matches!(t, Tab::File(f) if f.path == path)) {
                    self.save_tab(i, true);
                }
            }
            Msg::EditorViewport(size) => {
                // The text area: the editor minus its vertical padding.
                self.editor_view = Some((size.height - EDITOR_PADDING_Y).max(0.0));
                let view = self.editor_view;
                if let Some(t) = self.active_file_mut() {
                    // Until the user acts, keep the line a file was opened at in view.
                    if t.reveal {
                        t.reveal(view);
                    } else {
                        t.follow(view);
                    }
                }
            }
            Msg::Undo | Msg::Redo => {
                let view = self.editor_view;
                if let Some(t) = self.active_file_mut() {
                    let done = if matches!(msg, Msg::Undo) {
                        t.history.undo(&mut t.content)
                    } else {
                        t.history.redo(&mut t.content)
                    };
                    if done {
                        t.follow(view);
                        t.dirty = t.history.is_dirty();
                        t.edited_at = Some(Instant::now());
                        t.refind();
                    }
                }
            }
            Msg::AutoSaveTick if self.confirmation.is_some() => {}
            Msg::AutoSaveTick => {
                if let AutoSave::AfterDelay(delay) = self.autosave {
                    for i in 0..self.tabs.len() {
                        if matches!(&self.tabs[i], Tab::File(t) if t.dirty && t.edited_at.is_none_or(|e| e.elapsed() >= delay))
                        {
                            self.save_tab(i, false);
                        }
                    }
                }
            }
            Msg::DiffLoaded(load, on_disk, diff) => {
                if let Some(d) = self.tabs.iter_mut().find_map(|t| match t {
                    Tab::Diff(d) if d.load == load => Some(d),
                    _ => None,
                }) {
                    d.diff = Some(diff);
                    d.on_disk = on_disk;
                }
            }
            Msg::DiffFull(full) => {
                if let Some(Tab::Diff(d)) = self.active_tab.and_then(|i| self.tabs.get_mut(i)) {
                    d.full = full;
                    return self.reload_active_diff();
                }
            }
            Msg::DiffOpenFile => {
                if let Some(d) = self.active_diff() {
                    let (path, line) = (self.repo_root().join(&d.rel), d.anchor.map(|l| l.saturating_sub(1)));
                    self.open_file(path, line);
                }
            }
            Msg::SendLine => {
                if let Some(Tab::File(t)) = self.active_tab.and_then(|i| self.tabs.get(i)) {
                    let rel = t.path.strip_prefix(&self.root).unwrap_or(&t.path).to_string_lossy();
                    let text = format!("{rel}:{} ", t.content.cursor().position.line + 1);
                    return self.send_to_agent(text, None, false);
                }
            }
            Msg::DiffAnchor(line) => {
                if let Some(Tab::Diff(d)) = self.active_tab.and_then(|i| self.tabs.get_mut(i)) {
                    d.anchor = if d.anchor == Some(line) { None } else { Some(line) };
                }
            }
            Msg::ExplorerClick(path, is_dir) => {
                // Clicking elsewhere leaves a name being typed, as Escape does.
                self.name_edit = None;
                if is_dir {
                    self.explorer.toggle(&path);
                } else {
                    self.open_file(path, None);
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// The active tab, if it is a diff.
    pub(crate) fn active_diff(&self) -> Option<&DiffTab> {
        match self.active_tab.and_then(|i| self.tabs.get(i)) {
            Some(Tab::Diff(d)) => Some(d),
            _ => None,
        }
    }

    /// The active tab, if it is a file.
    pub(super) fn active_file_mut(&mut self) -> Option<&mut FileTab> {
        match self.active_tab.and_then(|i| self.tabs.get_mut(i)) {
            Some(Tab::File(t)) => Some(t),
            _ => None,
        }
    }

    fn open_tab(&mut self, tab: Tab) -> usize {
        self.tabs.push(tab);
        self.tabs.len() - 1
    }

    /// Make tab `i` the active one. Every switch goes through here, so
    /// `OnFocusChange` saves the tab being left however it's left.
    pub(super) fn activate(&mut self, i: usize) {
        if self.autosave == AutoSave::OnFocusChange
            && let Some(prev) = self.active_tab.filter(|p| *p != i)
        {
            self.save_tab(prev, false);
        }
        self.active_tab = Some(i);
    }

    pub(super) fn open_file(&mut self, path: PathBuf, line: Option<usize>) {
        if imagefile::kind(&path) == Some(imagefile::Kind::Raster) {
            self.open_image(path);
            return;
        }
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
                        crumbs: crumbs(&self.root, &path),
                        content: Content::with_text(&text),
                        path,
                        dirty: false,
                        syntax,
                        history: crate::history::History::default(),
                        edited_at: None,
                        conflict: false,
                        write_failed: false,
                        scroll: crate::gutter::Scroll::default(),
                        reveal: false,
                        find: None,
                    }))
                }
                Err(e) => {
                    self.set_status(format!("{}: {e}", file_name(&path)), true);
                    return;
                }
            },
        };
        let view = self.editor_view;
        if let (Some(line), Tab::File(t)) = (line, &mut self.tabs[idx]) {
            t.go_to_line(line, view);
            t.reveal = true;
        }
        self.activate(idx);
        self.focus_mode = false;
        self.sync_center();
    }

    /// Open `path` in an image tab (a raster image, or an SVG's preview).
    pub(super) fn open_image(&mut self, path: PathBuf) {
        let Some(kind) = imagefile::kind(&path) else { return };
        let idx = match self.tabs.iter().position(|t| matches!(t, Tab::Image(f) if f.path == path)) {
            Some(i) => i,
            None => match ImageTab::load(&self.root, path.clone(), kind) {
                Ok(tab) => self.open_tab(Tab::Image(tab)),
                Err(e) => {
                    self.set_status(format!("{}: {e}", file_name(&path)), true);
                    return;
                }
            },
        };
        self.activate(idx);
        self.focus_mode = false;
        self.sync_center();
    }

    pub(super) fn open_diff(&mut self, rel: String, source: DiffSource) -> Task<Msg> {
        let idx = self.tabs.iter().position(|t| matches!(t, Tab::Diff(d) if d.rel == rel && d.source == source));
        let tab = |rel, source| {
            Tab::Diff(DiffTab {
                rel,
                source,
                full: false,
                diff: None,
                anchor: None,
                loaded_mtime: None,
                on_disk: false,
                load: 0,
            })
        };
        let idx = match idx {
            Some(i) => i,
            // Review diffs share one preview tab instead of piling up.
            None if matches!(source, DiffSource::Trees { .. } | DiffSource::Commit { .. }) => {
                match self
                    .tabs
                    .iter()
                    .position(|t| matches!(t, Tab::Diff(d) if matches!(d.source, DiffSource::Trees { .. } | DiffSource::Commit { .. })))
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
        self.activate(idx);
        self.focus_mode = false;
        self.sync_center();
        self.reload_active_diff()
    }

    pub(super) fn reload_active_diff(&mut self) -> Task<Msg> {
        let root = self.repo_root();
        let Some(Tab::Diff(d)) = self.active_tab.and_then(|i| self.tabs.get_mut(i)) else {
            return Task::none();
        };
        d.loaded_mtime = mtime(&root.join(&d.rel));
        d.load = diff::next_id();
        let (rel, source, full, load) = (d.rel.clone(), d.source.clone(), d.full, d.load);
        let task = Task::perform(
            blocking(move || {
                let diff = Arc::new(load_diff(&root, &rel, &source, full));
                (root.join(&rel).is_file(), diff)
            }),
            move |(on_disk, diff)| Msg::DiffLoaded(load, on_disk, diff),
        );
        // A reply for another folder's repo is dropped too.
        self.for_root(task)
    }

    /// The active diff compares against the working tree and the file changed
    /// since it was computed. Run diffs (between snapshots) never go stale.
    pub(super) fn active_diff_stale(&self) -> bool {
        let root = self.repo_root();
        self.active_diff().is_some_and(|d| {
            matches!(d.source, DiffSource::Index { .. }) && mtime(&root.join(&d.rel)) != d.loaded_mtime
        })
    }

    pub(super) fn active_diff_is_index(&self) -> bool {
        self.active_diff().is_some_and(|d| matches!(d.source, DiffSource::Index { .. }))
    }

    pub(super) fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        if matches!(&self.tabs[i], Tab::File(t) if t.dirty) && self.autosave != AutoSave::Off {
            self.save_tab(i, false);
        }
        if let Tab::File(t) = &self.tabs[i]
            && t.dirty
            && !self.confirm_action(super::ConfirmAction::CloseTab(t.path.clone()), "Close without saving?",
                "The unsaved edits in this editor tab will be lost. The file on disk will not be changed. You can cancel and save it first.",
                vec![t.path.to_string_lossy().into_owned()], "Close without saving")
        {
            return;
        }
        self.tabs.remove(i);
        self.active_tab = active_after_remove(self.active_tab, i, self.tabs.len());
        self.sync_center();
    }

    /// Save tab `i`. Autosave (`manual == false`) never overwrites a file that
    /// changed on disk since it was loaded (an agent may have written it): it
    /// pauses for that tab and says so. A manual save replaces it only after
    /// explicit confirmation (see [`manual_save`]).
    pub(super) fn save_tab(&mut self, i: usize, manual: bool) -> bool {
        let Some(Tab::File(t)) = self.tabs.get_mut(i) else { return false };
        if manual {
            let changed = mtime(&t.path) != t.mtime;
            let action = super::ConfirmAction::Overwrite(t.path.clone(), mtime(&t.path));
            let confirmed = self.approval.as_ref() == Some(&action);
            match manual_save(t.dirty, t.conflict, changed, confirmed) {
                SaveCheck::Skip => return false,
                SaveCheck::Write => {}
                SaveCheck::Confirm => {
                    let name = file_name(&t.path);
                    let what = if mtime(&t.path).is_none() { "was deleted on disk" } else { "changed on disk" };
                    let path = t.path.to_string_lossy().into_owned();
                    self.confirm_action(action, "Overwrite the file on disk?",
                        format!("{name} {what}, possibly by an agent. Saving will replace the current disk contents with the editor's version. The overwritten disk contents have no Undo in the editor. Cancel to review them first."),
                        vec![path], "Overwrite file");
                    return false;
                }
            }
        } else {
            if !t.dirty || t.conflict || t.write_failed {
                return false;
            }
            if mtime(&t.path) != t.mtime {
                t.conflict = true;
                let name = file_name(&t.path);
                self.set_status(
                    format!("{name} changed on disk — autosave paused for it; Ctrl+S asks before replacing it"),
                    true,
                );
                return false;
            }
        }
        match crate::files::rewrite(&t.path, t.content.text().as_bytes()) {
            Ok(()) => {
                t.history.mark_saved(&t.content);
                t.dirty = false;
                t.conflict = false;
                t.write_failed = false;
                t.edited_at = None;
                t.mtime = mtime(&t.path);
                let name = file_name(&t.path);
                if manual {
                    self.set_status(format!("Saved {name}"), false);
                }
                true
            }
            Err(e) => {
                // Stop retrying every tick; a manual save tries again. What a
                // half-done write left on disk is ours, not a change to ask about.
                t.write_failed = true;
                t.mtime = mtime(&t.path);
                let name = file_name(&t.path);
                self.set_status(format!("Save {name} failed: {e} — autosave paused for it; Ctrl+S tries again"), true);
                false
            }
        }
    }

    pub(super) fn save_dirty_tabs(&mut self) {
        for i in 0..self.tabs.len() {
            self.save_tab(i, false);
        }
    }

    pub(super) fn reload_changed_files(&mut self) {
        let view = self.editor_view;
        let mut deleted = Vec::new();
        let mut unreadable = Vec::new();
        for tab in &mut self.tabs {
            if let Tab::Image(t) = tab {
                // An agent re-rendered a screenshot or an icon: show the new one.
                let now = mtime(&t.path);
                if now != t.mtime {
                    t.mtime = now;
                    t.gone = now.is_none();
                    if let (false, Some(kind)) = (t.gone, imagefile::kind(&t.path))
                        && let Ok(fresh) = ImageTab::load(&self.root, t.path.clone(), kind)
                    {
                        *t = fresh;
                    }
                }
                continue;
            }
            let Tab::File(t) = tab else { continue };
            let now = mtime(&t.path);
            if t.dirty || now == t.mtime {
                continue;
            }
            t.mtime = now;
            if now.is_none() {
                // Deleted (e.g. by an agent): keep the text, but autosave must
                // not write the file back. Ctrl+S still does.
                t.conflict = true;
                deleted.push(file_name(&t.path));
                continue;
            }
            let text = match read_text(&t.path) {
                Ok(text) => text,
                // Too large, binary or unreadable now: keep the text, and keep autosave
                // from writing it over the new file (that would undo the agent's change).
                Err(e) => {
                    t.conflict = true;
                    unreadable.push(format!("{}: {e}", file_name(&t.path)));
                    continue;
                }
            };
            {
                t.conflict = false;
                let old = t.content.text();
                let before = t.content.cursor();
                t.content = Content::with_text(&text);
                // A new editor starts at the top, then follows its cursor.
                t.scroll = crate::gutter::Scroll::default();
                let line = before.position.line.min(t.content.line_count().saturating_sub(1));
                t.content.move_to(text_editor::Cursor {
                    position: text_editor::Position { line, column: 0 },
                    selection: None,
                });
                t.follow(view);
                // Undoable, like VSCodium: Ctrl+Z brings back the text before the agent's change.
                t.history.record(&old, &t.content.text(), before, t.content.cursor());
                t.history.mark_saved(&t.content);
                t.refind();
            }
        }
        if let Some(first) = unreadable.first() {
            self.set_status(format!("{first} — not reloaded; autosave paused for it"), true);
        }
        if let Some(first) = deleted.first() {
            let more = if deleted.len() > 1 { format!(" (and {} more)", deleted.len() - 1) } else { String::new() };
            self.set_status(
                format!("{first}{more} deleted on disk — autosave paused; Ctrl+S asks before recreating it"),
                true,
            );
        }
    }
}

/// What Ctrl+S does with a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveCheck {
    /// Nothing to save (writing a clean buffer could only undo a newer file).
    Skip,
    Write,
    /// The file changed or vanished since it was loaded: ask before replacing it.
    Confirm,
}

/// `conflict`: autosave paused (the file changed, vanished or can't be read);
/// a clean tab in conflict is one whose file was deleted, which Ctrl+S writes back.
fn manual_save(dirty: bool, conflict: bool, disk_changed: bool, confirmed: bool) -> SaveCheck {
    match (dirty || conflict, conflict || disk_changed) {
        (false, _) => SaveCheck::Skip,
        (true, false) => SaveCheck::Write,
        (true, true) if confirmed => SaveCheck::Write,
        (true, true) => SaveCheck::Confirm,
    }
}

/// The active tab after keeping only the tabs marked in `keep`: the same tab
/// if it stays, else the one that takes its place (or the last one left).
pub(super) fn active_after_retain(active: Option<usize>, keep: &[bool]) -> Option<usize> {
    let left = keep.iter().filter(|k| **k).count();
    let a = active?;
    if left == 0 {
        return None;
    }
    let before = keep.iter().take(a).filter(|k| **k).count();
    Some(before.min(left - 1))
}

/// The active tab after removing tab `removed`; `len` tabs are left.
fn active_after_remove(active: Option<usize>, removed: usize, len: usize) -> Option<usize> {
    match active {
        _ if len == 0 => None,
        Some(a) if a > removed => Some(a - 1),
        Some(a) if a == removed => Some(removed.min(len - 1)),
        other => other,
    }
}

#[cfg(test)]
impl FileTab {
    /// A tab holding `text`, not backed by a file.
    pub(super) fn for_test(text: &str) -> Self {
        Self {
            path: PathBuf::from("test.txt"),
            crumbs: String::new(),
            content: Content::with_text(text),
            dirty: false,
            mtime: None,
            syntax: "txt".into(),
            history: crate::history::History::default(),
            edited_at: None,
            conflict: false,
            write_failed: false,
            scroll: crate::gutter::Scroll::default(),
            reveal: false,
            find: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_s_never_silently_replaces_a_newer_file() {
        use SaveCheck::*;
        assert_eq!(manual_save(false, false, true, false), Skip, "a clean tab writes nothing");
        assert_eq!(manual_save(true, false, false, false), Write);
        assert_eq!(manual_save(true, false, true, false), Confirm, "changed on disk: ask");
        assert_eq!(manual_save(true, true, false, false), Confirm, "a conflict: ask");
        assert_eq!(manual_save(true, true, true, true), Write, "explicitly approved");
        assert_eq!(manual_save(false, true, true, false), Confirm, "deleted on disk: written back after asking");
    }

    #[test]
    fn autosave_setting_follows_vscodium_names() {
        let second = Duration::from_millis(1000);
        assert_eq!(AutoSave::parse(None, None), AutoSave::AfterDelay(second));
        assert_eq!(
            AutoSave::parse(Some("afterDelay"), Some("2500")),
            AutoSave::AfterDelay(Duration::from_millis(2500))
        );
        assert_eq!(AutoSave::parse(Some("afterDelay"), Some("5")), AutoSave::AfterDelay(Duration::from_millis(100)));
        assert_eq!(AutoSave::parse(Some("afterDelay"), Some("soon")), AutoSave::AfterDelay(second));
        assert_eq!(AutoSave::parse(Some("off"), None), AutoSave::Off);
        assert_eq!(AutoSave::parse(Some("onFocusChange"), None), AutoSave::OnFocusChange);
        assert_eq!(AutoSave::parse(Some("onWindowChange"), None), AutoSave::OnWindowChange);
    }

    #[test]
    fn closing_a_tab_keeps_the_active_one_or_its_neighbour() {
        // Closing a tab before the active one shifts it left.
        assert_eq!(active_after_remove(Some(2), 0, 2), Some(1));
        // Closing the active tab selects the one that took its place, or the new last.
        assert_eq!(active_after_remove(Some(1), 1, 2), Some(1));
        assert_eq!(active_after_remove(Some(2), 2, 2), Some(1));
        // A tab after the active one doesn't move it.
        assert_eq!(active_after_remove(Some(0), 1, 1), Some(0));
        assert_eq!(active_after_remove(Some(0), 0, 0), None);
        assert_eq!(active_after_remove(None, 0, 3), None);
    }

    #[test]
    fn switching_folders_keeps_the_active_tab_when_it_stays() {
        // Diff tabs (false) close: the active file tab keeps being shown.
        assert_eq!(active_after_retain(Some(2), &[false, true, true]), Some(1));
        assert_eq!(active_after_retain(Some(0), &[true, false, true]), Some(0));
        // The active diff closes: the next tab left, or the last one.
        assert_eq!(active_after_retain(Some(1), &[true, false, true]), Some(1));
        assert_eq!(active_after_retain(Some(2), &[true, true, false]), Some(1));
        assert_eq!(active_after_retain(Some(0), &[false, false]), None);
        assert_eq!(active_after_retain(None, &[true]), None);
    }

    #[test]
    fn crlf_checkouts_diff_like_git() {
        let dir = crate::files::test_dir("crlfdiff");
        assert!(git::git(&dir, &["init", "-q"]).is_ok());
        std::fs::write(dir.join(".gitattributes"), "*.txt text eol=crlf\n").unwrap();
        std::fs::write(dir.join("a.txt"), "one\r\ntwo\r\nthree\r\n").unwrap();
        assert!(git::git(&dir, &["add", "-A"]).is_ok());
        assert!(git::checks_out_crlf(&dir, "a.txt") && !git::checks_out_crlf(&dir, ".gitattributes"));
        // One line edited: only it differs, not every CRLF line against the LF blob.
        std::fs::write(dir.join("a.txt"), "one\r\nTWO\r\nthree\r\n").unwrap();
        let d = load_diff(&dir, "a.txt", &DiffSource::Index { staged: false }, false);
        assert_eq!(d.added, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn huge_files_are_not_diffed() {
        let dir = crate::files::test_dir("bigdiff");
        assert!(git::git(&dir, &["init", "-q"]).is_ok());
        let f = std::fs::File::create(dir.join("big.log")).unwrap();
        f.set_len(MAX_OPEN_BYTES + 1).unwrap();
        std::fs::write(dir.join("small.txt"), "a\n").unwrap();
        let unstaged = DiffSource::Index { staged: false };
        assert!(load_diff(&dir, "big.log", &unstaged, false).too_large);
        let small = load_diff(&dir, "small.txt", &unstaged, false);
        assert!(!small.too_large && small.added == 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
