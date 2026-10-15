//! Moving around files, as in VSCodium: find in the open file (Ctrl+F, a bar
//! per file tab), go to line (Ctrl+G) and quick open (Ctrl+P), the last two in
//! one palette (`:` goes to a line).
//!
//! The find bar lives in its `FileTab` (`FileTab::find`): each tab keeps its
//! query and matches, which follow its text (edits, undo, reloads). Every
//! cursor move made here goes through `FileTab::follow`, so the line numbers
//! stay with the editor's scroll.

use super::{App, FileTab, Msg, Tab, blocking};
use crate::quickopen;
use crate::textfind::{self, Match};

use iced::Task;
use iced::widget::operation;
use iced::widget::text_editor::{self, Cursor, Position};

/// Widget id of the find bar's input.
pub const FIND_INPUT: &str = "find-in-file";
/// Widget id of the quick-open input.
pub const PALETTE_INPUT: &str = "quick-open";
/// Widget id of the file editor (one at a time: the active tab's).
pub const EDITOR: &str = "file-editor";
/// Results shown in the palette.
pub const PALETTE_ROWS: usize = 12;

/// The find bar of one file tab.
#[derive(Debug, Default)]
pub struct FindBar {
    pub query: String,
    pub case_sensitive: bool,
    /// In text order, at most `textfind::MAX_MATCHES` (`capped`).
    pub matches: Vec<Match>,
    pub capped: bool,
    /// The match the editor has selected.
    pub current: Option<usize>,
}

/// The quick-open palette, while open.
#[derive(Debug, Default)]
pub struct QuickOpen {
    pub query: String,
    /// The folder's files, relative; `None` while they are listed.
    pub files: Option<Vec<String>>,
    /// The list stopped at `quickopen::MAX_FILES`.
    pub capped: bool,
    /// Indices into `files`, best first.
    pub results: Vec<usize>,
    pub selected: usize,
    /// Matches the file list asked for when it opened (a reopened palette asks again).
    seq: u64,
    /// The terminal had the keyboard when it opened: cancelling gives it back.
    from_terminal: bool,
}

impl QuickOpen {
    /// The line typed after `:` (go to line), if the query is one.
    pub fn line_query(&self) -> Option<&str> {
        self.query.strip_prefix(':').map(str::trim)
    }

    fn rank(&mut self) {
        self.selected = 0;
        self.results = match (&self.files, self.line_query()) {
            (Some(files), None) => quickopen::rank(&self.query, files, PALETTE_ROWS),
            _ => Vec::new(),
        };
    }
}

/// The selection as `(line, start, end)` when it is within one line.
fn selection_span(c: Cursor) -> Option<(usize, usize, usize)> {
    let s = c.selection?;
    (s.line == c.position.line).then(|| (s.line, s.column.min(c.position.column), s.column.max(c.position.column)))
}

/// Where a search from the cursor starts: the selection's start, or the cursor.
fn search_from(c: Cursor) -> (Position, bool) {
    match c.selection {
        Some(s) if (s.line, s.column) < (c.position.line, c.position.column) => (s, true),
        Some(_) => (c.position, true),
        None => (c.position, false),
    }
}

impl FileTab {
    /// Recompute the find bar's matches (query, case or text changed).
    pub(super) fn refind(&mut self) {
        let Some(f) = &mut self.find else { return };
        let (matches, capped) = textfind::find(self.content.lines().map(|l| l.text), &f.query, f.case_sensitive);
        f.matches = matches;
        f.capped = capped;
        self.sync_find();
    }

    /// The current match follows the editor's selection (after a click, a move…).
    pub(super) fn sync_find(&mut self) {
        let Some(f) = &mut self.find else { return };
        f.current = selection_span(self.content.cursor()).and_then(|(l, s, e)| textfind::selected(&f.matches, l, s, e));
    }

