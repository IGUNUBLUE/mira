//! The terminal widget: draws a [`Frame`](super::frame::Frame) and turns keys
//! and the mouse into [`Action`]s.
//!
//! Drawing: one quad per run of the same background, the cursor, lines and
//! block glyphs as quads (`boxdraw`), then the text: runs of plain ASCII with
//! the same style are one text each (the cell width is the font's own
//! advance, so they line up), anything else one text per cell. Within a
//! layer iced draws all quads before any text, so nothing covers a glyph and
//! the cursor shows on lines with their own background.

use super::frame::{Cell, Frame, Underline};
use super::input::{self, Button, MouseEvent};
use super::{Action, Pos, Terminal, boxdraw};

use alacritty_terminal::index::Side;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{CursorShape, Rgb};
use iced::advanced::layout::{self, Layout};
use iced::advanced::mouse::{self, Click};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::text::{self, Paragraph as _, Renderer as _};
use iced::advanced::widget::{Operation, Tree, operation, tree};
use iced::advanced::{Clipboard, Renderer as _, Shell, Widget, clipboard};
use iced::alignment::Vertical;
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::{Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Size};

/// Line height, relative to the font size (what iced_term used).
const LINE_HEIGHT: f32 = 1.3;

/// mira's shortcuts that must reach the app while the terminal has focus
/// (`Ctrl+Shift+<letter>`; never C/V, the terminal's copy and paste).
const APP_LETTERS: [&str; 7] = ["g", "f", "e", "m", "b", "t", "p"];

pub struct View<'a, M> {
    term: &'a Terminal,
    font: Font,
    size: f32,
    on_action: Box<dyn Fn(Action) -> M + 'a>,
}

/// The terminal, drawn with `font` at `size`, reporting with `on_action`.
pub fn view<'a, M: 'a>(
    term: &'a Terminal,
    font: Font,
    size: f32,
    on_action: impl Fn(Action) -> M + 'a,
) -> Element<'a, M> {
    Element::new(View { term, font, size, on_action: Box::new(on_action) })
}

#[derive(Default)]
struct State {
    focused: bool,
    /// Cell size, for the font size it was measured at.
    cell: Option<(Size, f32)>,
    /// What the terminal was last told: columns, lines, cell size.
    reported: Option<(usize, usize, Size)>,
    modifiers: Modifiers,
    /// A left drag is selecting.
    selecting: bool,
    last_click: Option<Click>,
    /// A button reported as pressed to the program, so its release is too.
    held: Option<Button>,
    last_motion: Option<Pos>,
    wheel_carry: f32,
    /// Cell whose link is underlined (Ctrl held).
    hover: Option<Pos>,
}

impl operation::Focusable for State {
    fn is_focused(&self) -> bool {
        self.focused
    }

    fn focus(&mut self) {
        self.focused = true;
    }

    fn unfocus(&mut self) {
        self.focused = false;
    }
}

fn color(c: Rgb) -> Color {
    Color::from_rgb8(c.r, c.g, c.b)
}

/// The cell size of `font` at `size`: the advance of its glyphs (fractional,
/// so runs of text line up with the grid) and the line height in pixels.
fn measure(font: Font, size: f32) -> Size {
    let paragraph = <iced::Renderer as text::Renderer>::Paragraph::with_text(iced::advanced::Text {
        content: "MMMMMMMMMM",
        bounds: Size::INFINITE,
        size: Pixels(size),
        line_height: text::LineHeight::Relative(1.0),
        font,
        align_x: text::Alignment::Left,
        align_y: Vertical::Top,
        shaping: text::Shaping::Basic,
        wrapping: text::Wrapping::None,
    });
    let width = paragraph.min_bounds().width / 10.0;
    Size::new(if width > 0.0 { width } else { size * 0.6 }, (size * LINE_HEIGHT).round())
}

/// The cell under `point`, clamped to the grid; and which half of it.
fn cell_at(bounds: Rectangle, cell: Size, cols: usize, lines: usize, point: Point) -> (Pos, Side) {
    let x = (point.x - bounds.x).max(0.0);
    let y = (point.y - bounds.y).max(0.0);
    let col = ((x / cell.width) as usize).min(cols.saturating_sub(1));
    let line = ((y / cell.height) as usize).min(lines.saturating_sub(1));
    let side = if x - col as f32 * cell.width > cell.width / 2.0 { Side::Right } else { Side::Left };
    (Pos { line, col }, side)
}

