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

/// Line height, relative to the font size.
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
    /// A left drag is selecting; where its end is now.
    selecting: Option<(Pos, Side)>,
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

/// What the input logic needs to know about the terminal and its place.
struct Ctx {
    bounds: Rectangle,
    cell: Size,
    cols: usize,
    lines: usize,
    mode: TermMode,
    /// Ctrl is over a link (Ctrl+click opens it).
    on_link: bool,
}

impl Ctx {
    fn at(&self, p: Point) -> (Pos, Side) {
        cell_at(self.bounds, self.cell, self.cols, self.lines, p)
    }
}

fn report(event: MouseEvent, pos: Pos, mods: Modifiers, mode: TermMode) -> Option<Action> {
    input::mouse_report(event, pos.line, pos.col, mods, mode).map(Action::Report)
}

fn button_of(b: mouse::Button) -> Option<Button> {
    match b {
        mouse::Button::Left => Some(Button::Left),
        mouse::Button::Middle => Some(Button::Middle),
        mouse::Button::Right => Some(Button::Right),
        _ => None,
    }
}

/// What a key press does: `None` leaves it to the app (its shortcuts, or
/// nothing to send). `paste` reads the clipboard.
fn on_key(
    key: &Key,
    text: Option<&str>,
    mods: Modifiers,
    ctx: &Ctx,
    paste: &mut dyn FnMut(clipboard::Kind) -> Option<String>,
) -> Option<Option<Action>> {
    if is_app_shortcut(key, mods) {
        return None;
    }
    if ctrl_shift(key, mods, "c") {
        return Some(Some(Action::Copy));
    }
    let shift_insert = mods.shift() && matches!(key, Key::Named(Named::Insert));
    if ctrl_shift(key, mods, "v") || shift_insert {
        return Some(paste(clipboard::Kind::Standard).filter(|t| !t.is_empty()).map(Action::Paste));
    }
    // Shift+PageUp/Down/Home/End page through the history, unless a full-screen program runs.
    if mods.shift() && !mods.control() && !mods.alt() && !ctx.mode.contains(TermMode::ALT_SCREEN) {
        let page = ctx.lines.saturating_sub(1).max(1) as i32;
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
    input::key(key, text, mods, ctx.mode).map(|bytes| Some(Action::Input(bytes)))
}

/// The actions for one event, and whether the terminal takes it. Pure: the
/// widget only measures, calls this and publishes.
fn handle(
    state: &mut State,
    event: &Event,
    cursor: mouse::Cursor,
    ctx: &Ctx,
    paste: &mut dyn FnMut(clipboard::Kind) -> Option<String>,
) -> (Vec<Action>, bool) {
    let mut actions = Vec::new();
    let mut captured = false;
    let mode = ctx.mode;
    let mods = state.modifiers;
    // Shift lets the user select even while a program takes the mouse (herdr).
    let mouse_mode = mode.intersects(TermMode::MOUSE_MODE) && !mods.shift();
    let over = cursor.is_over(ctx.bounds);
    match event {
        Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
            state.modifiers = *m;
            if !m.control() && state.hover.take().is_some() {
                actions.push(Action::Hover(None));
            } else if m.control()
                && let Some(p) = cursor.position_over(ctx.bounds)
            {
                let (pos, _) = ctx.at(p);
                state.hover = Some(pos);
                actions.push(Action::Hover(Some(pos)));
            }
        }
        Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, text, .. }) if state.focused => {
            if let Some(action) = on_key(key, text.as_deref(), *modifiers, ctx, paste) {
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
                let (pos, side) = ctx.at(p);
                match button_of(*button) {
                    Some(Button::Left) if mods.control() && ctx.on_link => actions.push(Action::Open(pos)),
                    Some(b) if mouse_mode => {
                        state.held = Some(b);
                        state.last_motion = Some(pos);
                        actions.extend(report(MouseEvent::Press(b), pos, mods, mode));
                    }
                    Some(Button::Left) => {
                        let click = Click::new(p, mouse::Button::Left, state.last_click);
                        let kind = match click.kind() {
                            mouse::click::Kind::Single => SelectionType::Simple,
                            mouse::click::Kind::Double => SelectionType::Semantic,
                            mouse::click::Kind::Triple => SelectionType::Lines,
                        };
                        state.last_click = Some(click);
                        state.selecting = Some((pos, side));
                        actions.push(Action::Select(kind, pos, side));
                    }
                    // X11/Wayland habit: middle click pastes the primary selection.
                    Some(Button::Middle) => {
                        actions.extend(paste(clipboard::Kind::Primary).filter(|t| !t.is_empty()).map(Action::Paste));
                    }
                    _ => {}
                }
                captured = true;
            }
        }
        Event::Mouse(mouse::Event::ButtonReleased(button)) => {
            let b = button_of(*button);
            if b.is_some() && state.held == b {
                state.held = None;
                let pos =
                    cursor.position().map_or(state.last_motion.unwrap_or(Pos { line: 0, col: 0 }), |p| ctx.at(p).0);
                if let Some(b) = b {
                    actions.extend(report(MouseEvent::Release(b), pos, mods, mode));
                }
                captured = true;
            }
            if *button == mouse::Button::Left && state.selecting.take().is_some() {
                actions.push(Action::SelectDone);
                captured = true;
            }
        }
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            let (pos, side) = ctx.at(*position);
            let moved = state.last_motion != Some(pos);
            if let Some(b) = state.held {
                if moved && mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
                    state.last_motion = Some(pos);
                    actions.extend(report(MouseEvent::Motion(Some(b)), pos, mods, mode));
                }
            } else if mouse_mode && over && moved && mode.contains(TermMode::MOUSE_MOTION) {
                state.last_motion = Some(pos);
                actions.extend(report(MouseEvent::Motion(None), pos, mods, mode));
            }
            // Only when the selection's end really moves (a cell, or its half).
            if let Some(end) = state.selecting.as_mut().filter(|end| **end != (pos, side)) {
                *end = (pos, side);
                actions.push(Action::SelectTo(pos, side));
            }
            if mods.control() && over && state.hover != Some(pos) {
                state.hover = Some(pos);
                actions.push(Action::Hover(Some(pos)));
            }
        }
        Event::Mouse(mouse::Event::WheelScrolled { delta }) if over => {
            let pos = cursor.position().map_or(Pos { line: 0, col: 0 }, |p| ctx.at(p).0);
            if mouse_mode {
                // One report per notch; the program decides how far that is.
                let n = input::wheel_lines(*delta, ctx.cell.height, &mut state.wheel_carry, 1);
                let b = if n > 0 { Button::WheelUp } else { Button::WheelDown };
                for _ in 0..n.unsigned_abs() {
                    actions.extend(report(MouseEvent::Press(b), pos, mods, mode));
                }
            } else {
                let n = input::wheel_lines(*delta, ctx.cell.height, &mut state.wheel_carry, input::WHEEL_LINES);
                if n != 0 && mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
                    // A full-screen program without mouse support: the wheel is the arrows.
                    let arrow: &[u8] = match (n > 0, mode.contains(TermMode::APP_CURSOR)) {
                        (true, true) => b"\x1bOA",
                        (true, false) => b"\x1b[A",
                        (false, true) => b"\x1bOB",
                        (false, false) => b"\x1b[B",
                    };
                    actions.push(Action::Report(arrow.repeat(n.unsigned_abs() as usize)));
                } else if n != 0 {
                    actions.push(Action::Scroll(n));
                }
            }
            captured = true;
        }
        _ => {}
    }
    (actions, captured)
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

        let ctx = Ctx { bounds, cell, cols, lines, mode: self.term.frame().mode, on_link: self.term.hovered_link() };
        let (more, captured) = handle(state, event, cursor, &ctx, &mut |kind| clipboard.read(kind));
        actions.extend(more);
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
        let look = Look { font: self.font, size: self.size, cell, focused: state.focused };
        renderer.with_layer(bounds, |r| paint(&mut Gpu { r, clip: bounds }, &frame, bounds, &look));
    }
}

