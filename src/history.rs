//! Undo/redo for a `text_editor::Content` (iced 0.14 has none).
//!
//! Edits are grouped like VSCodium: a burst of typing is one step, broken by a
//! pause, a cursor move, a switch between typing and deleting, or the start of
//! a new word; Enter, paste and indent are steps of their own. A step stores
//! only the changed span (common prefix/suffix trimmed), not the whole text,
//! and the stacks are capped, so long sessions on big files stay cheap. The
//! text is read once per step, never per keystroke.

use iced::widget::text_editor::{Action, Content, Cursor, Edit, Position};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const MAX_STEPS: usize = 500;
/// Bytes of text kept per history (both stacks).
const MAX_BYTES: usize = 8 * 1024 * 1024;
const PAUSE: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Typing,
    Deleting,
    /// Enter, paste, indent: always a step of its own.
    Other,
}

struct Pending {
    before: String,
    cursor: Cursor,
    kind: Kind,
    last_char: Option<char>,
    at: Instant,
}

#[derive(Debug)]
struct Step {
    id: u64,
    /// Byte offset of the change.
    start: usize,
    old: String,
    new: String,
    cursor_before: Cursor,
    cursor_after: Cursor,
}

impl Step {
    fn bytes(&self) -> usize {
        self.old.len() + self.new.len()
    }
}

#[derive(Default)]
pub struct History {
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    pending: Option<Pending>,
    next_id: u64,
    /// State id when the undo stack is empty (moves when old steps are dropped).
    floor: u64,
    saved: u64,
    bytes: usize,
}

impl History {
    /// Apply an editor action, recording it when it edits.
    pub fn perform(&mut self, content: &mut Content, action: Action) {
        if !action.is_edit() {
            // A scroll keeps the group; anything that moves the cursor ends it.
            if !matches!(action, Action::Scroll { .. }) {
                self.flush(content);
            }
            content.perform(action);
            return;
        }
        let (kind, ch) = match &action {
            Action::Edit(Edit::Insert(c)) => (Kind::Typing, Some(*c)),
            Action::Edit(Edit::Backspace | Edit::Delete) => (Kind::Deleting, None),
            _ => (Kind::Other, None),
        };
        if let Some(p) = &self.pending {
            let new_word = ch.is_some_and(char::is_whitespace) && p.last_char.is_some_and(|c| !c.is_whitespace());
            if kind == Kind::Other || p.kind != kind || p.at.elapsed() > PAUSE || new_word {
                self.flush(content);
            }
        }
        let now = Instant::now();
        let pending = self.pending.get_or_insert_with(|| Pending {
            before: content.text(),
            cursor: content.cursor(),
            kind,
            last_char: None,
            at: now,
        });
        pending.at = now;
        pending.last_char = ch.or(pending.last_char);
        content.perform(action);
        self.redo.clear();
        if kind == Kind::Other {
            self.flush(content);
        }
    }

    /// Close the open group, if any.
    pub fn flush(&mut self, content: &Content) {
        if let Some(p) = self.pending.take() {
            self.push(&p.before, &content.text(), p.cursor, content.cursor());
        }
    }

    /// Record a change made outside the editor (e.g. the file was reloaded
    /// from disk), so it can be undone too.
    pub fn record(&mut self, before: &str, after: &str, cursor_before: Cursor, cursor_after: Cursor) {
        self.pending = None;
        self.redo.clear();
        self.push(before, after, cursor_before, cursor_after);
    }

    fn push(&mut self, before: &str, after: &str, cursor_before: Cursor, cursor_after: Cursor) {
        let Some((start, old, new)) = changed_span(before, after) else { return };
        self.next_id += 1;
        let step = Step { id: self.next_id, start, old, new, cursor_before, cursor_after };
        if step.bytes() > MAX_BYTES {
            // Too big to keep: start over from here.
            self.undo.clear();
            self.redo.clear();
            self.bytes = 0;
            self.floor = step.id;
            return;
        }
        self.bytes += step.bytes();
        self.undo.push_back(step);
        while self.undo.len() > MAX_STEPS || self.bytes + self.redo_bytes() > MAX_BYTES {
            let Some(old) = self.undo.pop_front() else { break };
            self.bytes -= old.bytes();
            self.floor = old.id;
        }
    }

    fn redo_bytes(&self) -> usize {
        self.redo.iter().map(Step::bytes).sum()
    }

    /// Undo one step. `false` when there is nothing to undo.
    pub fn undo(&mut self, content: &mut Content) -> bool {
        self.flush(content);
        let Some(step) = self.undo.pop_back() else { return false };
        self.bytes -= step.bytes();
        replace(content, step.start, &step.new, &step.old);
        content.move_to(step.cursor_before);
        self.redo.push(step);
        true
    }