fn is_app_shortcut(key: &Key, mods: Modifiers) -> bool {
    let Key::Character(c) = key else { return false };
    let ctrl_shift = mods.control() && mods.shift() && !mods.alt();
    (ctrl_shift && APP_LETTERS.iter().any(|l| c.eq_ignore_ascii_case(l))) || (mods.control() && c.as_str() == "`")
}

fn ctrl_shift(key: &Key, mods: Modifiers, letter: &str) -> bool {
    mods.control() && mods.shift() && matches!(key, Key::Character(c) if c.eq_ignore_ascii_case(letter))
}

impl<M> View<'_, M> {
    /// What a key press does, or `None` to leave it to the app.
    fn on_key(
        &self,
        key: &Key,
        text: Option<&str>,
        mods: Modifiers,
        frame: &Frame,
        clip: &mut dyn Clipboard,
    ) -> Option<Option<Action>> {
        if is_app_shortcut(key, mods) {
            return None;
        }
        if ctrl_shift(key, mods, "c") {
            return Some(Some(Action::Copy));
        }
        let shift_insert = mods.shift() && matches!(key, Key::Named(Named::Insert));
        if ctrl_shift(key, mods, "v") || shift_insert {
            return Some(clip.read(clipboard::Kind::Standard).filter(|t| !t.is_empty()).map(Action::Paste));
        }
        // Shift+PageUp/Down/Home/End page through the history, unless a full-screen program runs.
        if mods.shift() && !mods.control() && !mods.alt() && !frame.mode.contains(TermMode::ALT_SCREEN) {
            let page = frame.lines.saturating_sub(1).max(1) as i32;
            let scroll = match key {
                Key::Named(Named::PageUp) => Some(Action::Scroll(page)),
                Key::Named(Named::PageDown) => Some(Action::Scroll(-page)),
                Key::Named(Named::Home) => Some(Action::ScrollToTop),
                Key::Named(Named::End) => Some(Action::ScrollToBottom),
                _ => None,
            };
            if scroll.is_some() {
                return Some(scroll);
            }
        }
        input::key(key, text, mods, frame.mode).map(|bytes| Some(Action::Input(bytes)))
    }

    fn report(&self, event: MouseEvent, pos: Pos, mods: Modifiers, mode: TermMode) -> Option<Action> {
        input::mouse_report(event, pos.line, pos.col, mods, mode).map(Action::Report)
    }
}