/// What the terminal is drawn with: everything `paint` does goes through it,
/// so tests can record it.
trait Painter {
    fn quad(&mut self, rect: Rectangle, color: Color);
    /// A 1 px border, nothing inside (the unfocused cursor).
    fn outline(&mut self, rect: Rectangle, color: Color);
    /// A filled rectangle with rounded corners (the history thumb).
    fn rounded(&mut self, rect: Rectangle, color: Color, radius: f32);
    fn text(&mut self, text: iced::advanced::Text<String>, at: Point, color: Color);
}

struct Gpu<'a> {
    r: &'a mut iced::Renderer,
    clip: Rectangle,
}

impl Painter for Gpu<'_> {
    fn quad(&mut self, rect: Rectangle, color: Color) {
        self.r.fill_quad(Quad { bounds: rect, snap: true, ..Quad::default() }, color);
    }

    fn outline(&mut self, rect: Rectangle, color: Color) {
        let border = iced::Border { color, width: 1.0, radius: 0.0.into() };
        self.r.fill_quad(Quad { bounds: rect, border, snap: true, ..Quad::default() }, Color::TRANSPARENT);
    }

    fn rounded(&mut self, rect: Rectangle, color: Color, radius: f32) {
        let border = iced::Border::default().rounded(radius);
        self.r.fill_quad(Quad { bounds: rect, border, snap: true, ..Quad::default() }, color);
    }

    fn text(&mut self, text: iced::advanced::Text<String>, at: Point, color: Color) {
        self.r.fill_text(text, at, color, self.clip);
    }
}