    /// Select match `i` in the editor and scroll to it.
    fn select_match(&mut self, i: usize, view: Option<f32>) {
        let Some(m) = self.find.as_ref().and_then(|f| f.matches.get(i)).copied() else { return };
        self.content.move_to(Cursor {
            position: Position { line: m.line, column: m.end },
            selection: Some(Position { line: m.line, column: m.start }),
        });
        if let Some(f) = &mut self.find {
            f.current = Some(i);
        }
        self.reveal = false;
        self.follow(view);
    }

    /// Go to the next (or previous) match from the cursor. `incremental`
    /// (typing a query): a match at the selection's start counts, so it grows
    /// in place instead of jumping on.
    fn find_step(&mut self, forward: bool, incremental: bool, view: Option<f32>) {
        let Some(f) = &self.find else { return };
        let (from, selection) = search_from(self.content.cursor());
        let skip = forward && selection && !incremental;
        if let Some(i) = textfind::pick(&f.matches, (from.line, from.column), forward, skip) {
            self.select_match(i, view);
        }
    }

    /// Put the cursor at the start of `line` (clamped), without a selection.
    pub(super) fn go_to_line(&mut self, line: usize, view: Option<f32>) {
        // `move_to` keeps an existing selection; moving collapses it first.
        if self.content.cursor().selection.is_some() {
            self.content.perform(text_editor::Action::Move(text_editor::Motion::Left));
        }
        let line = line.min(self.content.line_count().saturating_sub(1));
        self.content.move_to(Cursor { position: Position { line, column: 0 }, selection: None });
        self.reveal = false;
        self.follow(view);
    }
}

impl App {
    /// The active tab, if it is a file.
    pub fn active_file(&self) -> Option<&FileTab> {
        match self.active_tab.and_then(|i| self.tabs.get(i)) {
            Some(Tab::File(t)) => Some(t),
            _ => None,
        }
    }

    /// The active file's find bar, if open.
    pub(super) fn active_find(&self) -> Option<&FindBar> {
        self.active_file().and_then(|t| t.find.as_deref())
    }

    pub(super) fn update_navigate(&mut self, msg: Msg) -> Task<Msg> {
        let view = self.editor_view;
        match msg {
            Msg::Modifiers(m) => self.modifiers = m,
            Msg::FindOpen => {
                let Some(t) = self.active_file_mut() else { return Task::none() };
                // Like VSCodium: a selection within one line becomes the query.
                let seed = t.content.selection().filter(|s| !s.is_empty() && !s.contains(['\n', '\r']));
                let f = t.find.get_or_insert_with(Box::default);
                if let Some(s) = seed {
                    f.query = s;
                }
                t.refind();
                self.focus_epoch += 1;
                self.term_focus = false;
                return operation::focus(FIND_INPUT).chain(operation::select_all(FIND_INPUT));
            }
            Msg::FindQuery(q) => {
                if let Some(t) = self.active_file_mut()
                    && let Some(f) = &mut t.find
                {
                    f.query = q;
                    t.refind();
                    t.find_step(true, true, view);
                }
            }
            Msg::FindToggleCase => {
                if let Some(t) = self.active_file_mut()
                    && let Some(f) = &mut t.find
                {
                    f.case_sensitive = !f.case_sensitive;
                    t.refind();
                    t.find_step(true, true, view);
                }
            }
            Msg::FindStep(forward) => {
                if let Some(t) = self.active_file_mut() {
                    t.find_step(forward, false, view);
                }
            }
            // Enter in the input; with Shift, back (text_input only says "submit").
            Msg::FindSubmit => {
                let forward = !self.modifiers.shift();
                if let Some(t) = self.active_file_mut() {
                    t.find_step(forward, false, view);
                }
            }
            Msg::FindClose => {
                if let Some(t) = self.active_file_mut()
                    && t.find.take().is_some()
                {
                    // Back to the editor, where the match stays selected.
                    return operation::focus(EDITOR);
                }
            }
            Msg::PaletteOpen(go_to_line) => return self.open_palette(go_to_line),
            Msg::PaletteFrom(terminal) => {
                if let Some(p) = &mut self.palette {
                    p.from_terminal = terminal;
                }
            }
            Msg::PaletteFiles(seq, (files, capped)) => {
                if let Some(p) = self.palette.as_mut().filter(|p| p.seq == seq) {
                    p.files = Some(files);
                    p.capped = capped;
                    p.rank();
                }
            }
            Msg::PaletteQuery(q) => {
                if let Some(p) = &mut self.palette {
                    p.query = q;
                    p.rank();
                }
            }
            Msg::PaletteMove(step) => {
                if let Some(p) = &mut self.palette
                    && !p.results.is_empty()
                {
                    let n = p.results.len() as isize;
                    p.selected = (p.selected as isize + step).rem_euclid(n) as usize;
                }
            }
            Msg::PalettePick(i) => {
                if let Some(p) = &mut self.palette {
                    p.selected = i;
                }
                return self.palette_submit();
            }
            Msg::PaletteSubmit => return self.palette_submit(),
            Msg::PaletteClose => {
                let back = match self.palette.take() {
                    Some(p) if p.from_terminal => self.focus_terminal(),
                    Some(_) if self.active_file_mut().is_some() => operation::focus(EDITOR),
                    _ => Task::none(),
                };
                return back;
            }
            _ => {}
        }
        Task::none()
    }

