//! Line numbers for the editor.
//!
//! iced 0.14's `text_editor` has no gutter and doesn't expose its scroll
//! position, so mira mirrors it: with wrapping off every line is one row of
//! the same height, and cosmic-text (0.15, which scrolls the editor) moves the
//! view by three rules only, reproduced here:
//!
//! - the wheel adds `lines × line height` (`Action::Scroll`);
//! - when the cursor moves, the view moves just enough to show its line
//!   (`Buffer::shape_until_cursor`);
//! - the view never goes past the top, nor past the point where the last line
//!   sits at the bottom (`Buffer::shape_until_scroll`).
//!
//! Every editor action goes through [`Scroll::scrolled`] or
//! [`Scroll::settle`], so the gutter follows the text exactly.

use iced::widget::text_editor::Position;

/// Height of one editor row for `size`: iced's default `LineHeight::Relative(1.3)`.
pub fn line_height(size: f32) -> f32 {
    size * 1.3
}

/// The mirrored vertical scroll of one editor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scroll {
    /// Pixels between the top of the first line and the top of the view.
    offset: f32,
    /// The cursor the view last followed; `None` to follow at the next settle.
    cursor: Option<Position>,
}

impl Default for Scroll {
    /// A new editor: at the top, cursor at the start.
    fn default() -> Self {
        Self { offset: 0.0, cursor: Some(Position { line: 0, column: 0 }) }
    }
}

impl Scroll {
    /// The wheel moved the view by `lines`.
    pub fn scrolled(&mut self, lines: i32, row: f32, line_count: usize, view: Option<f32>) {
        self.offset += lines as f32 * row;
        if let Some(view) = view {
            self.offset = clamp(self.offset, row, line_count, view);
        }
    }

    /// After any other action: follow the cursor if it moved. Without the
    /// view's height yet (`view` is `None`) nothing moves, as in the editor;
    /// the follow happens once the height arrives.
    pub fn settle(&mut self, cursor: Position, row: f32, line_count: usize, view: Option<f32>) {
        let Some(view) = view else { return };
        if self.cursor != Some(cursor) {
            let top = cursor.line as f32 * row;
            if top < self.offset {
                self.offset = top;
            } else if top + row > self.offset + view {
                self.offset = top + row - view;
            }
            self.cursor = Some(cursor);
        }
        self.offset = clamp(self.offset, row, line_count, view);
    }

    /// The next [`Scroll::settle`] follows the cursor even if it didn't move
    /// (mira makes the editor do the same, see `FileTab::reveal`).
    pub fn force_follow(&mut self) {
        self.cursor = None;
    }

    /// Offset of the view for drawing.
    pub fn offset(&self, row: f32, line_count: usize, view: f32) -> f32 {
        clamp(self.offset, row, line_count, view)
    }
}

fn clamp(offset: f32, row: f32, line_count: usize, view: f32) -> f32 {
    offset.min(line_count as f32 * row - view).max(0.0)
}

/// Which lines to draw for a view of height `view` scrolled by `offset`:
/// `(first line, how many, pixels the first one is cut off at the top)`.
pub fn visible(offset: f32, row: f32, line_count: usize, view: f32) -> (usize, usize, f32) {
    let first = ((offset / row).floor() as usize).min(line_count.saturating_sub(1));
    let cut = offset - first as f32 * row;
    let count = (((view + cut) / row).ceil() as usize).min(line_count - first.min(line_count));
    (first, count, cut)
}