/// How to draw: font, size, cell and whether the terminal has the keyboard.
struct Look {
    font: Font,
    size: f32,
    cell: Size,
    focused: bool,
}

/// Draw `frame` in `bounds`: the background, then per row its background
/// runs, decorations, the cursor and the text; blank rows are skipped. Within
/// a layer iced draws all quads before any text.
fn paint(p: &mut dyn Painter, frame: &Frame, bounds: Rectangle, look: &Look) {
    let cell = look.cell;
    p.quad(bounds, color(frame.background));
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
        backgrounds(p, row, bounds.x, y, cell);
        decorations(p, row, bounds.x, y, cell);
        if let Some(c) = cursor {
            draw_cursor(p, c, look.focused, bounds.x, y, cell);
        }
        let block = cursor.filter(|c| look.focused && c.shape == CursorShape::Block);
        texts(p, row, bounds.x, y, look, block.map(|c| (c.col, c.text)));
    }
    if let Some(thumb) = history_thumb(frame, bounds) {
        p.rounded(thumb, Color { a: 0.6, ..crate::theme::p().line_no }, 2.0);
    }
}

/// Scrolled back into the history: where in it, as a thumb at the right edge.
fn history_thumb(frame: &Frame, bounds: Rectangle) -> Option<Rectangle> {
    if frame.display_offset == 0 || frame.history == 0 {
        return None;
    }
    let total = (frame.history + frame.lines) as f32;
    let h = (bounds.height * frame.lines as f32 / total).clamp(24.0_f32.min(bounds.height), bounds.height);
    // 0 at the oldest line, 1 at the bottom.
    let at = 1.0 - frame.display_offset.min(frame.history) as f32 / frame.history as f32;
    let y = bounds.y + (bounds.height - h) * at;
    Some(Rectangle::new(Point::new(bounds.x + bounds.width - 5.0, y), Size::new(4.0, h)))
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rectangle {
    Rectangle::new(Point::new(x, y), Size::new(w, h))
}

/// One quad per run of cells with the same (non-default) background.
fn backgrounds(p: &mut dyn Painter, row: &[Cell], x0: f32, y: f32, cell: Size) {
    for (start, len, bg) in bg_runs(row) {
        p.quad(rect(x0 + start as f32 * cell.width, y, len as f32 * cell.width, cell.height), color(bg));
    }
}

/// `(first column, cells, color)` of each run of one background color.
fn bg_runs(row: &[Cell]) -> Vec<(usize, usize, Rgb)> {
    let mut out = Vec::new();
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
        out.push((start, col - start, bg));
    }
    out
}

/// Underlines and strike-through, under the text.
fn decorations(p: &mut dyn Painter, row: &[Cell], x0: f32, y: f32, cell: Size) {
    let thick = (cell.height / 16.0).round().max(1.0);
    for (col, c) in row.iter().enumerate() {
        if c.hidden || c.spacer || (c.underline == Underline::None && !c.strike) {
            continue;
        }
        let x = x0 + col as f32 * cell.width;
        let w = cell.width * if c.wide { 2.0 } else { 1.0 };
        let line_color = color(c.underline_color.unwrap_or(c.fg));
        let base = y + cell.height - thick * 2.0;
        let mut bar = |x: f32, y: f32, w: f32, col: Color| p.quad(rect(x, y, w, thick), col);
        match c.underline {
            Underline::None => {}
            Underline::Single => bar(x, base, w, line_color),
            Underline::Double => {
                bar(x, base, w, line_color);
                bar(x, base - thick * 2.0, w, line_color);
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
                    bar(x + dx, base - rise, on.min(w - dx), line_color);
                    dx += on + gap;
                    up = !up;
                }
            }
        }
        if c.strike {
            bar(x, y + (cell.height / 2.0).round(), w, color(c.fg));
        }
    }
}