    fn open_palette(&mut self, go_to_line: bool) -> Task<Msg> {
        if self.signin.is_some() {
            return Task::none();
        }
        let prefill = if go_to_line { ":" } else { "" };
        let fresh = self.palette.is_none();
        let mut list = Task::none();
        match &mut self.palette {
            Some(p) => {
                p.query = prefill.to_string();
                p.rank();
            }
            None => {
                self.palette_seq += 1;
                let seq = self.palette_seq;
                let root = self.root.clone();
                list = self.for_root(Task::perform(blocking(move || quickopen::list_files(&root)), move |files| {
                    Msg::PaletteFiles(seq, files)
                }));
                self.palette = Some(QuickOpen { query: prefill.to_string(), seq, ..QuickOpen::default() });
            }
        }
        // The input takes the keyboard (the focus also leaves the terminal).
        self.focus_epoch += 1;
        self.term_focus = false;
        let focus = move || {
            let caret = if go_to_line {
                operation::move_cursor_to_end(PALETTE_INPUT)
            } else {
                operation::select_all(PALETTE_INPUT)
            };
            operation::focus(PALETTE_INPUT).chain(caret)
        };
        // Ask first whether the terminal had the keyboard, to give it back on Escape
        // (once: a palette already open has it).
        let focus = match &self.term {
            Some(t) if fresh => operation::is_focused(t.widget_id().clone())
                .then(move |was| Task::done(Msg::PaletteFrom(was)).chain(focus())),
            _ => focus(),
        };
        Task::batch([list, focus])
    }

