//! Terminal input and host-terminal duties that iced_term 0.8 leaves out,
//! done from mira through iced_term's public API (no fork, no patch):
//!
//! - [`TermInput`] wraps the terminal view and, while a mouse-reporting TUI
//!   (herdr) runs, turns the wheel and the right/middle buttons into mouse
//!   reports, sends Alt+key as `ESC key`, pastes with bracketed paste and stops
//!   `Ctrl+Shift+C` from overwriting what herdr already copied.
//! - [`color_reply_rgb`] answers OSC 4/10/11 color queries with mira's palette.
//! - [`bracketed`] wraps pasted text.
//!
//! Everything else is forwarded untouched to the wrapped widget.

use crate::theme;
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, clipboard, overlay};
use iced::keyboard::{self, Key, Modifiers};
use iced::mouse::{self, ScrollDelta};
use iced::{Element, Event, Length, Rectangle, Size, Vector};

/// A mouse report for the PTY, in grid cells (0-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Mouse {
        button: Button,
        pressed: bool,
        line: usize,
        col: usize,
        mods: Modifiers,
    },
    /// Raw bytes to write (Alt+key as `ESC key`).
    Bytes(Vec<u8>),
    /// Clipboard text to paste.
    Paste(String),
}

#[derive(Default)]
struct State {
    focused: bool,
    seen_epoch: u64,
    scroll_pixels: f32,
    modifiers: Modifiers,
}

pub struct TermInput<'a, Message> {
    content: Element<'a, Message>,
    /// Only intercept while a mouse-reporting TUI runs.
    enabled: bool,
    /// Cell size in pixels, as the terminal measured it.
    cell: Option<Size>,
    /// Bumped by the app whenever it focuses the terminal programmatically.
    focus_epoch: u64,
    on_input: Box<dyn Fn(Input) -> Message + 'a>,
}

impl<'a, Message> TermInput<'a, Message> {
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        enabled: bool,
        cell: Option<Size>,
        focus_epoch: u64,
        on_input: impl Fn(Input) -> Message + 'a,
    ) -> Self {
        Self { content: content.into(), enabled, cell, focus_epoch, on_input: Box::new(on_input) }
    }
}

/// Grid cell under `pos` (relative to the terminal's top-left).
fn grid(bounds: Rectangle, cell: Size, pos: iced::Point) -> (usize, usize) {
    let cols = (bounds.width / cell.width).floor().max(1.0) as usize;
    let lines = (bounds.height / cell.height).floor().max(1.0) as usize;
    let x = (pos.x - bounds.x).max(0.0);
    let y = (pos.y - bounds.y).max(0.0);
    (((y / cell.height) as usize).min(lines - 1), ((x / cell.width) as usize).min(cols - 1))
}

/// Wheel delta → whole lines, keeping the pixel remainder for next time.
fn wheel_lines(delta: ScrollDelta, cell_height: f32, carry: &mut f32) -> i32 {
    match delta {
        ScrollDelta::Lines { y, .. } => y.round() as i32,
        ScrollDelta::Pixels { y, .. } => {
            *carry += y;
            let lines = (*carry / cell_height).trunc();
            *carry -= lines * cell_height;
            lines as i32
        }
    }
}

/// `Alt+key` → `ESC key`, for printable keys without Ctrl/Super.
fn alt_bytes(key: &Key, mods: Modifiers) -> Option<Vec<u8>> {
    if !mods.alt() || mods.control() || mods.logo() {
        return None;
    }
    let Key::Character(c) = key else { return None };
    let mut out = vec![0x1b];
    if mods.shift() {
        out.extend(c.to_uppercase().as_bytes());
    } else {
        out.extend(c.as_bytes());
    }
    Some(out)
}

fn is_ctrl_shift(key: &Key, mods: Modifiers, letter: &str) -> bool {
    mods.control() && mods.shift() && matches!(key, Key::Character(c) if c.eq_ignore_ascii_case(letter))
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for TermInput<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state: &mut State = tree.state.downcast_mut();
        if state.seen_epoch != self.focus_epoch {
            state.seen_epoch = self.focus_epoch;
            state.focused = true;
        }
        if let Event::Keyboard(keyboard::Event::ModifiersChanged(m)) = event {
            state.modifiers = *m;
        }
        if let Event::Mouse(mouse::Event::ButtonPressed(_)) = event {
            // Same rule as the terminal widget: a click inside focuses it.
            state.focused = cursor.is_over(bounds);
        }

        if self.enabled {
            let mut handled = None;
            match event {
                Event::Mouse(e) if self.cell.is_some() && cursor.is_over(bounds) => {
                    let cell = self.cell.unwrap_or(Size::new(1.0, 1.0));
                    let pos = cursor.position().unwrap_or(bounds.position());
                    let (line, col) = grid(bounds, cell, pos);
                    let mods = state.modifiers;
                    let report = |button, pressed| Input::Mouse { button, pressed, line, col, mods };
                    handled = match e {
                        mouse::Event::ButtonPressed(mouse::Button::Right) => Some(vec![report(Button::Right, true)]),
                        mouse::Event::ButtonReleased(mouse::Button::Right) => Some(vec![report(Button::Right, false)]),
                        mouse::Event::ButtonPressed(mouse::Button::Middle) => Some(vec![report(Button::Middle, true)]),
                        mouse::Event::ButtonReleased(mouse::Button::Middle) => {
                            Some(vec![report(Button::Middle, false)])
                        }
                        mouse::Event::WheelScrolled { delta } => {
                            let n = wheel_lines(*delta, cell.height, &mut state.scroll_pixels);
                            let button = if n > 0 { Button::WheelUp } else { Button::WheelDown };
                            Some((0..n.unsigned_abs()).map(|_| report(button, true)).collect())
                        }
                        _ => None,
                    };
                }
                Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) if state.focused => {
                    if is_ctrl_shift(key, *modifiers, "v") {
                        let text = clipboard.read(clipboard::Kind::Standard).unwrap_or_default();
                        handled = Some(if text.is_empty() { vec![] } else { vec![Input::Paste(text)] });
                    } else if is_ctrl_shift(key, *modifiers, "c") {
                        // herdr copies on select; the widget would copy its own (empty) selection.
                        handled = Some(vec![]);
                    } else if let Some(bytes) = alt_bytes(key, *modifiers) {
                        handled = Some(vec![Input::Bytes(bytes)]);
                    }
                }
                _ => {}
            }
            if let Some(inputs) = handled {
                for i in inputs {
                    shell.publish((self.on_input)(i));
                }
                shell.capture_event();
                return;
            }
        }

        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<TermInput<'a, Message>> for Element<'a, Message> {
    fn from(w: TermInput<'a, Message>) -> Self {
        Element::new(w)
    }
}