impl<M> Widget<M, iced::Theme, iced::Renderer> for View<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.resolve(Length::Fill, Length::Fill, Size::ZERO))
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        let state: &mut State = tree.state.downcast_mut();
        operation.focusable(Some(self.term.widget_id()), layout.bounds(), state);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        let state: &mut State = tree.state.downcast_mut();
        let bounds = layout.bounds();
        let cell = match state.cell {
            Some((cell, size)) if size == self.size => cell,
            _ => {
                let cell = measure(self.font, self.size);
                state.cell = Some((cell, self.size));
                cell
            }
        };
        let cols = ((bounds.width / cell.width).floor() as usize).max(1);
        let lines = ((bounds.height / cell.height).floor() as usize).max(1);
        let mut actions = Vec::new();
        if state.reported != Some((cols, lines, cell)) {
            state.reported = Some((cols, lines, cell));
            actions.push(Action::Resize { cols, lines, cell_width: cell.width, cell_height: cell.height });
        }

        let frame = self.term.frame();
        let mode = frame.mode;
        let mods = state.modifiers;
        // Shift lets the user select even while a program takes the mouse (herdr).
        let mouse_mode = mode.intersects(TermMode::MOUSE_MODE) && !mods.shift();
        let over = cursor.is_over(bounds);
        let at = |p: Point| cell_at(bounds, cell, cols, lines, p);
        let mut captured = false;

        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                state.modifiers = *m;
                if !m.control() && state.hover.take().is_some() {
                    actions.push(Action::Hover(None));
                } else if m.control()
                    && let Some(p) = cursor.position_over(bounds)
                {
                    let (pos, _) = at(p);
                    state.hover = Some(pos);
                    actions.push(Action::Hover(Some(pos)));
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, text, .. }) if state.focused => {
                if let Some(action) = self.on_key(key, text.as_deref(), *modifiers, &frame, clipboard) {
                    actions.extend(action);
                    captured = true;
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                if *button == mouse::Button::Left {
                    // Same rule as iced's inputs: a left click moves the focus.
                    state.focused = over;
                }
                if let (true, Some(p)) = (over, cursor.position()) {
                    let (pos, side) = at(p);
                    let b = match button {
                        mouse::Button::Left => Some(Button::Left),
                        mouse::Button::Middle => Some(Button::Middle),
                        mouse::Button::Right => Some(Button::Right),
                        _ => None,
                    };
                    match b {
                        Some(Button::Left) if mods.control() && self.term.hovered_link() => {
                            actions.push(Action::Open(pos));
                        }
                        Some(b) if mouse_mode => {
                            state.held = Some(b);
                            state.last_motion = Some(pos);
                            actions.extend(self.report(MouseEvent::Press(b), pos, mods, mode));
                        }
                        Some(Button::Left) => {
                            let click = Click::new(p, mouse::Button::Left, state.last_click);
                            let kind = match click.kind() {
                                mouse::click::Kind::Single => SelectionType::Simple,
                                mouse::click::Kind::Double => SelectionType::Semantic,
                                mouse::click::Kind::Triple => SelectionType::Lines,
                            };
                            state.last_click = Some(click);
                            state.selecting = true;
                            actions.push(Action::Select(kind, pos, side));
                        }
                        Some(Button::Middle) => {
                            // X11/Wayland habit: middle click pastes the primary selection.
                            actions.extend(
                                clipboard.read(clipboard::Kind::Primary).filter(|t| !t.is_empty()).map(Action::Paste),
                            );
                        }
                        _ => {}
                    }
                    captured = true;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(button)) => {
                let b = match button {
                    mouse::Button::Left => Some(Button::Left),
                    mouse::Button::Middle => Some(Button::Middle),
                    mouse::Button::Right => Some(Button::Right),
                    _ => None,
                };
                if b.is_some() && state.held == b {
                    state.held = None;
                    let pos =
                        cursor.position().map_or(state.last_motion.unwrap_or(Pos { line: 0, col: 0 }), |p| at(p).0);
                    if let Some(b) = b {
                        actions.extend(self.report(MouseEvent::Release(b), pos, mods, mode));
                    }
                    captured = true;
                }
                if *button == mouse::Button::Left && std::mem::take(&mut state.selecting) {
                    actions.push(Action::SelectDone);
                    captured = true;
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let (pos, side) = at(*position);
                let moved = state.last_motion != Some(pos);
                if let Some(b) = state.held {
                    if moved && mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
                        state.last_motion = Some(pos);
                        actions.extend(self.report(MouseEvent::Motion(Some(b)), pos, mods, mode));
                    }
                } else if mouse_mode && over && moved && mode.contains(TermMode::MOUSE_MOTION) {
                    state.last_motion = Some(pos);
                    actions.extend(self.report(MouseEvent::Motion(None), pos, mods, mode));
                }
                if state.selecting {
                    actions.push(Action::SelectTo(pos, side));
                }
                if mods.control() && over && state.hover != Some(pos) {
                    state.hover = Some(pos);
                    actions.push(Action::Hover(Some(pos)));
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if over => {
                let pos = cursor.position().map_or(Pos { line: 0, col: 0 }, |p| at(p).0);
                if mouse_mode {
                    // One report per notch; the program decides how far that is.
                    let n = input::wheel_lines(*delta, cell.height, &mut state.wheel_carry, 1);
                    let b = if n > 0 { Button::WheelUp } else { Button::WheelDown };
                    for _ in 0..n.unsigned_abs() {
                        actions.extend(self.report(MouseEvent::Press(b), pos, mods, mode));
                    }
                } else {
                    let n = input::wheel_lines(*delta, cell.height, &mut state.wheel_carry, input::WHEEL_LINES);
                    if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
                        // A full-screen program without mouse support: the wheel is the arrows.
                        let arrow: &[u8] = match (n > 0, mode.contains(TermMode::APP_CURSOR)) {
                            (true, true) => b"\x1bOA",
                            (true, false) => b"\x1b[A",
                            (false, true) => b"\x1bOB",
                            (false, false) => b"\x1b[B",
                        };
                        if n != 0 {
                            actions.push(Action::Report(arrow.repeat(n.unsigned_abs() as usize)));
                        }
                    } else if n != 0 {
                        actions.push(Action::Scroll(n));
                    }
                }
                captured = true;
            }
            _ => {}
        }
        drop(frame);
        for action in actions {
            shell.publish((self.on_action)(action));
        }
        if captured {
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let state: &State = tree.state.downcast_ref();
        if !cursor.is_over(layout.bounds()) {
            return mouse::Interaction::None;
        }
        if state.modifiers.control() && self.term.hovered_link() {
            return mouse::Interaction::Pointer;
        }
        let mode = self.term.frame().mode;
        if mode.intersects(TermMode::MOUSE_MODE) && !state.modifiers.shift() {
            mouse::Interaction::Idle
        } else {
            mouse::Interaction::Text
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state: &State = tree.state.downcast_ref();
        let bounds = layout.bounds();
        let frame = self.term.frame();
        let cell = state.cell.map_or_else(|| measure(self.font, self.size), |(c, _)| c);
        renderer.with_layer(bounds, |r| {
            fill(r, bounds, color(frame.background));
            for line in 0..frame.lines {
                let y = bounds.y + line as f32 * cell.height;
                if y >= bounds.y + bounds.height {
                    break;
                }
                let row = frame.row(line);
                let cursor = frame.cursor.filter(|c| c.line == line);
                if cursor.is_none() && row.iter().all(Cell::is_empty) {
                    continue;
                }
                backgrounds(r, row, bounds.x, y, cell);
                decorations(r, row, bounds.x, y, cell);
                if let Some(c) = cursor {
                    draw_cursor(r, c, state.focused, bounds.x, y, cell);
                }
                let block = cursor.filter(|c| state.focused && c.shape == CursorShape::Block);
                self.texts(r, row, bounds, y, cell, block.map(|c| (c.col, color(c.text))));
            }
            // Scrolled back into the history: where in it, at the right edge.
            if frame.display_offset > 0 && frame.history > 0 {
                let total = (frame.history + frame.lines) as f32;
                let h = (bounds.height * frame.lines as f32 / total).clamp(24.0_f32.min(bounds.height), bounds.height);
                // 0 at the oldest line, 1 at the bottom.
                let at = 1.0 - frame.display_offset.min(frame.history) as f32 / frame.history as f32;
                let y = bounds.y + (bounds.height - h) * at;
                let thumb = Rectangle::new(Point::new(bounds.x + bounds.width - 5.0, y), Size::new(4.0, h));
                r.fill_quad(
                    Quad { bounds: thumb, border: iced::Border::default().rounded(2), snap: true, ..Quad::default() },
                    Color { a: 0.6, ..crate::theme::p().line_no },
                );
            }
        });
    }
}

fn fill(r: &mut iced::Renderer, bounds: Rectangle, c: Color) {
    r.fill_quad(Quad { bounds, snap: true, ..Quad::default() }, c);
}

/// One quad per run of cells with the same (non-default) background.
fn backgrounds(r: &mut iced::Renderer, row: &[Cell], x0: f32, y: f32, cell: Size) {
    let mut col = 0;
    while col < row.len() {
        let Some(bg) = row[col].bg else {
            col += 1;
            continue;
        };
        let start = col;
        while col < row.len() && row[col].bg == Some(bg) {
            col += 1;
        }
        let x = x0 + start as f32 * cell.width;
        fill(r, Rectangle::new(Point::new(x, y), Size::new((col - start) as f32 * cell.width, cell.height)), color(bg));
    }
}

/// Underlines and strike-through, under the text.
fn decorations(r: &mut iced::Renderer, row: &[Cell], x0: f32, y: f32, cell: Size) {
    let thick = (cell.height / 16.0).round().max(1.0);
    for (col, c) in row.iter().enumerate() {
        if c.hidden || c.spacer || (c.underline == Underline::None && !c.strike) {
            continue;
        }
        let x = x0 + col as f32 * cell.width;
        let w = cell.width * if c.wide { 2.0 } else { 1.0 };
        let line_color = color(c.underline_color.unwrap_or(c.fg));
        let base = y + cell.height - thick * 2.0;
        let bar = |r: &mut iced::Renderer, x: f32, y: f32, w: f32, col: Color| {
            fill(r, Rectangle::new(Point::new(x, y), Size::new(w, thick)), col);
        };
        match c.underline {
            Underline::None => {}
            Underline::Single => bar(r, x, base, w, line_color),
            Underline::Double => {
                bar(r, x, base, w, line_color);
                bar(r, x, base - thick * 2.0, w, line_color);
            }
            // A curl at this size reads as a raised dashed line; dots and dashes as such.
            Underline::Curl | Underline::Dashed | Underline::Dotted => {
                let (on, gap) = match c.underline {
                    Underline::Dotted => (thick, thick),
                    Underline::Dashed => (cell.width / 2.0 - 1.0, 2.0),
                    _ => (cell.width / 4.0, cell.width / 4.0),
                };
                let mut dx = 0.0;
                let mut up = false;
                while dx < w {
                    let rise = if c.underline == Underline::Curl && up { thick } else { 0.0 };
                    bar(r, x + dx, base - rise, on.min(w - dx), line_color);
                    dx += on + gap;
                    up = !up;
                }
            }
        }
        if c.strike {
            bar(r, x, y + (cell.height / 2.0).round(), w, color(c.fg));
        }
    }
}

fn draw_cursor(r: &mut iced::Renderer, c: super::frame::Cursor, focused: bool, x0: f32, y: f32, cell: Size) {
    let x = x0 + c.col as f32 * cell.width;
    let w = cell.width * if c.wide { 2.0 } else { 1.0 };
    let thick = (cell.width / 8.0).round().max(2.0);
    let at = |x: f32, y: f32, w: f32, h: f32| Rectangle::new(Point::new(x, y), Size::new(w, h));
    let col = color(c.color);
    match (c.shape, focused) {
        (CursorShape::Hidden, _) => {}
        // Unfocused, any shape shows as an outline, as in other terminals.
        (CursorShape::HollowBlock, _) | (CursorShape::Block, false) => r.fill_quad(
            Quad {
                bounds: at(x, y, w, cell.height),
                border: iced::Border { color: col, width: 1.0, radius: 0.0.into() },
                snap: true,
                ..Quad::default()
            },
            Color::TRANSPARENT,
        ),
        (CursorShape::Block, true) => fill(r, at(x, y, w, cell.height), col),
        (CursorShape::Beam, _) => fill(r, at(x, y, thick, cell.height), col),
        (CursorShape::Underline, _) => fill(r, at(x, y + cell.height - thick, w, thick), col),
    }
}

fn is_plain(c: &Cell) -> bool {
    c.extra.is_none() && !c.wide && (' '..='~').contains(&c.c)
}

impl<M> View<'_, M> {
    fn font_for(&self, c: &Cell) -> Font {
        Font {
            weight: if c.bold { iced::font::Weight::Bold } else { self.font.weight },
            style: if c.italic { iced::font::Style::Italic } else { self.font.style },
            ..self.font
        }
    }

    fn text(
        &self,
        content: String,
        font: Font,
        shaping: text::Shaping,
        width: f32,
        cell: Size,
    ) -> iced::advanced::Text<String> {
        iced::advanced::Text {
            content,
            bounds: Size::new(width, cell.height),
            size: Pixels(self.size),
            line_height: text::LineHeight::Absolute(Pixels(cell.height)),
            font,
            align_x: text::Alignment::Left,
            align_y: Vertical::Top,
            shaping,
            wrapping: text::Wrapping::None,
        }
    }

    /// The text of a row: runs of plain ASCII in one style as one text, every
    /// other cell on its own (centered in its one or two cells). Block glyphs
    /// and box lines are quads. `block`: the block cursor's column and the
    /// color of the character under it.
    fn texts(
        &self,
        r: &mut iced::Renderer,
        row: &[Cell],
        bounds: Rectangle,
        y: f32,
        cell: Size,
        block: Option<(usize, Color)>,
    ) {
        let mut run = String::new();
        let mut run_start = 0;
        let mut run_style: Option<(Color, Font)> = None;
        let flush = |r: &mut iced::Renderer, run: &mut String, start: usize, style: Option<(Color, Font)>| {
            let Some((col, font)) = style else { return };
            let lead = run.len() - run.trim_start().len();
            let trimmed = run.trim();
            if !trimmed.is_empty() {
                let x = bounds.x + (start + lead) as f32 * cell.width;
                let width = (trimmed.len() + 1) as f32 * cell.width;
                r.fill_text(
                    self.text(trimmed.to_string(), font, text::Shaping::Basic, width, cell),
                    Point::new(x, y),
                    col,
                    bounds,
                );
            }
            run.clear();
        };
        for (i, c) in row.iter().enumerate() {
            if c.spacer || c.hidden {
                flush(r, &mut run, run_start, run_style.take());
                continue;
            }
            let fg = match block {
                Some((col, under)) if col == i => under,
                _ => color(c.fg),
            };
            if is_plain(c) {
                let style = (fg, self.font_for(c));
                if run_style != Some(style) {
                    flush(r, &mut run, run_start, run_style.take());
                    run_style = Some(style);
                    run_start = i;
                }
                run.push(c.c);
                continue;
            }
            flush(r, &mut run, run_start, run_style.take());
            let x = bounds.x + i as f32 * cell.width;
            if let Some(pieces) = boxdraw::pieces(c.c, cell.width, cell.height) {
                for p in pieces {
                    let rect = Rectangle::new(Point::new(x + p.x, y + p.y), Size::new(p.w, p.h));
                    fill(r, rect, Color { a: fg.a * p.alpha, ..fg });
                }
                continue;
            }
            let span = if c.wide { 2.0 } else { 1.0 };
            let mut content = c.c.to_string();
            if let Some(extra) = &c.extra {
                content.push_str(extra);
            }
            let t = iced::advanced::Text {
                align_x: text::Alignment::Center,
                align_y: Vertical::Center,
                ..self.text(content, self.font_for(c), text::Shaping::Advanced, cell.width * span, cell)
            };
            r.fill_text(t, Point::new(x + cell.width * span / 2.0, y + cell.height / 2.0), fg, bounds);
        }
        flush(r, &mut run, run_start, run_style.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_map_to_cells_and_halves() {
        let b = Rectangle { x: 100.0, y: 50.0, width: 80.0, height: 40.0 };
        let cell = Size::new(8.0, 20.0);
        assert_eq!(cell_at(b, cell, 10, 2, Point::new(100.0, 50.0)), (Pos { line: 0, col: 0 }, Side::Left));
        assert_eq!(cell_at(b, cell, 10, 2, Point::new(127.0, 71.0)), (Pos { line: 1, col: 3 }, Side::Left));
        assert_eq!(cell_at(b, cell, 10, 2, Point::new(129.0, 71.0)), (Pos { line: 1, col: 3 }, Side::Right));
        // Past the edges: clamped.
        assert_eq!(cell_at(b, cell, 10, 2, Point::new(500.0, 500.0)).0, Pos { line: 1, col: 9 });
        assert_eq!(cell_at(b, cell, 10, 2, Point::new(0.0, 0.0)).0, Pos { line: 0, col: 0 });
    }

    #[test]
    fn app_shortcuts_pass_through() {
        let cs = Modifiers::CTRL | Modifiers::SHIFT;
        assert!(is_app_shortcut(&Key::Character("G".into()), cs));
        assert!(is_app_shortcut(&Key::Character("p".into()), cs));
        assert!(is_app_shortcut(&Key::Character("`".into()), Modifiers::CTRL));
        // Copy/paste and plain Ctrl+P stay with the terminal.
        assert!(!is_app_shortcut(&Key::Character("c".into()), cs));
        assert!(!is_app_shortcut(&Key::Character("p".into()), Modifiers::CTRL));
    }
}
