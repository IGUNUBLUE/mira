//! Document tabs: files (edit, undo, save, autosave) and diffs, plus the explorer click.

use super::{App, Msg, blocking, file_name};
use crate::diff::{self, FileDiff};
use crate::git;
use crate::persist;
use crate::runs::RunId;

use iced::Task;
use iced::widget::text_editor::{self, Content};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// Largest file opened in a tab or diffed (bytes).
const MAX_OPEN_BYTES: u64 = 8 * 1024 * 1024;

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
    /// Shared with the view's cached rows (see `view_diff`).
    pub diff: Option<Arc<FileDiff>>,
    /// Line clicked in the diff (new-side number preferred), for comments.
    pub anchor: Option<usize>,
    /// Working file's mtime when the diff was last computed (index diffs).
    pub loaded_mtime: Option<SystemTime>,
    /// The file exists in the working tree (checked when the diff loads), for "Open file".
    pub on_disk: bool,
}

pub enum Tab {
    File(FileTab),
    Diff(DiffTab),
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
    };
    if too_large {
        return FileDiff { too_large: true, id: diff::next_id(), ..Default::default() };
    }
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

impl App {
    pub(super) fn update_documents(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::OpenFile(path, line) => self.open_file(path, line),
            Msg::SelectTab(i) => {
                if i < self.tabs.len() {
                    if self.autosave == AutoSave::OnFocusChange
                        && let Some(prev) = self.active_tab.filter(|p| *p != i)
                    {
                        self.save_tab(prev, false);
                    }
                    self.active_tab = Some(i);
                    return self.reload_active_diff();
                }
            }
            Msg::CloseTab(i) => self.close_tab(i),
            Msg::Edit(action) => {
                let edit = action.is_edit();
                if edit {
                    // Editing again: a later Ctrl+W must warn again.
                    self.pending_close = None;
                }
                if let Some(t) = self.active_file_mut() {
                    t.history.perform(&mut t.content, action);
                    if edit {
                        t.dirty = true;
                        t.edited_at = Some(Instant::now());
                    }
                }
            }
            Msg::Save => {
                if let Some(i) = self.active_tab {
                    self.save_tab(i, true);
                }
            }
            Msg::Undo | Msg::Redo => {
                if let Some(t) = self.active_file_mut() {
                    let done = if matches!(msg, Msg::Undo) {
                        t.history.undo(&mut t.content)
                    } else {
                        t.history.redo(&mut t.content)
                    };
                    if done {
                        t.dirty = t.history.is_dirty();
                        t.edited_at = Some(Instant::now());
                    }
                }
            }
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
            Msg::DiffLoaded(rel, source, full, diff) => {
                let on_disk = self.repo_root().join(&rel).is_file();
                for tab in &mut self.tabs {
                    if let Tab::Diff(d) = tab
                        && d.rel == rel
                        && d.source == source
                        && d.full == full
                    {
                        d.diff = Some(Arc::clone(&diff));
                        d.on_disk = on_disk;
                    }
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
                    return self.send_to_agent(text, None);
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
            _ => {}
        }
        Task::none()
    }

    /// The active tab, if it is a diff.
    pub(super) fn active_diff(&self) -> Option<&DiffTab> {
        match self.active_tab.and_then(|i| self.tabs.get(i)) {
            Some(Tab::Diff(d)) => Some(d),
            _ => None,
        }
    }

    /// The active tab, if it is a file.
    fn active_file_mut(&mut self) -> Option<&mut FileTab> {
        match self.active_tab.and_then(|i| self.tabs.get_mut(i)) {
            Some(Tab::File(t)) => Some(t),
            _ => None,
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
                        crumbs: crumbs(&self.root, &path),
                        content: Content::with_text(&text),
                        path,
                        dirty: false,
                        syntax,
                        history: crate::history::History::default(),
                        edited_at: None,
                        conflict: false,
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
            })
        };
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

    pub(super) fn reload_active_diff(&mut self) -> Task<Msg> {
        let root = self.repo_root();
        let Some(Tab::Diff(d)) = self.active_tab.and_then(|i| self.tabs.get_mut(i)) else {
            return Task::none();
        };
        d.loaded_mtime = mtime(&root.join(&d.rel));
        let (rel, source, full) = (d.rel.clone(), d.source.clone(), d.full);
        Task::perform(
            blocking(move || {
                let diff = Arc::new(load_diff(&root, &rel, &source, full));
                (rel, source, full, diff)
            }),
            |(rel, source, full, diff)| Msg::DiffLoaded(rel, source, full, diff),
        )
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
            && self.pending_close.as_ref() != Some(&t.path)
        {
            self.pending_close = Some(t.path.clone());
            self.set_confirm("Unsaved changes — close again to discard them (Ctrl+S saves · Esc cancels)");
            return;
        }
        self.pending_close = None;
        self.tabs.remove(i);
        self.active_tab = active_after_remove(self.active_tab, i, self.tabs.len());
        self.sync_center();
    }

    /// Save tab `i`. Autosave (`manual == false`) never overwrites a file that
    /// changed on disk since it was loaded (an agent may have written it): it
    /// pauses for that tab and says so; Ctrl+S still overwrites.
    pub(super) fn save_tab(&mut self, i: usize, manual: bool) -> bool {
        let Some(Tab::File(t)) = self.tabs.get_mut(i) else { return false };
        if !manual {
            if !t.dirty || t.conflict {
                return false;
            }
            if mtime(&t.path) != t.mtime {
                t.conflict = true;
                let name = file_name(&t.path);
                self.set_status(
                    format!("{name} changed on disk — autosave paused for it; Ctrl+S overwrites the file"),
                    true,
                );
                return false;
            }
        }
        match std::fs::write(&t.path, t.content.text()) {
            Ok(()) => {
                t.history.mark_saved(&t.content);
                t.dirty = false;
                t.conflict = false;
                t.edited_at = None;
                t.mtime = mtime(&t.path);
                let name = file_name(&t.path);
                if manual {
                    self.set_status(format!("Saved {name}"), false);
                }
                true
            }
            Err(e) => {
                // Stop retrying every tick; a manual save tries again.
                t.conflict = true;
                let name = file_name(&t.path);
                self.set_status(format!("Save {name} failed: {e}"), true);
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
        let mut deleted = Vec::new();
        for tab in &mut self.tabs {
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
            if let Ok(text) = read_text(&t.path) {
                t.conflict = false;
                let old = t.content.text();
                let before = t.content.cursor();
                t.content = Content::with_text(&text);
                let line = before.position.line.min(t.content.line_count().saturating_sub(1));
                t.content.move_to(text_editor::Cursor {
                    position: text_editor::Position { line, column: 0 },
                    selection: None,
                });
                // Undoable, like VSCodium: Ctrl+Z brings back the text before the agent's change.
                t.history.record(&old, &t.content.text(), before, t.content.cursor());
                t.history.mark_saved(&t.content);
            }
        }
        if let Some(first) = deleted.first() {
            let more = if deleted.len() > 1 { format!(" (and {} more)", deleted.len() - 1) } else { String::new() };
            self.set_status(format!("{first}{more} deleted on disk — autosave paused; Ctrl+S writes it again"), true);
        }
    }
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
mod tests {
    use super::*;

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
