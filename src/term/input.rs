//! What the terminal sends to the program for a key or a mouse event, as
//! xterm does: pure functions over the key, the modifiers and the terminal's
//! modes, so every sequence is tested here.

use alacritty_terminal::term::TermMode;
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::mouse::ScrollDelta;

const ESC: u8 = 0x1b;

/// xterm's modifier parameter: 1 + Shift 1 + Alt 2 + Ctrl 4.
fn modifier_param(mods: Modifiers) -> u8 {
    1 + u8::from(mods.shift()) + 2 * u8::from(mods.alt()) + 4 * u8::from(mods.control())
}

fn has_mods(mods: Modifiers) -> bool {
    mods.shift() || mods.alt() || mods.control()
}

/// `CSI 1 ; m X` with modifiers, else `SS3 X` (application mode) or `CSI X`.
fn cursor_key(letter: u8, mods: Modifiers, app_cursor: bool) -> Vec<u8> {
    if has_mods(mods) {
        format!("\x1b[1;{}{}", modifier_param(mods), letter as char).into_bytes()
    } else if app_cursor {
        vec![ESC, b'O', letter]
    } else {
        vec![ESC, b'[', letter]
    }
}

/// `CSI n ~`, or `CSI n ; m ~` with modifiers.
fn tilde_key(n: u8, mods: Modifiers) -> Vec<u8> {
    if has_mods(mods) { format!("\x1b[{n};{}~", modifier_param(mods)) } else { format!("\x1b[{n}~") }.into_bytes()
}

/// F1–F4 are `SS3 P..S` alone and `CSI 1 ; m P..S` with modifiers.
fn pf_key(letter: u8, mods: Modifiers) -> Vec<u8> {
    if has_mods(mods) {
        format!("\x1b[1;{}{}", modifier_param(mods), letter as char).into_bytes()
    } else {
        vec![ESC, b'O', letter]
    }
}

/// With Alt, the sequence goes after an ESC (xterm's metaSendsEscape).
fn alt_prefixed(mut bytes: Vec<u8>, alt: bool) -> Vec<u8> {
    if alt {
        bytes.insert(0, ESC);
    }
    bytes
}

fn named(key: Named, mods: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let alt = mods.alt();
    Some(match key {
        Named::Enter => alt_prefixed(vec![b'\r'], alt),
        Named::Backspace if mods.control() => alt_prefixed(vec![0x08], alt),
        Named::Backspace => alt_prefixed(vec![0x7f], alt),
        Named::Tab if mods.shift() => b"\x1b[Z".to_vec(),
        Named::Tab => alt_prefixed(vec![b'\t'], alt),
        Named::Escape => alt_prefixed(vec![ESC], alt),
        Named::Space if mods.control() => alt_prefixed(vec![0], alt),
        Named::Space => alt_prefixed(vec![b' '], alt),
        Named::ArrowUp => cursor_key(b'A', mods, app_cursor),
        Named::ArrowDown => cursor_key(b'B', mods, app_cursor),
        Named::ArrowRight => cursor_key(b'C', mods, app_cursor),
        Named::ArrowLeft => cursor_key(b'D', mods, app_cursor),
        Named::Home => cursor_key(b'H', mods, app_cursor),
        Named::End => cursor_key(b'F', mods, app_cursor),
        Named::Insert => tilde_key(2, mods),
        Named::Delete => tilde_key(3, mods),
        Named::PageUp => tilde_key(5, mods),
        Named::PageDown => tilde_key(6, mods),
        Named::F1 => pf_key(b'P', mods),
        Named::F2 => pf_key(b'Q', mods),
        Named::F3 => pf_key(b'R', mods),
        Named::F4 => pf_key(b'S', mods),
        Named::F5 => tilde_key(15, mods),
        Named::F6 => tilde_key(17, mods),
        Named::F7 => tilde_key(18, mods),
        Named::F8 => tilde_key(19, mods),
        Named::F9 => tilde_key(20, mods),
        Named::F10 => tilde_key(21, mods),
        Named::F11 => tilde_key(23, mods),
        Named::F12 => tilde_key(24, mods),
        Named::F13 => tilde_key(25, mods),
        Named::F14 => tilde_key(26, mods),
        Named::F15 => tilde_key(28, mods),
        Named::F16 => tilde_key(29, mods),
        Named::F17 => tilde_key(31, mods),
        Named::F18 => tilde_key(32, mods),
        Named::F19 => tilde_key(33, mods),
        Named::F20 => tilde_key(34, mods),
        _ => return None,
    })
}

