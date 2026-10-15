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
    /// Right/middle button reported as pressed, so its release is reported
    /// even outside the terminal.
    held: Option<Button>,
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
    /// `(epoch, terminal)`: the epoch moves whenever the app moves focus
    /// itself, to the terminal or (`false`) to another widget.
    focus: (u64, bool),
    on_input: Box<dyn Fn(Input) -> Message + 'a>,
}

impl<'a, Message> TermInput<'a, Message> {
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        enabled: bool,
        cell: Option<Size>,
        focus: (u64, bool),
        on_input: impl Fn(Input) -> Message + 'a,
    ) -> Self {
        Self { content: content.into(), enabled, cell, focus, on_input: Box::new(on_input) }
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
        if state.seen_epoch != self.focus.0 {
            state.seen_epoch = self.focus.0;
            state.focused = self.focus.1;
        }
        if let Event::Keyboard(keyboard::Event::ModifiersChanged(m)) = event {
            state.modifiers = *m;
        }
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            // Same rule as the terminal and text widgets: a left click moves focus.
            state.focused = cursor.is_over(bounds);
        }

        if self.enabled {
            let mut handled = None;
            match event {
                Event::Mouse(e) if self.cell.is_some() => {
                    let over = cursor.is_over(bounds);
                    let cell = self.cell.unwrap_or(Size::new(1.0, 1.0));
                    let pos = cursor.position().unwrap_or(bounds.position());
                    let (line, col) = grid(bounds, cell, pos);
                    let mods = state.modifiers;
                    let report = |button, pressed| Input::Mouse { button, pressed, line, col, mods };
                    let button_of = |b: &mouse::Button| match b {
                        mouse::Button::Right => Some(Button::Right),
                        mouse::Button::Middle => Some(Button::Middle),
                        _ => None,
                    };
                    handled = match e {
                        mouse::Event::ButtonPressed(b) if over => button_of(b).map(|b| {
                            state.held = Some(b);
                            vec![report(b, true)]
                        }),
                        mouse::Event::ButtonReleased(b) if button_of(b).is_some() && state.held == button_of(b) => {
                            state.held = None;
                            button_of(b).map(|b| vec![report(b, false)])
                        }
                        mouse::Event::WheelScrolled { delta } if over => {
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

// ------------------------------------------------------------------ gate

/// Keeps mira's UI thread from deadlocking with the terminal's PTY thread.
///
/// iced_term 0.8 sends alacritty events with `blocking_send` into a channel of
/// 100, *while the PTY thread holds the terminal lock*, and that channel only
/// drains while the UI thread processes messages. Every `Terminal::handle` call
/// takes the same lock, and `Write` additionally emits an event from the UI
/// thread. So if the UI thread handles a command while the PTY thread is in the
/// middle of a large batch of events (herdr asks for all 256 palette colors at
/// startup), both wait on each other forever.
///
/// The gate fixes that from the outside: while events of a batch are arriving
/// nothing is handed to the terminal; they are queued (and color queries are
/// answered from mira's palette) and forwarded in one go once the batch ends
/// (alacritty's `Wakeup`), or after a quiet moment for batches that end without
/// one (synchronized output). Writes are coalesced so a flush emits as few
/// events as possible.
#[derive(Default)]
pub struct Gate {
    queue: Vec<iced_term::Command>,
    last_event: Option<std::time::Instant>,
}

/// Quiet time after the last event before a batch without `Wakeup` is flushed.
pub const GATE_QUIET: std::time::Duration = std::time::Duration::from_millis(25);

/// What to do with one terminal command.
pub enum Gated {
    /// Hand these to the terminal now, in order.
    Forward(Vec<iced_term::Command>),
    /// Queued; the app must make sure a flush happens (see [`Gate::flush_due`]).
    Queued,
}

impl Gate {
    pub fn busy(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Whether a batch has been quiet long enough to flush.
    pub fn flush_due(&self, now: std::time::Instant) -> bool {
        self.busy() && self.last_event.is_none_or(|t| now.duration_since(t) >= GATE_QUIET)
    }

    /// Everything queued, with adjacent writes merged.
    pub fn flush(&mut self) -> Vec<iced_term::Command> {
        use iced_term::BackendCommand::Write;
        use iced_term::Command::ProxyToBackend;
        let mut out: Vec<iced_term::Command> = Vec::with_capacity(self.queue.len());
        for cmd in self.queue.drain(..) {
            match (out.last_mut(), cmd) {
                (Some(ProxyToBackend(Write(prev))), ProxyToBackend(Write(more))) => prev.extend(more),
                (_, cmd) => out.push(cmd),
            }
        }
        out
    }

    /// A command from mira or the terminal widget (keys, mouse, resize, theme).
    pub fn command(&mut self, cmd: iced_term::Command) -> Gated {
        if self.busy() {
            self.queue.push(cmd);
            Gated::Queued
        } else {
            Gated::Forward(vec![cmd])
        }
    }

    /// An event from the PTY thread. `color` answers palette queries.
    pub fn event(
        &mut self,
        event: alacritty_terminal::event::Event,
        now: std::time::Instant,
        color: impl Fn(usize) -> alacritty_terminal::vte::ansi::Rgb,
    ) -> Gated {
        use alacritty_terminal::event::Event;
        use iced_term::BackendCommand::{ProcessAlacrittyEvent, Write};
        use iced_term::Command::ProxyToBackend;
        let queued = match event {
            // End of a batch: forward everything, then the redraw.
            Event::Wakeup => {
                let mut out = self.flush();
                out.push(ProxyToBackend(ProcessAlacrittyEvent(Event::Wakeup)));
                self.last_event = None;
                return Gated::Forward(out);
            }
            Event::ColorRequest(index, reply) => ProxyToBackend(Write(reply(color(index)).into_bytes())),
            // The only other events iced_term acts on.
            e @ (Event::PtyWrite(_) | Event::Exit) => ProxyToBackend(ProcessAlacrittyEvent(e)),
            // iced_term ignores the rest (cursor dirty/blink, bell, title, size
            // requests); clipboard stores are taken by the app before this.
            _ => return Gated::Forward(Vec::new()),
        };
        self.last_event = Some(now);
        self.queue.push(queued);
        Gated::Queued
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

/// The terminal's light/dark report (`CSI ? 997 ; 1 n` dark, `; 2 n` light),
/// as Ghostty and other terminals send it for mode 2031. alacritty_terminal
/// doesn't implement it, so mira types it to the app (herdr reads it at any
/// time and switches between its `dark_name` and `light_name` themes).
pub fn color_scheme_report(dark: bool) -> &'static [u8] {
    if dark { b"\x1b[?997;1n" } else { b"\x1b[?997;2n" }
}

/// Whether `palette` is dark, judged like terminals and herdr do:
/// perceived luminance (Rec. 601) of the background below one half.
pub fn is_dark(palette: &theme::Palette) -> bool {
    let bg = palette.bg;
    0.299 * bg.r + 0.587 * bg.g + 0.114 * bg.b < 0.5
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
    fn color_scheme_report_matches_each_palette() {
        assert_eq!(color_scheme_report(true), b"\x1b[?997;1n");
        assert_eq!(color_scheme_report(false), b"\x1b[?997;2n");
        for p in &theme::PALETTES {
            assert_eq!(is_dark(p), p.name.ends_with("Dark"), "{}", p.name);
        }
    }

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

    mod gate {
        use super::super::*;
        use alacritty_terminal::event::Event;
        use alacritty_terminal::vte::ansi::Rgb;
        use iced_term::BackendCommand::{ProcessAlacrittyEvent, Write};
        use iced_term::Command::ProxyToBackend;
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        fn rgb(i: usize) -> Rgb {
            Rgb { r: i as u8, g: 0, b: 0 }
        }

        fn query(i: usize) -> Event {
            Event::ColorRequest(i, Arc::new(move |c: Rgb| format!("[{i}:{}]", c.r)))
        }

        fn written(cmds: &[iced_term::Command]) -> Vec<String> {
            cmds.iter()
                .map(|c| match c {
                    ProxyToBackend(Write(b)) => String::from_utf8_lossy(b).into_owned(),
                    ProxyToBackend(ProcessAlacrittyEvent(Event::Wakeup)) => "wakeup".into(),
                    ProxyToBackend(ProcessAlacrittyEvent(Event::PtyWrite(t))) => format!("pty:{t}"),
                    _ => "other".into(),
                })
                .collect()
        }

        #[test]
        fn a_burst_of_color_queries_never_reaches_the_terminal_until_it_ends() {
            let mut g = Gate::default();
            let now = Instant::now();
            for i in 0..258 {
                assert!(matches!(g.event(query(i), now, rgb), Gated::Queued), "query {i} must be queued");
            }
            // Keys typed meanwhile wait too, in order.
            assert!(matches!(g.command(ProxyToBackend(Write(b"k".to_vec()))), Gated::Queued));
            let Gated::Forward(out) = g.event(Event::Wakeup, now, rgb) else { panic!("wakeup forwards") };
            // All answers plus the key in ONE write (one lock, one event), then the redraw.
            let text = written(&out);
            assert_eq!(text.len(), 2);
            assert!(text[0].starts_with("[0:0][1:1]") && text[0].ends_with("[257:1]k"));
            assert_eq!(text[1], "wakeup");
            assert!(!g.busy());
        }

        #[test]
        fn idle_commands_pass_through_and_noise_events_are_dropped() {
            let mut g = Gate::default();
            assert!(matches!(g.command(ProxyToBackend(Write(b"x".to_vec()))), Gated::Forward(v) if v.len() == 1));
            assert!(matches!(g.event(Event::MouseCursorDirty, Instant::now(), rgb), Gated::Forward(v) if v.is_empty()));
            assert!(!g.busy());
        }

        #[test]
        fn batches_without_wakeup_flush_after_a_quiet_moment() {
            let mut g = Gate::default();
            let t0 = Instant::now();
            assert!(matches!(g.event(Event::PtyWrite("\x1b[?1;2c".into()), t0, rgb), Gated::Queued));
            assert!(!g.flush_due(t0));
            assert!(g.flush_due(t0 + GATE_QUIET + Duration::from_millis(1)));
            assert_eq!(written(&g.flush()), ["pty:\x1b[?1;2c"]);
            assert!(!g.flush_due(t0 + Duration::from_secs(1)));
        }
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
