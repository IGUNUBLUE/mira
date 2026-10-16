//! Host-terminal duties alacritty_terminal leaves to the terminal app: the
//! light/dark report herdr reads to pick its theme. (Keys, mouse, paste and
//! color queries are handled in `term`.)

use crate::theme;

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
}