/// The control code for `Ctrl+c`, as xterm maps it (letters, `@[\]^_`, and
/// the digits 2–8 of the VT220 layout).
fn control_code(c: char) -> Option<u8> {
    let c = c.to_ascii_lowercase();
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        '@' | '2' | ' ' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// Bytes for a key press, or `None` when the terminal doesn't send anything
/// for it (it is left to the app: Super shortcuts, lone modifiers).
/// `text` is what the key types with the layout and Shift applied.
pub fn key(key: &Key, text: Option<&str>, mods: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    if mods.logo() {
        return None;
    }
    match key {
        Key::Named(n) => named(*n, mods, mode),
        Key::Character(c) if mods.control() => {
            let ch = c.chars().next()?;
            match control_code(ch) {
                Some(code) => Some(alt_prefixed(vec![code], mods.alt())),
                // Ctrl with a key that has no control code types it as is.
                None => text.map(|t| alt_prefixed(t.as_bytes().to_vec(), mods.alt())),
            }
        }
        Key::Character(c) => {
            let typed = text.filter(|t| !t.is_empty()).unwrap_or(c.as_str());
            Some(alt_prefixed(typed.as_bytes().to_vec(), mods.alt()))
        }
        // A key the layout doesn't name but that types something (compose, IME-less input).
        _ => text.filter(|t| !t.is_empty()).map(|t| alt_prefixed(t.as_bytes().to_vec(), mods.alt())),
    }
}

/// A mouse button as xterm numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left = 0,
    Middle = 1,
    Right = 2,
    WheelUp = 64,
    WheelDown = 65,
}

/// What happened with the mouse, for a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEvent {
    Press(Button),
    Release(Button),
    /// Movement with this button held, or with none.
    Motion(Option<Button>),
}

/// The mouse report for `event` at cell (`line`, `col`), 0-based, in the
/// encoding the program asked for (SGR, UTF-8 or the legacy one). `None`
/// when the legacy encodings can't express the position.
pub fn mouse_report(event: MouseEvent, line: usize, col: usize, mods: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let mod_bits = 4 * u8::from(mods.shift()) + 8 * u8::from(mods.alt()) + 16 * u8::from(mods.control());
    let (button, pressed) = match event {
        MouseEvent::Press(b) => (b as u8, true),
        MouseEvent::Release(b) => (b as u8, false),
        // 32 marks motion; 3 is "no button".
        MouseEvent::Motion(b) => (32 + b.map_or(3, |b| b as u8), true),
    };
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{};{};{}{end}", button + mod_bits, col + 1, line + 1).into_bytes());
    }
    // Legacy encodings report every release as button 3, and never wheel releases.
    let code = match event {
        MouseEvent::Release(Button::WheelUp | Button::WheelDown) => return None,
        MouseEvent::Release(_) => 3,
        _ => button,
    } + mod_bits;
    let mut out = vec![ESC, b'[', b'M', 32 + code];
    let utf8 = mode.contains(TermMode::UTF8_MOUSE);
    for pos in [col, line] {
        let v = 32 + 1 + pos;
        if v < 128 {
            out.push(v as u8);
        } else if utf8 && v < 2048 {
            out.extend(char::from_u32(v as u32)?.to_string().as_bytes());
        } else if !utf8 && v < 256 {
            out.push(v as u8);
        } else {
            return None;
        }
    }
    Some(out)
}