    pub fn redo(&mut self, content: &mut Content) -> bool {
        self.flush(content);
        let Some(step) = self.redo.pop() else { return false };
        replace(content, step.start, &step.old, &step.new);
        content.move_to(step.cursor_after);
        self.bytes += step.bytes();
        self.undo.push_back(step);
        true
    }

    fn state(&self) -> u64 {
        self.undo.back().map_or(self.floor, |s| s.id)
    }

    /// The current text matches the file on disk.
    pub fn mark_saved(&mut self, content: &Content) {
        self.flush(content);
        self.saved = self.state();
    }

    /// Unsaved changes, counting undos back to the saved text as clean.
    pub fn is_dirty(&self) -> bool {
        self.pending.is_some() || self.state() != self.saved
    }
}

/// `(offset, old, new)`: the span that differs between `before` and `after`,
/// on char boundaries and never splitting a `\r\n`.
fn changed_span(before: &str, after: &str) -> Option<(usize, String, String)> {
    if before == after {
        return None;
    }
    let (b, a) = (before.as_bytes(), after.as_bytes());
    let splits = |s: &[u8], i: usize| i > 0 && i < s.len() && s[i - 1] == b'\r' && s[i] == b'\n';
    let mut start = b.iter().zip(a).take_while(|(x, y)| x == y).count();
    while !before.is_char_boundary(start) || !after.is_char_boundary(start) || splits(b, start) || splits(a, start) {
        start -= 1;
    }
    let max_suffix = b.len().min(a.len()) - start;
    let mut suffix = b.iter().rev().zip(a.iter().rev()).take(max_suffix).take_while(|(x, y)| x == y).count();
    let (mut eb, mut ea) = (b.len() - suffix, a.len() - suffix);
    while !before.is_char_boundary(eb) || !after.is_char_boundary(ea) || splits(b, eb) || splits(a, ea) {
        suffix -= 1;
        (eb, ea) = (b.len() - suffix, a.len() - suffix);
    }
    Some((start, before[start..eb].to_string(), after[start..ea].to_string()))
}

/// `offset` clamped into `text` and moved back to a char boundary. Offsets
/// come from the history; if it ever disagrees with the editor's text, an
/// off-boundary slice would panic (and the release build aborts on panic).
fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Editor position (line, byte column) of a byte offset into `text`.
fn position(text: &str, offset: usize) -> Position {
    let offset = floor_boundary(text, offset);
    let head = &text[..offset];
    let line = head.matches('\n').count();
    let line_start = head.rfind('\n').map_or(0, |i| i + 1);
    Position { line, column: offset - line_start }
}