    fn palette_submit(&mut self) -> Task<Msg> {
        let Some(p) = &self.palette else { return Task::none() };
        if let Some(line) = p.line_query() {
            let Ok(n) = line.parse::<usize>() else { return Task::none() };
            let view = self.editor_view;
            let Some(t) = self.active_file_mut() else {
                self.set_status("Open a file to go to a line", false);
                return Task::none();
            };
            t.go_to_line(n.saturating_sub(1), view);
            self.palette = None;
            return operation::focus(EDITOR);
        }
        let Some(rel) = p.results.get(p.selected).and_then(|&i| p.files.as_ref()?.get(i)) else {
            return Task::none();
        };
        let path = self.root.join(rel);
        self.palette = None;
        let open = self.update(Msg::OpenFile(path.clone(), None));
        // Focus the editor only if the file opened (it can be binary or too large).
        let opened = matches!(self.active_tab.and_then(|i| self.tabs.get(i)), Some(Tab::File(t)) if t.path == path);
        if opened { Task::batch([open, operation::focus(EDITOR)]) } else { open }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor(line: usize, column: usize, selection: Option<(usize, usize)>) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: selection.map(|(line, column)| Position { line, column }),
        }
    }

    #[test]
    fn a_selection_within_a_line_is_a_span_either_way() {
        assert_eq!(selection_span(cursor(3, 9, Some((3, 4)))), Some((3, 4, 9)));
        assert_eq!(selection_span(cursor(3, 4, Some((3, 9)))), Some((3, 4, 9)));
        assert_eq!(selection_span(cursor(3, 4, Some((5, 0)))), None);
        assert_eq!(selection_span(cursor(3, 4, None)), None);
    }

    #[test]
    fn searching_starts_at_the_selection_start() {
        assert_eq!(search_from(cursor(3, 9, Some((3, 4)))), (Position { line: 3, column: 4 }, true));
        assert_eq!(search_from(cursor(3, 4, Some((7, 0)))), (Position { line: 3, column: 4 }, true));
        assert_eq!(search_from(cursor(2, 1, None)), (Position { line: 2, column: 1 }, false));
    }

    const VIEW: Option<f32> = Some(100.0);

    fn find(t: &mut FileTab, query: &str) {
        t.find = Some(Box::new(FindBar { query: query.into(), ..FindBar::default() }));
        t.refind();
    }

    fn selected_text(t: &FileTab) -> Option<String> {
        t.content.selection()
    }

    #[test]
    fn steps_select_matches_in_the_editor_and_wrap() {
        let mut t = FileTab::for_test("foo bar\nbaz FOO\nfoo");
        find(&mut t, "foo");
        assert_eq!(t.find.as_ref().map(|f| f.matches.len()), Some(3));
        // From the cursor at the start: the first match, then on.
        t.find_step(true, false, VIEW);
        assert_eq!(t.content.cursor(), cursor(0, 3, Some((0, 0))));
        assert_eq!(selected_text(&t).as_deref(), Some("foo"));
        t.find_step(true, false, VIEW);
        assert_eq!(t.content.cursor(), cursor(1, 7, Some((1, 4))));
        assert_eq!(t.find.as_ref().and_then(|f| f.current), Some(1));
        t.find_step(true, false, VIEW);
        t.find_step(true, false, VIEW);
        assert_eq!(t.find.as_ref().and_then(|f| f.current), Some(0), "wraps to the first");
        t.find_step(false, false, VIEW);
        assert_eq!(t.content.cursor(), cursor(2, 3, Some((2, 0))), "back wraps to the last");
    }

    #[test]
    fn typing_grows_the_match_in_place() {
        let mut t = FileTab::for_test("fa foo");
        find(&mut t, "f");
        t.find_step(true, true, VIEW);
        assert_eq!(t.content.cursor(), cursor(0, 1, Some((0, 0))));
        find(&mut t, "fo");
        t.find_step(true, true, VIEW);
        assert_eq!(t.content.cursor(), cursor(0, 5, Some((0, 3))));
        // The selected match is the current one after a recount.
        t.refind();
        assert_eq!(t.find.as_ref().and_then(|f| f.current), Some(0));
    }

    #[test]
    fn going_to_a_line_drops_the_selection_and_clamps() {
        let mut t = FileTab::for_test("a\nfoo\nc");
        find(&mut t, "foo");
        t.find_step(true, false, VIEW);
        assert!(t.content.cursor().selection.is_some());
        t.go_to_line(99, VIEW);
        assert_eq!(t.content.cursor(), cursor(2, 0, None));
        t.go_to_line(0, VIEW);
        assert_eq!(t.content.cursor(), cursor(0, 0, None));
    }

    #[test]
    fn the_palette_ranks_files_unless_going_to_a_line() {
        let mut p = QuickOpen {
            files: Some(vec!["src/main.rs".into(), "README.md".into()]),
            query: "main".into(),
            ..QuickOpen::default()
        };
        p.rank();
        assert_eq!(p.results, [0]);
        p.query = ": 42".into();
        p.rank();
        assert!(p.results.is_empty());
        assert_eq!(p.line_query(), Some("42"));
    }
}