/// Lines of history to scroll per wheel notch, as Alacritty does.
pub const WHEEL_LINES: i32 = 3;

/// Wheel delta → whole lines (positive: up/back), keeping the pixel remainder
/// in `carry` for the next event. A notch counts `per_notch` lines.
pub fn wheel_lines(delta: ScrollDelta, cell_height: f32, carry: &mut f32, per_notch: i32) -> i32 {
    match delta {
        ScrollDelta::Lines { y, .. } => (y * per_notch as f32).round() as i32,
        ScrollDelta::Pixels { y, .. } => {
            *carry += y;
            let lines = (*carry / cell_height.max(1.0)).trunc();
            *carry -= lines * cell_height.max(1.0);
            lines as i32
        }
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

/// What to write for a paste: bracketed when the program asked for it, with
/// line breaks as Enter (CR) like other terminals.
pub fn paste(text: &str, mode: TermMode) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if mode.contains(TermMode::BRACKETED_PASTE) { bracketed(&text) } else { text.into_bytes() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(c: &str) -> Key {
        Key::Character(c.into())
    }

    fn k(key: Key, text: Option<&str>, mods: Modifiers) -> Vec<u8> {
        super::key(&key, text, mods, TermMode::empty()).unwrap_or_default()
    }

    const NONE: Modifiers = Modifiers::empty();

    #[test]
    fn text_types_as_is_and_alt_prefixes_esc() {
        assert_eq!(k(ch("a"), Some("a"), NONE), b"a");
        assert_eq!(k(ch("a"), Some("A"), Modifiers::SHIFT), b"A");
        assert_eq!(k(ch("é"), Some("é"), NONE), "é".as_bytes());
        assert_eq!(k(ch("b"), Some("b"), Modifiers::ALT), b"\x1bb");
        assert_eq!(k(ch("b"), Some("B"), Modifiers::ALT | Modifiers::SHIFT), b"\x1bB");
        // Super belongs to the desktop.
        assert_eq!(super::key(&ch("a"), Some("a"), Modifiers::LOGO, TermMode::empty()), None);
        // Typed by a key the layout doesn't name.
        assert_eq!(k(Key::Unidentified, Some("ñ"), NONE), "ñ".as_bytes());
        assert_eq!(super::key(&Key::Unidentified, None, NONE, TermMode::empty()), None);
    }

    #[test]
    fn control_letters_and_symbols() {
        assert_eq!(k(ch("c"), None, Modifiers::CTRL), [0x03]);
        // Kill line (0x15), not 0x51.
        assert_eq!(k(ch("u"), None, Modifiers::CTRL), [0x15]);
        assert_eq!(k(ch("U"), None, Modifiers::CTRL | Modifiers::SHIFT), [0x15]);
        assert_eq!(k(ch("["), None, Modifiers::CTRL), [0x1b]);
        assert_eq!(k(ch("2"), None, Modifiers::CTRL), [0x00]);
        assert_eq!(k(ch("/"), None, Modifiers::CTRL), [0x1f]);
        assert_eq!(k(ch("d"), None, Modifiers::CTRL | Modifiers::ALT), [0x1b, 0x04]);
        assert_eq!(k(ch("."), Some("."), Modifiers::CTRL), b".");
    }

    #[test]
    fn named_keys_follow_xterm() {
        let n = |key, mods| k(Key::Named(key), None, mods);
        assert_eq!(n(Named::Enter, NONE), b"\r");
        assert_eq!(n(Named::Enter, Modifiers::ALT), b"\x1b\r");
        assert_eq!(n(Named::Backspace, NONE), [0x7f]);
        assert_eq!(n(Named::Backspace, Modifiers::CTRL), [0x08]);
        assert_eq!(n(Named::Backspace, Modifiers::ALT), [0x1b, 0x7f]);
        assert_eq!(n(Named::Tab, Modifiers::SHIFT), b"\x1b[Z");
        assert_eq!(n(Named::Space, Modifiers::CTRL), [0]);
        assert_eq!(n(Named::ArrowUp, NONE), b"\x1b[A");
        assert_eq!(n(Named::ArrowLeft, Modifiers::CTRL), b"\x1b[1;5D");
        assert_eq!(n(Named::ArrowRight, Modifiers::SHIFT | Modifiers::ALT), b"\x1b[1;4C");
        assert_eq!(n(Named::Home, NONE), b"\x1b[H");
        assert_eq!(n(Named::Delete, NONE), b"\x1b[3~");
        assert_eq!(n(Named::PageUp, Modifiers::CTRL), b"\x1b[5;5~");
        assert_eq!(n(Named::F1, NONE), b"\x1bOP");
        assert_eq!(n(Named::F4, Modifiers::SHIFT), b"\x1b[1;2S");
        assert_eq!(n(Named::F5, NONE), b"\x1b[15~");
        assert_eq!(n(Named::F12, Modifiers::CTRL), b"\x1b[24;5~");
        // Lone modifiers send nothing.
        assert_eq!(super::key(&Key::Named(Named::Shift), None, Modifiers::SHIFT, TermMode::empty()), None);
    }

    #[test]
    fn every_named_key_and_control_code() {
        let n = |key| String::from_utf8(k(Key::Named(key), None, NONE)).unwrap();
        let table = [
            (Named::Escape, "\x1b"),
            (Named::Tab, "\t"),
            (Named::Space, " "),
            (Named::ArrowDown, "\x1b[B"),
            (Named::ArrowRight, "\x1b[C"),
            (Named::End, "\x1b[F"),
            (Named::Insert, "\x1b[2~"),
            (Named::PageDown, "\x1b[6~"),
            (Named::F2, "\x1bOQ"),
            (Named::F3, "\x1bOR"),
            (Named::F6, "\x1b[17~"),
            (Named::F7, "\x1b[18~"),
            (Named::F8, "\x1b[19~"),
            (Named::F9, "\x1b[20~"),
            (Named::F10, "\x1b[21~"),
            (Named::F11, "\x1b[23~"),
            (Named::F13, "\x1b[25~"),
            (Named::F14, "\x1b[26~"),
            (Named::F15, "\x1b[28~"),
            (Named::F16, "\x1b[29~"),
            (Named::F17, "\x1b[31~"),
            (Named::F18, "\x1b[32~"),
            (Named::F19, "\x1b[33~"),
            (Named::F20, "\x1b[34~"),
        ];
        for (key, seq) in table {
            assert_eq!(n(key), seq, "{key:?}");
        }
        let ctrl = |c: &str| k(ch(c), None, Modifiers::CTRL);
        let codes = [("@", 0), (" ", 0), ("3", 0x1b), ("\\", 0x1c), ("4", 0x1c), ("]", 0x1d), ("5", 0x1d)];
        let more = [("^", 0x1e), ("6", 0x1e), ("_", 0x1f), ("-", 0x1f), ("7", 0x1f), ("?", 0x7f), ("8", 0x7f)];
        for (c, code) in codes.into_iter().chain(more) {
            assert_eq!(ctrl(c), [code], "Ctrl+{c}");
        }
        // Ctrl with a key that has no code and types nothing sends nothing.
        assert_eq!(super::key(&ch("."), None, Modifiers::CTRL, TermMode::empty()), None);
    }

    #[test]
    fn application_cursor_mode_uses_ss3() {
        let app = TermMode::APP_CURSOR;
        assert_eq!(super::key(&Key::Named(Named::ArrowUp), None, NONE, app), Some(b"\x1bOA".to_vec()));
        assert_eq!(super::key(&Key::Named(Named::End), None, NONE, app), Some(b"\x1bOF".to_vec()));
        // With modifiers it is CSI either way.
        assert_eq!(super::key(&Key::Named(Named::ArrowUp), None, Modifiers::CTRL, app), Some(b"\x1b[1;5A".to_vec()));
    }

    #[test]
    fn sgr_mouse_reports() {
        let sgr = TermMode::SGR_MOUSE;
        let r = |e, mods| mouse_report(e, 4, 9, mods, sgr).unwrap();
        assert_eq!(r(MouseEvent::Press(Button::Left), NONE), b"\x1b[<0;10;5M");
        assert_eq!(r(MouseEvent::Release(Button::Left), NONE), b"\x1b[<0;10;5m");
        assert_eq!(r(MouseEvent::Press(Button::Right), Modifiers::CTRL), b"\x1b[<18;10;5M");
        assert_eq!(r(MouseEvent::Press(Button::WheelDown), NONE), b"\x1b[<65;10;5M");
        assert_eq!(r(MouseEvent::Motion(Some(Button::Left)), NONE), b"\x1b[<32;10;5M");
        assert_eq!(r(MouseEvent::Motion(None), NONE), b"\x1b[<35;10;5M");
    }

    #[test]
    fn legacy_mouse_reports() {
        let normal = TermMode::empty();
        assert_eq!(
            mouse_report(MouseEvent::Press(Button::Middle), 0, 0, NONE, normal),
            Some(vec![0x1b, b'[', b'M', 33, 33, 33])
        );
        assert_eq!(
            mouse_report(MouseEvent::Release(Button::Middle), 1, 2, NONE, normal),
            Some(vec![0x1b, b'[', b'M', 35, 35, 34])
        );
        assert_eq!(mouse_report(MouseEvent::Release(Button::WheelUp), 0, 0, NONE, normal), None);
        // Past column 222 only the UTF-8 encoding can say where.
        assert_eq!(mouse_report(MouseEvent::Press(Button::Left), 0, 300, NONE, normal), None);
        let utf8 = mouse_report(MouseEvent::Press(Button::Left), 0, 300, NONE, TermMode::UTF8_MOUSE).unwrap();
        assert_eq!(&utf8[4..], "\u{14d}!".as_bytes());
        // Columns 95–222 fit in one byte without UTF-8; past 2014 nothing can say it.
        assert_eq!(mouse_report(MouseEvent::Press(Button::Left), 0, 150, NONE, normal).map(|r| r[4]), Some(183));
        assert_eq!(mouse_report(MouseEvent::Press(Button::Left), 0, 3000, NONE, TermMode::UTF8_MOUSE), None);
    }

    #[test]
    fn wheel_notches_and_pixels() {
        let mut carry = 0.0;
        assert_eq!(wheel_lines(ScrollDelta::Lines { x: 0.0, y: 1.0 }, 20.0, &mut carry, WHEEL_LINES), 3);
        assert_eq!(wheel_lines(ScrollDelta::Lines { x: 0.0, y: -2.0 }, 20.0, &mut carry, WHEEL_LINES), -6);
        assert_eq!(wheel_lines(ScrollDelta::Lines { x: 0.0, y: -2.0 }, 20.0, &mut carry, 1), -2);
        assert_eq!(wheel_lines(ScrollDelta::Pixels { x: 0.0, y: 15.0 }, 20.0, &mut carry, 1), 0);
        assert_eq!(wheel_lines(ScrollDelta::Pixels { x: 0.0, y: 30.0 }, 20.0, &mut carry, 1), 2);
    }

    #[test]
    fn pastes_are_bracketed_only_when_asked_and_cannot_escape() {
        assert_eq!(paste("a\nb", TermMode::empty()), b"a\rb");
        assert_eq!(paste("a\r\nb", TermMode::BRACKETED_PASTE), b"\x1b[200~a\rb\x1b[201~");
        assert_eq!(bracketed("a\x1b[201~rm -rf /\n"), b"\x1b[200~arm -rf /\n\x1b[201~".to_vec());
    }
}