/// Width of the gutter for `line_count` lines: room for the digits plus margins.
pub fn width(line_count: usize, char_width: f32) -> f32 {
    let digits = line_count.max(1).ilog10() as f32 + 1.0;
    digits.max(2.0) * char_width + 24.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: f32 = 18.2;
    const VIEW: f32 = 10.0 * ROW + 5.0; // ten and a bit rows

    fn at(line: usize) -> Position {
        Position { line, column: 0 }
    }

    #[test]
    fn follows_the_cursor_down_and_up_like_the_editor() {
        let mut s = Scroll::default();
        // Moving inside the view doesn't scroll.
        s.settle(at(5), ROW, 100, Some(VIEW));
        assert_eq!(s.offset(ROW, 100, VIEW), 0.0);
        // Line 20 ends at 21 rows: its bottom lands on the view's bottom.
        s.settle(at(20), ROW, 100, Some(VIEW));
        assert!((s.offset(ROW, 100, VIEW) - (21.0 * ROW - VIEW)).abs() < 1e-3);
        // Back above the top: that line becomes the first.
        s.settle(at(3), ROW, 100, Some(VIEW));
        assert_eq!(s.offset(ROW, 100, VIEW), 3.0 * ROW);
    }

    #[test]
    fn the_wheel_scrolls_and_stops_at_both_ends() {
        let mut s = Scroll::default();
        s.scrolled(5, ROW, 100, Some(VIEW));
        assert_eq!(s.offset(ROW, 100, VIEW), 5.0 * ROW);
        s.scrolled(-50, ROW, 100, Some(VIEW));
        assert_eq!(s.offset(ROW, 100, VIEW), 0.0);
        s.scrolled(500, ROW, 100, Some(VIEW));
        assert!((s.offset(ROW, 100, VIEW) - (100.0 * ROW - VIEW)).abs() < 1e-3);
        // A file shorter than the view never scrolls.
        let mut short = Scroll::default();
        short.scrolled(3, ROW, 4, Some(VIEW));
        assert_eq!(short.offset(ROW, 4, VIEW), 0.0);
    }

    #[test]
    fn an_unmoved_cursor_keeps_the_wheel_position() {
        let mut s = Scroll::default();
        s.settle(at(2), ROW, 100, Some(VIEW));
        s.scrolled(40, ROW, 100, Some(VIEW));
        // Selecting with the mouse elsewhere moved nothing: still scrolled.
        s.settle(at(2), ROW, 100, Some(VIEW));
        assert_eq!(s.offset(ROW, 100, VIEW), 40.0 * ROW);
        // Typing moves the cursor (one column): the view jumps back to it.
        s.settle(Position { line: 2, column: 1 }, ROW, 100, Some(VIEW));
        assert_eq!(s.offset(ROW, 100, VIEW), 2.0 * ROW);
    }

    #[test]
    fn waits_for_the_view_size_before_following() {
        let mut s = Scroll::default();
        s.settle(at(50), ROW, 100, None);
        assert_eq!(s.offset(ROW, 100, VIEW), 0.0);
        s.settle(at(50), ROW, 100, Some(VIEW));
        assert!((s.offset(ROW, 100, VIEW) - (51.0 * ROW - VIEW)).abs() < 1e-3);
    }

    #[test]
    fn deleting_lines_pulls_the_view_back() {
        let mut s = Scroll::default();
        s.settle(at(99), ROW, 100, Some(VIEW));
        // The file shrinks to 30 lines, cursor now on line 29.
        s.settle(at(29), ROW, 30, Some(VIEW));
        assert!((s.offset(ROW, 30, VIEW) - (30.0 * ROW - VIEW)).abs() < 1e-3);
    }

    #[test]
    fn visible_rows_and_gutter_width() {
        assert_eq!(visible(0.0, ROW, 100, VIEW), (0, 11, 0.0));
        let (first, count, cut) = visible(3.5 * ROW, ROW, 100, VIEW);
        assert_eq!((first, count), (3, 11));
        assert!((cut - 0.5 * ROW).abs() < 1e-3);
        assert_eq!(visible(0.0, ROW, 3, VIEW), (0, 3, 0.0));
        assert_eq!(visible(0.0, ROW, 1, VIEW).1, 1);
        assert!(width(9, 8.0) == width(99, 8.0) && width(100, 8.0) > width(99, 8.0));
    }
}

/// The model against the editor's real text engine: cosmic-text, driven the
/// way iced drives it (an action, then `shape_as_needed` at layout).
#[cfg(test)]
mod against_cosmic_text {
    use super::*;
    use cosmic_text::{Action, Attrs, Buffer, Edit, Editor, FontSystem, Metrics, Motion, Shaping, Wrap};

    #[test]
    fn the_mirrored_offset_matches_the_editor_after_every_action() {
        let mut fonts = FontSystem::new();
        let (size, row, view) = (14.0, line_height(14.0), 260.0);
        let lines = 300;
        let text: String = (0..lines).map(|i| format!("line {i} with some text\n")).collect();
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(size, row));
        buffer.set_wrap(&mut fonts, Wrap::None);
        buffer.set_size(&mut fonts, Some(500.0), Some(view));
        buffer.set_text(&mut fonts, &text, &Attrs::new(), Shaping::Advanced, None);
        let mut editor = Editor::new(buffer);
        let mut model = Scroll::default();
        let mut seen = Vec::new();

        enum Step {
            Act(Action),
            Wheel(i32),
        }
        use Step::{Act, Wheel};
        let mut steps = Vec::new();
        steps.extend((0..25).map(|_| Act(Action::Motion(Motion::Down))));
        steps.extend([Act(Action::Motion(Motion::PageDown)), Act(Action::Motion(Motion::PageDown))]);
        steps.extend([Wheel(7), Wheel(-3), Wheel(40), Act(Action::Insert('x'))]);
        steps.extend((0..12).map(|_| Act(Action::Motion(Motion::Up))));
        steps.extend([Act(Action::Motion(Motion::BufferEnd)), Wheel(-500), Wheel(2)]);
        steps.extend([Act(Action::Enter), Act(Action::Enter), Act(Action::Backspace)]);
        steps.extend([Act(Action::Motion(Motion::PageUp)), Act(Action::Motion(Motion::BufferStart)), Wheel(1000)]);

        for (i, step) in steps.into_iter().enumerate() {
            let mut e = editor.borrow_with(&mut fonts);
            let wheel = match step {
                Act(action) => {
                    e.action(action);
                    None
                }
                // iced: `Action::Scroll { lines }` → pixels = lines × line height.
                Wheel(n) => {
                    e.action(Action::Scroll { pixels: n as f32 * row });
                    Some(n)
                }
            };
            e.shape_as_needed(false);
            let cursor = e.cursor();
            let count = e.with_buffer(|b| b.lines.len());
            let real = e.with_buffer(|b| b.scroll().line as f32 * row + b.scroll().vertical);
            match wheel {
                Some(n) => model.scrolled(n, row, count, Some(view)),
                None => model.settle(Position { line: cursor.line, column: cursor.index }, row, count, Some(view)),
            }
            let mirrored = model.offset(row, count, view);
            assert!((real - mirrored).abs() < 0.01, "step {i}: editor at {real}, gutter at {mirrored}");
            seen.push(real);
        }
        // The steps really scroll: down, a partial last row, the bottom and back to the top.
        let max = (lines + 1) as f32 * row - view;
        assert!(seen.iter().any(|o| *o > 0.0 && (o / row).fract() > 0.01), "{seen:?}");
        assert!(seen.iter().any(|o| (o - max).abs() < 1.0 || *o > max - row), "{seen:?}");
    }
}