fn draw_cursor(p: &mut dyn Painter, c: super::frame::Cursor, focused: bool, x0: f32, y: f32, cell: Size) {
    let x = x0 + c.col as f32 * cell.width;
    let w = cell.width * if c.wide { 2.0 } else { 1.0 };
    let thick = (cell.width / 8.0).round().max(2.0);
    let col = color(c.color);
    match (c.shape, focused) {
        (CursorShape::Hidden, _) => {}
        // Unfocused, any shape shows as an outline, as in other terminals.
        (CursorShape::HollowBlock, _) | (CursorShape::Block, false) => p.outline(rect(x, y, w, cell.height), col),
        (CursorShape::Block, true) => p.quad(rect(x, y, w, cell.height), col),
        (CursorShape::Beam, _) => p.quad(rect(x, y, thick, cell.height), col),
        (CursorShape::Underline, _) => p.quad(rect(x, y + cell.height - thick, w, thick), col),
    }
}

fn is_plain(c: &Cell) -> bool {
    c.extra.is_none() && !c.wide && (' '..='~').contains(&c.c)
}

fn text_spec(
    content: String,
    font: Font,
    shaping: text::Shaping,
    width: f32,
    look: &Look,
) -> iced::advanced::Text<String> {
    iced::advanced::Text {
        content,
        bounds: Size::new(width, look.cell.height),
        size: Pixels(look.size),
        line_height: text::LineHeight::Absolute(Pixels(look.cell.height)),
        font,
        align_x: text::Alignment::Left,
        align_y: Vertical::Top,
        shaping,
        wrapping: text::Wrapping::None,
    }
}