/// Replace `old` (which is at byte `start`) with `new`, through the editor so
/// its scroll and highlighting survive.
fn replace(content: &mut Content, start: usize, old: &str, new: &str) {
    let text = content.text();
    let start = floor_boundary(&text, start);
    let end = floor_boundary(&text, start + old.len());
    let (from, to) = (position(&text, start), position(&text, end));
    content.move_to(Cursor { position: to, selection: (from != to).then_some(from) });
    content.perform(Action::Edit(Edit::Paste(std::sync::Arc::new(new.to_string()))));
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::text_editor::Motion;

    fn typed(h: &mut History, c: &mut Content, s: &str) {
        for ch in s.chars() {
            let edit = match ch {
                '\n' => Edit::Enter,
                '\u{8}' => Edit::Backspace,
                ch => Edit::Insert(ch),
            };
            h.perform(c, Action::Edit(edit));
        }
    }

    #[test]
    fn spans_are_minimal_and_safe() {
        assert_eq!(changed_span("abc", "abc"), None);
        assert_eq!(changed_span("hello world", "hello brave world"), Some((6, "".into(), "brave ".into())));
        assert_eq!(changed_span("aéb", "aèb"), Some((1, "é".into(), "è".into())));
        assert_eq!(changed_span("aaa", "aaaa"), Some((3, "".into(), "a".into())));
        // Never split a CRLF.
        assert_eq!(changed_span("a\r\nb", "a\r\r\nb"), Some((1, "".into(), "\r".into())));
        assert_eq!(position("ab\ncd\r\nef", 8), Position { line: 2, column: 1 });
    }

    #[test]
    fn spans_keep_crlf_and_multibyte_whole() {
        // Inserting a line before a CRLF line ending.
        assert_eq!(changed_span("a\r\nb", "a\r\nx\r\nb"), Some((3, "".into(), "x\r\n".into())));
        // Changing an ending (\n ⇄ \r\n) replaces the whole ending, never half of it.
        assert_eq!(changed_span("a\nb", "a\r\nb"), Some((1, "\n".into(), "\r\n".into())));
        assert_eq!(changed_span("a\r\nb", "a\nb"), Some((1, "\r\n".into(), "\n".into())));
        // Characters that share leading or trailing bytes stay whole.
        assert_eq!(changed_span("日本", "日木"), Some((3, "本".into(), "木".into())));
        assert_eq!(changed_span("é", "è"), Some((0, "é".into(), "è".into())));
        assert_eq!(changed_span("👍x", "x"), Some((0, "👍".into(), "".into())));
    }

    #[test]
    fn offsets_off_a_char_boundary_or_past_the_end_are_clamped() {
        assert_eq!(position("aé", 2), Position { line: 0, column: 1 });
        assert_eq!(position("ab", 99), Position { line: 0, column: 2 });
        let mut c = Content::with_text("aéb");
        replace(&mut c, 2, "éb", "z");
        assert_eq!(c.text(), "az");
    }

    #[test]
    fn typing_groups_by_word_and_undo_redo_round_trip() {
        let mut c = Content::with_text("fn main() {}\n");
        c.perform(Action::Move(Motion::DocumentEnd));
        let mut h = History::default();
        h.mark_saved(&c);
        typed(&mut h, &mut c, "let x = 1");
        assert!(h.is_dirty());
        // One step per word, like VSCodium: "let", " x", " =", " 1".
        assert!(h.undo(&mut c));
        assert_eq!(c.text(), "fn main() {}\nlet x =");
        assert!(h.undo(&mut c) && h.undo(&mut c));
        assert_eq!(c.text(), "fn main() {}\nlet");
        assert!(h.undo(&mut c));
        assert_eq!(c.text(), "fn main() {}\n");
        assert!(!h.is_dirty(), "back at the saved text");
        assert!(!h.undo(&mut c));
        while h.redo(&mut c) {}
        assert_eq!(c.text(), "fn main() {}\nlet x = 1");
        assert_eq!(c.cursor().position, Position { line: 1, column: 9 });
        assert!(!h.redo(&mut c));
    }

    #[test]
    fn deleting_moving_and_pasting_are_separate_steps() {
        let mut c = Content::with_text("one\ntwo");
        let mut h = History::default();
        c.perform(Action::Move(Motion::DocumentEnd));
        typed(&mut h, &mut c, "\u{8}\u{8}");
        h.perform(&mut c, Action::Move(Motion::DocumentStart));
        h.perform(&mut c, Action::Edit(Edit::Paste(std::sync::Arc::new("zero\n".into()))));
        typed(&mut h, &mut c, "\n");
        assert_eq!(c.text(), "zero\n\none\nt");
        h.undo(&mut c);
        assert_eq!(c.text(), "zero\none\nt");
        h.undo(&mut c);
        assert_eq!(c.text(), "one\nt");
        h.undo(&mut c);
        assert_eq!(c.text(), "one\ntwo");
    }

    #[test]
    fn replacing_a_selection_restores_it_on_undo() {
        let mut c = Content::with_text("alpha beta");
        let mut h = History::default();
        let sel =
            Cursor { position: Position { line: 0, column: 10 }, selection: Some(Position { line: 0, column: 6 }) };
        c.move_to(sel);
        typed(&mut h, &mut c, "gamma");
        assert_eq!(c.text(), "alpha gamma");
        h.undo(&mut c);
        assert_eq!(c.text(), "alpha beta");
        assert_eq!(c.selection().as_deref(), Some("beta"));
    }

    #[test]
    fn a_new_edit_drops_redo_and_external_changes_are_undoable() {
        let mut c = Content::with_text("a");
        let mut h = History::default();
        c.perform(Action::Move(Motion::DocumentEnd));
        typed(&mut h, &mut c, "b");
        h.undo(&mut c);
        typed(&mut h, &mut c, "c");
        assert!(!h.redo(&mut c));
        assert_eq!(c.text(), "ac");

        let cur = c.cursor();
        let mut c2 = Content::with_text("ac\nfrom disk");
        h.record("ac", "ac\nfrom disk", cur, c2.cursor());
        h.mark_saved(&c2);
        assert!(h.undo(&mut c2));
        assert_eq!(c2.text(), "ac");
        assert!(h.is_dirty());
    }

    #[test]
    fn caps_drop_the_oldest_steps() {
        let mut c = Content::with_text("");
        let mut h = History::default();
        h.mark_saved(&c);
        for _ in 0..MAX_STEPS + 20 {
            h.perform(&mut c, Action::Edit(Edit::Enter));
        }
        let mut n = 0;
        while h.undo(&mut c) {
            n += 1;
        }
        assert_eq!(n, MAX_STEPS);
        assert!(h.is_dirty(), "the dropped steps are not undone, so the text differs from the save");
    }
}