/// Bracketed paste. An embedded end marker would let pasted text escape the
/// paste and run as typed input, so it is removed.
pub fn bracketed(text: &str) -> Vec<u8> {
    let clean = text.replace("\x1b[201~", "");
    let mut out = Vec::with_capacity(clean.len() + 12);
    out.extend_from_slice(b"\x1b[200~");
    out.extend_from_slice(clean.as_bytes());
    out.extend_from_slice(b"\x1b[201~");
    out
}

fn hex_rgb(s: &str) -> (u8, u8, u8) {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
    ((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn color_rgb(c: iced::Color) -> (u8, u8, u8) {
    let [r, g, b, _] = c.into_rgba8();
    (r, g, b)
}

/// RGB for an alacritty color index: 0–15 palette, 16–255 xterm cube and
/// greys, 256 foreground, 257 background, 258 cursor, others foreground.
pub fn color_reply_rgb(index: usize) -> (u8, u8, u8) {
    let p = theme::p();
    match index {
        0..=15 => hex_rgb(p.ansi[index]),
        16..=231 => {
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let i = index - 16;
            (LEVELS[i / 36], LEVELS[(i / 6) % 6], LEVELS[i % 6])
        }
        232..=255 => {
            let g = (8 + 10 * (index - 232)) as u8;
            (g, g, g)
        }
        257 => color_rgb(p.bg),
        _ => color_rgb(p.fg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_maps_pixels_to_clamped_cells() {
        let b = Rectangle { x: 100.0, y: 50.0, width: 80.0, height: 40.0 };
        let cell = Size::new(8.0, 20.0);
        assert_eq!(grid(b, cell, iced::Point::new(100.0, 50.0)), (0, 0));
        assert_eq!(grid(b, cell, iced::Point::new(125.0, 71.0)), (1, 3));
        assert_eq!(grid(b, cell, iced::Point::new(500.0, 500.0)), (1, 9));
    }

    #[test]
    fn wheel_pixels_accumulate_into_lines() {
        let mut carry = 0.0;
        assert_eq!(wheel_lines(ScrollDelta::Pixels { x: 0.0, y: 15.0 }, 20.0, &mut carry), 0);
        assert_eq!(wheel_lines(ScrollDelta::Pixels { x: 0.0, y: 30.0 }, 20.0, &mut carry), 2);
        assert_eq!(wheel_lines(ScrollDelta::Lines { x: 0.0, y: -3.0 }, 20.0, &mut carry), -3);
    }

    #[test]
    fn alt_letters_become_escape_prefixed() {
        assert_eq!(alt_bytes(&Key::Character("b".into()), Modifiers::ALT), Some(b"\x1bb".to_vec()));
        assert_eq!(alt_bytes(&Key::Character("b".into()), Modifiers::ALT | Modifiers::SHIFT), Some(b"\x1bB".to_vec()));
        assert_eq!(alt_bytes(&Key::Character("b".into()), Modifiers::ALT | Modifiers::CTRL), None);
        assert_eq!(alt_bytes(&Key::Character("b".into()), Modifiers::empty()), None);
    }

    #[test]
    fn paste_cannot_escape_the_brackets() {
        assert_eq!(bracketed("a\x1b[201~rm -rf /\n"), b"\x1b[200~arm -rf /\n\x1b[201~".to_vec());
    }

    #[test]
    fn color_queries_follow_xterm_layout() {
        assert_eq!(color_reply_rgb(16), (0, 0, 0));
        assert_eq!(color_reply_rgb(231), (255, 255, 255));
        assert_eq!(color_reply_rgb(232), (8, 8, 8));
        assert_eq!(color_reply_rgb(1), hex_rgb(theme::p().ansi[1]));
        assert_eq!(color_reply_rgb(257), color_rgb(theme::p().bg));
    }
}