/// Draw a row's text (see [`glyphs`]).
fn texts(p: &mut dyn Painter, row: &[Cell], x0: f32, y: f32, look: &Look, block: Option<(usize, Rgb)>) {
    let cell = look.cell;
    for g in glyphs(row, block) {
        let x = x0 + g.col as f32 * cell.width;
        let fg = color(g.fg);
        let font = Font {
            weight: if g.bold { iced::font::Weight::Bold } else { look.font.weight },
            style: if g.italic { iced::font::Style::Italic } else { look.font.style },
            ..look.font
        };
        match g.kind {
            GlyphKind::Run(run) => {
                let width = (run.len() + 1) as f32 * cell.width;
                p.text(text_spec(run, font, text::Shaping::Basic, width, look), Point::new(x, y), fg);
            }
            GlyphKind::Shape(c) => {
                for piece in boxdraw::pieces(c, cell.width, cell.height).unwrap_or_default() {
                    p.quad(rect(x + piece.x, y + piece.y, piece.w, piece.h), Color { a: fg.a * piece.alpha, ..fg });
                }
            }
            GlyphKind::Single { content, wide } => {
                let span = if wide { 2.0 } else { 1.0 };
                let t = iced::advanced::Text {
                    align_x: text::Alignment::Center,
                    align_y: Vertical::Center,
                    ..text_spec(content, font, text::Shaping::Advanced, cell.width * span, look)
                };
                p.text(t, Point::new(x + cell.width * span / 2.0, y + cell.height / 2.0), fg);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum GlyphKind {
    /// Plain ASCII in one style, drawn as one text: the cell width is the
    /// font's advance, so it lines up with the grid.
    Run(String),
    /// Anything else, centered in its one or two cells.
    Single { content: String, wide: bool },
    /// A box line or block, drawn as quads (`boxdraw`).
    Shape(char),
}

#[derive(Debug, Clone, PartialEq)]
struct Glyph {
    col: usize,
    kind: GlyphKind,
    fg: Rgb,
    bold: bool,
    italic: bool,
}

/// What to draw for the text of `row`. `block`: the block cursor's column and
/// the color of the character under it.
fn glyphs(row: &[Cell], block: Option<(usize, Rgb)>) -> Vec<Glyph> {
    let mut out: Vec<Glyph> = Vec::new();
    // The run being collected: (first column, text, fg, bold, italic).
    let mut run: Option<Glyph> = None;
    let flush = |run: &mut Option<Glyph>, out: &mut Vec<Glyph>| {
        let Some(mut g) = run.take() else { return };
        let GlyphKind::Run(text) = &g.kind else { return };
        // Spaces cost nothing to skip: the run starts at its first letter.
        let lead = text.len() - text.trim_start().len();
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            g.col += lead;
            g.kind = GlyphKind::Run(trimmed.to_string());
            out.push(g);
        }
    };
    for (i, c) in row.iter().enumerate() {
        if c.spacer || c.hidden {
            flush(&mut run, &mut out);
            continue;
        }
        let fg = match block {
            Some((col, under)) if col == i => under,
            _ => c.fg,
        };
        if is_plain(c) {
            match &mut run {
                Some(g) if (g.fg, g.bold, g.italic) == (fg, c.bold, c.italic) => {
                    if let GlyphKind::Run(text) = &mut g.kind {
                        text.push(c.c);
                    }
                }
                _ => {
                    flush(&mut run, &mut out);
                    run = Some(Glyph {
                        col: i,
                        kind: GlyphKind::Run(c.c.to_string()),
                        fg,
                        bold: c.bold,
                        italic: c.italic,
                    });
                }
            }
            continue;
        }
        flush(&mut run, &mut out);
        let kind = if boxdraw::pieces(c.c, 1.0, 1.0).is_some() {
            GlyphKind::Shape(c.c)
        } else {
            let mut content = c.c.to_string();
            if let Some(extra) = &c.extra {
                content.push_str(extra);
            }
            GlyphKind::Single { content, wide: c.wide }
        };
        out.push(Glyph { col: i, kind, fg, bold: c.bold, italic: c.italic });
    }
    flush(&mut run, &mut out);
    out
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

    const B: Rectangle = Rectangle { x: 0.0, y: 0.0, width: 80.0, height: 40.0 };

    fn ctx(mode: TermMode) -> Ctx {
        Ctx { bounds: B, cell: Size::new(8.0, 20.0), cols: 10, lines: 2, mode, on_link: false }
    }

    fn at(x: f32, y: f32) -> mouse::Cursor {
        mouse::Cursor::Available(Point::new(x, y))
    }

    fn press(b: mouse::Button) -> Event {
        Event::Mouse(mouse::Event::ButtonPressed(b))
    }

    fn release(b: mouse::Button) -> Event {
        Event::Mouse(mouse::Event::ButtonReleased(b))
    }

    fn moved(x: f32, y: f32) -> Event {
        Event::Mouse(mouse::Event::CursorMoved { position: Point::new(x, y) })
    }

    fn mods(m: Modifiers) -> Event {
        Event::Keyboard(keyboard::Event::ModifiersChanged(m))
    }

    fn run(state: &mut State, event: Event, cursor: mouse::Cursor, ctx: &Ctx) -> (Vec<Action>, bool) {
        handle(state, &event, cursor, ctx, &mut |kind| Some(format!("{kind:?}")))
    }

    fn reports(actions: &[Action]) -> Vec<String> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Report(b) => Some(String::from_utf8_lossy(b).into_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_click_focuses_and_a_drag_selects_once_per_cell() {
        let (mut s, c) = (State::default(), ctx(TermMode::empty()));
        let (a, captured) = run(&mut s, press(mouse::Button::Left), at(9.0, 5.0), &c);
        assert!(s.focused && captured);
        assert!(matches!(a.as_slice(), [Action::Select(SelectionType::Simple, Pos { line: 0, col: 1 }, Side::Left)]));
        // Moving within the same half of a cell selects nothing new.
        assert!(run(&mut s, moved(10.0, 5.0), at(10.0, 5.0), &c).0.is_empty());
        let (a, _) = run(&mut s, moved(30.0, 25.0), at(30.0, 25.0), &c);
        assert!(matches!(a.as_slice(), [Action::SelectTo(Pos { line: 1, col: 3 }, Side::Right)]));
        let (a, _) = run(&mut s, release(mouse::Button::Left), at(30.0, 25.0), &c);
        assert!(matches!(a.as_slice(), [Action::SelectDone]));
        // A click outside takes the focus away.
        run(&mut s, press(mouse::Button::Left), at(500.0, 5.0), &c);
        assert!(!s.focused);
    }

    #[test]
    fn a_mouse_program_gets_reports_unless_shift_is_held() {
        let (mut s, c) = (State::default(), ctx(TermMode::SGR_MOUSE | TermMode::MOUSE_DRAG));
        let (a, _) = run(&mut s, press(mouse::Button::Left), at(9.0, 5.0), &c);
        assert_eq!(reports(&a), ["\x1b[<0;2;1M"]);
        let (a, _) = run(&mut s, moved(17.0, 5.0), at(17.0, 5.0), &c);
        assert_eq!(reports(&a), ["\x1b[<32;3;1M"]);
        // The release is reported even outside the terminal.
        let (a, _) = run(&mut s, release(mouse::Button::Left), at(500.0, 500.0), &c);
        assert_eq!(reports(&a), ["\x1b[<0;10;2m"]);
        // Shift: mira selects instead.
        run(&mut s, mods(Modifiers::SHIFT), at(9.0, 5.0), &c);
        let (a, _) = run(&mut s, press(mouse::Button::Left), at(9.0, 5.0), &c);
        assert!(matches!(a.as_slice(), [Action::Select(..)]));
    }

    #[test]
    fn the_wheel_scrolls_history_or_becomes_reports_or_arrows() {
        let wheel = |y| Event::Mouse(mouse::Event::WheelScrolled { delta: mouse::ScrollDelta::Lines { x: 0.0, y } });
        let mut s = State::default();
        let (a, _) = run(&mut s, wheel(1.0), at(5.0, 5.0), &ctx(TermMode::empty()));
        assert!(matches!(a.as_slice(), [Action::Scroll(3)]));
        let (a, _) = run(&mut s, wheel(-2.0), at(5.0, 5.0), &ctx(TermMode::SGR_MOUSE | TermMode::MOUSE_REPORT_CLICK));
        assert_eq!(reports(&a), ["\x1b[<65;1;1M", "\x1b[<65;1;1M"]);
        let pager = TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL;
        let (a, _) = run(&mut s, wheel(1.0), at(5.0, 5.0), &ctx(pager));
        assert_eq!(reports(&a), ["\x1b[A\x1b[A\x1b[A"]);
        // Not over the terminal: left to whatever is there.
        let (a, captured) = run(&mut s, wheel(1.0), at(500.0, 5.0), &ctx(TermMode::empty()));
        assert!(a.is_empty() && !captured);
    }

    #[test]
    fn keys_paste_copy_and_page_only_while_focused() {
        let key = |k: Key, m: Modifiers| {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: k.clone(),
                modified_key: k,
                physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
                location: keyboard::Location::Standard,
                modifiers: m,
                text: None,
                repeat: false,
            })
        };
        let c = ctx(TermMode::empty());
        let mut s = State::default();
        assert!(run(&mut s, key(Key::Character("a".into()), Modifiers::empty()), at(5.0, 5.0), &c).0.is_empty());
        s.focused = true;
        let cs = Modifiers::CTRL | Modifiers::SHIFT;
        let (a, _) = run(&mut s, key(Key::Character("V".into()), cs), at(5.0, 5.0), &c);
        assert!(matches!(a.as_slice(), [Action::Paste(t)] if t == "Standard"));
        let (a, _) = run(&mut s, key(Key::Named(Named::Insert), Modifiers::SHIFT), at(5.0, 5.0), &c);
        assert!(matches!(a.as_slice(), [Action::Paste(_)]));
        assert!(matches!(
            run(&mut s, key(Key::Character("c".into()), cs), at(5.0, 5.0), &c).0.as_slice(),
            [Action::Copy]
        ));
        let (a, _) = run(&mut s, key(Key::Named(Named::PageUp), Modifiers::SHIFT), at(5.0, 5.0), &c);
        assert!(matches!(a.as_slice(), [Action::Scroll(1)]));
        // An app shortcut is not taken.
        let (a, captured) = run(&mut s, key(Key::Character("g".into()), cs), at(5.0, 5.0), &c);
        assert!(a.is_empty() && !captured);
        // Middle click pastes the primary selection.
        let (a, _) = run(&mut s, press(mouse::Button::Middle), at(5.0, 5.0), &c);
        assert!(matches!(a.as_slice(), [Action::Paste(t)] if t == "Primary"));
    }

    #[test]
    fn ctrl_hovers_and_opens_links() {
        let mut c = ctx(TermMode::SGR_MOUSE);
        let mut s = State::default();
        let (a, _) = run(&mut s, mods(Modifiers::CTRL), at(9.0, 5.0), &c);
        assert!(matches!(a.as_slice(), [Action::Hover(Some(Pos { line: 0, col: 1 }))]));
        c.on_link = true;
        // Even with a mouse program, Ctrl+click on a link opens it.
        let (a, _) = run(&mut s, press(mouse::Button::Left), at(9.0, 5.0), &c);
        assert!(matches!(a.as_slice(), [Action::Open(Pos { line: 0, col: 1 })]));
        let (a, _) = run(&mut s, mods(Modifiers::empty()), at(9.0, 5.0), &c);
        assert!(matches!(a.as_slice(), [Action::Hover(None)]));
    }

    fn row_of(input: &str) -> Vec<Cell> {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::term::{Config, Term, test::TermSize};
        let mut term = Term::new(Config::default(), &TermSize::new(16, 1), VoidListener);
        let mut parser: alacritty_terminal::vte::ansi::Processor = alacritty_terminal::vte::ansi::Processor::new();
        parser.advance(&mut term, input.as_bytes());
        super::super::frame::build(&term, &crate::theme::PALETTES[0], &[]).row(0).to_vec()
    }

    fn kinds(g: &[Glyph]) -> Vec<(usize, GlyphKind)> {
        g.iter().map(|g| (g.col, g.kind.clone())).collect()
    }

    #[test]
    fn text_runs_split_on_style_and_non_ascii() {
        let row = row_of("  ab cd\x1b[1mef\x1b[0m日x│é");
        let run = |s: &str| GlyphKind::Run(s.into());
        assert_eq!(
            kinds(&glyphs(&row, None)),
            [
                (2, run("ab cd")),
                (7, run("ef")),
                (9, GlyphKind::Single { content: "日".into(), wide: true }),
                (11, run("x")),
                (12, GlyphKind::Shape('│')),
                (13, GlyphKind::Single { content: "é".into(), wide: false }),
            ]
        );
        assert!(glyphs(&row, None)[1].bold);
        // The block cursor's cell is its own run, in the cursor's text color.
        let under = Rgb { r: 1, g: 2, b: 3 };
        let g = glyphs(&row_of("abc"), Some((1, under)));
        assert_eq!(kinds(&g), [(0, run("a")), (1, run("b")), (2, run("c"))]);
        assert_eq!(g[1].fg, under);
        assert!(glyphs(&row_of("     "), None).is_empty());
    }

    #[test]
    fn backgrounds_merge_into_runs() {
        let row = row_of("a\x1b[41mbcd\x1b[42mef\x1b[0mg\x1b[41mh");
        let runs: Vec<(usize, usize)> = bg_runs(&row).iter().map(|(s, l, _)| (*s, *l)).collect();
        assert_eq!(runs, [(1, 3), (4, 2), (7, 1)]);
    }

    #[derive(Debug, Clone, PartialEq)]
    enum Op {
        Quad(Rectangle, Color),
        Outline(Rectangle, Color),
        Rounded(Rectangle),
        Text(String, Point, Color, bool),
    }

    #[derive(Default)]
    struct Recorder(Vec<Op>);

    impl Painter for Recorder {
        fn quad(&mut self, rect: Rectangle, color: Color) {
            self.0.push(Op::Quad(rect, color));
        }
        fn outline(&mut self, rect: Rectangle, color: Color) {
            self.0.push(Op::Outline(rect, color));
        }
        fn rounded(&mut self, rect: Rectangle, _color: Color, _radius: f32) {
            self.0.push(Op::Rounded(rect));
        }
        fn text(&mut self, text: iced::advanced::Text<String>, at: Point, color: Color) {
            self.0.push(Op::Text(text.content, at, color, text.font.weight == iced::font::Weight::Bold));
        }
    }

    fn frame_of(cols: usize, lines: usize, input: &str) -> Frame {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::term::{Config, Term, test::TermSize};
        let mut term = Term::new(Config::default(), &TermSize::new(cols, lines), VoidListener);
        let mut parser: alacritty_terminal::vte::ansi::Processor = alacritty_terminal::vte::ansi::Processor::new();
        parser.advance(&mut term, input.as_bytes());
        super::super::frame::build(&term, &crate::theme::PALETTES[0], &[])
    }

    const CELL: Size = Size { width: 8.0, height: 20.0 };

    fn painted(f: &Frame, focused: bool) -> Vec<Op> {
        let look = Look { font: Font::MONOSPACE, size: 14.0, cell: CELL, focused };
        let bounds = rect(100.0, 10.0, f.cols as f32 * 8.0, f.lines as f32 * 20.0);
        let mut r = Recorder::default();
        paint(&mut r, f, bounds, &look);
        r.0
    }

    #[test]
    fn paints_background_runs_cursor_then_text_and_skips_blank_rows() {
        let f = frame_of(10, 3, "\x1b[48;2;1;2;3m> hi\x1b[K\r\n\r\n\x1b[1mok");
        let ops = painted(&f, true);
        let bg = color(f.background);
        let line_bg = Color::from_rgb8(1, 2, 3);
        // The terminal's background, then row 0's own background over the whole line.
        assert_eq!(ops[0], Op::Quad(rect(100.0, 10.0, 80.0, 60.0), bg));
        assert_eq!(ops[1], Op::Quad(rect(100.0, 10.0, 80.0, 20.0), line_bg));
        // Row 1 is blank: nothing for it. The cursor is on row 2, after "ok".
        let cursor =
            ops.iter().position(|o| matches!(o, Op::Quad(r, _) if *r == rect(116.0, 50.0, 8.0, 20.0))).unwrap();
        let ok = ops.iter().position(|o| matches!(o, Op::Text(t, _, _, true) if t == "ok")).unwrap();
        assert!(cursor < ok, "the cursor is drawn before the text over it: {ops:?}");
        assert!(
            ops.iter().any(|o| matches!(o, Op::Text(t, p, _, false) if t == "> hi" && *p == Point::new(100.0, 10.0)))
        );
        assert!(!ops.iter().any(|o| matches!(o, Op::Rounded(_))), "no history thumb at the bottom");
    }

    #[test]
    fn the_block_cursor_shows_on_a_colored_line_and_its_letter_takes_the_line_color() {
        // The cursor on the 'x' of a line with its own background (Codex's prompt, moved left).
        let f = frame_of(10, 1, "\x1b[48;2;229;236;237m> xy\x1b[K\x1b[2D");
        let ops = painted(&f, true);
        let c = f.cursor.unwrap();
        assert_eq!(c.col, 2);
        let at = rect(116.0, 10.0, 8.0, 20.0);
        let line_bg = Op::Quad(rect(100.0, 10.0, 80.0, 20.0), Color::from_rgb8(229, 236, 237));
        let line = ops.iter().position(|o| *o == line_bg).unwrap();
        let cursor = ops.iter().position(|o| *o == Op::Quad(at, color(c.color))).unwrap();
        assert!(line < cursor, "the cursor is painted over the line's background");
        assert!(ops.contains(&Op::Text("x".into(), Point::new(116.0, 10.0), Color::from_rgb8(229, 236, 237), false)));
        // Unfocused: an outline, and the letter keeps its color.
        let ops = painted(&f, false);
        assert!(ops.contains(&Op::Outline(at, color(c.color))));
        assert!(ops.iter().any(|o| matches!(o, Op::Text(t, _, _, _) if t == "> xy")));
    }

    #[test]
    fn cursor_shapes_and_decorations() {
        let beam = painted(&frame_of(4, 1, "\x1b[6 q"), true);
        assert!(
            beam.contains(&Op::Quad(rect(100.0, 10.0, 2.0, 20.0), color(frame_of(4, 1, "").cursor.unwrap().color)))
        );
        let under = painted(&frame_of(4, 1, "\x1b[4 q"), true);
        assert!(under.iter().any(|o| matches!(o, Op::Quad(r, _) if *r == rect(100.0, 28.0, 8.0, 2.0))));
        // Underline and strike: bars at the bottom and the middle of the cell.
        let deco = painted(&frame_of(4, 1, "\x1b[4;9mu\x1b[0m\x1b[?25l"), true);
        let bars: Vec<Rectangle> = deco
            .iter()
            .filter_map(|o| match o {
                Op::Quad(r, _) if r.height == 1.0 => Some(*r),
                _ => None,
            })
            .collect();
        assert_eq!(bars, [rect(100.0, 28.0, 8.0, 1.0), rect(100.0, 20.0, 8.0, 1.0)]);
        // Box lines are quads, not text.
        let boxed = painted(&frame_of(4, 1, "─\x1b[?25l"), true);
        assert!(!boxed.iter().any(|o| matches!(o, Op::Text(..))));
        assert_eq!(boxed.iter().filter(|o| matches!(o, Op::Quad(..))).count(), 3);
    }

    #[test]
    fn the_history_thumb_tracks_the_scroll() {
        let mut f = frame_of(4, 2, "");
        f.history = 8;
        f.display_offset = 8;
        let b = rect(0.0, 0.0, 32.0, 100.0);
        let top = history_thumb(&f, b).unwrap();
        assert_eq!((top.x, top.y, top.width), (27.0, 0.0, 4.0));
        assert_eq!(top.height, 24.0, "a minimum height");
        f.display_offset = 4;
        let mid = history_thumb(&f, b).unwrap();
        assert_eq!(mid.y, 38.0);
        f.display_offset = 0;
        assert_eq!(history_thumb(&f, b), None);
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
