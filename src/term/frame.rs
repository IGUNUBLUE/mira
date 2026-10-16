//! What the terminal widget draws: a snapshot of the visible rows only (never
//! the scrollback), with colors already resolved against mira's palette and
//! the program's own overrides (OSC 4/10/11/12), plus the cursor.

use crate::theme::Palette;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor, Rgb};
use std::ops::Range;

/// How a cell's text is decorated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curl,
    Dotted,
    Dashed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    /// Zero-width characters drawn on top of `c` (combining accents, ZWJ emoji).
    pub extra: Option<Box<str>>,
    pub fg: Rgb,
    /// `None`: the terminal's background (not drawn per cell).
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub hidden: bool,
    /// Spans this cell and the next one (CJK, emoji).
    pub wide: bool,
    /// The second half of a wide character: nothing to draw.
    pub spacer: bool,
    pub underline: Underline,
    pub underline_color: Option<Rgb>,
}

impl Cell {
    fn blank(fg: Rgb) -> Self {
        Self {
            c: ' ',
            extra: None,
            fg,
            bg: None,
            bold: false,
            italic: false,
            strike: false,
            hidden: false,
            wide: false,
            spacer: false,
            underline: Underline::None,
            underline_color: None,
        }
    }

    /// Nothing visible: a space with the terminal's background.
    pub fn is_empty(&self) -> bool {
        (self.c == ' ' || self.hidden) && self.bg.is_none() && self.underline == Underline::None && !self.strike
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub line: usize,
    pub col: usize,
    pub shape: CursorShape,
    /// On a wide character: two cells wide.
    pub wide: bool,
    /// The cursor's color, and the color of the character under a block cursor.
    pub color: Rgb,
    pub text: Rgb,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub cols: usize,
    pub lines: usize,
    /// `lines × cols`, row by row.
    pub cells: Vec<Cell>,
    pub background: Rgb,
    pub cursor: Option<Cursor>,
    pub mode: TermMode,
    /// Lines scrolled back into the history (0: at the bottom).
    pub display_offset: usize,
    /// Lines of history kept.
    pub history: usize,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            cols: 0,
            lines: 0,
            cells: Vec::new(),
            background: Rgb { r: 0, g: 0, b: 0 },
            cursor: None,
            mode: TermMode::empty(),
            display_offset: 0,
            history: 0,
        }
    }
}

impl Frame {
    pub fn row(&self, line: usize) -> &[Cell] {
        let start = line * self.cols;
        self.cells.get(start..start + self.cols).unwrap_or(&[])
    }
}

pub fn to_rgb(c: iced::Color) -> Rgb {
    let [r, g, b, _] = c.into_rgba8();
    Rgb { r, g, b }
}

fn hex(s: &str) -> Rgb {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
    Rgb { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 }
}

/// `a` moved `t` of the way towards `b`.
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Rgb { r: m(a.r, b.r), g: m(a.g, b.g), b: m(a.b, b.b) }
}

/// Dim text: a third of the way to the background, as Alacritty does.
fn dim(c: Rgb, bg: Rgb) -> Rgb {
    mix(c, bg, 0.34)
}

/// mira's color for an index of alacritty's table: 0–15 the palette's ANSI
/// colors, 16–255 the xterm cube and greys, then foreground, background,
/// cursor, the dim colors and the bright/dim foreground.
pub fn palette_rgb(index: usize, p: &Palette) -> Rgb {
    let (fg, bg) = (to_rgb(p.fg), to_rgb(p.bg));
    match index {
        0..=15 => hex(p.ansi[index]),
        16..=231 => {
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let i = index - 16;
            Rgb { r: LEVELS[i / 36], g: LEVELS[(i / 6) % 6], b: LEVELS[i % 6] }
        }
        232..=255 => {
            let g = (8 + 10 * (index - 232)) as u8;
            Rgb { r: g, g, b: g }
        }
        257 => bg,
        // DimBlack..DimWhite.
        259..=266 => dim(hex(p.ansi[index - 259]), bg),
        268 => to_rgb(p.muted),
        // Foreground, cursor, bright foreground.
        _ => fg,
    }
}

/// The color the program asked for, after its overrides, else mira's.
fn resolve(color: AnsiColor, colors: &Colors, p: &Palette) -> Rgb {
    match color {
        AnsiColor::Spec(rgb) => rgb,
        AnsiColor::Indexed(i) => colors[i as usize].unwrap_or_else(|| palette_rgb(i as usize, p)),
        AnsiColor::Named(n) => colors[n].unwrap_or_else(|| palette_rgb(n as usize, p)),
    }
}

/// The dim variant of a color: the named dim color for the 8 normal ones.
fn dimmed(color: AnsiColor) -> Option<AnsiColor> {
    const DIM: [NamedColor; 8] = [
        NamedColor::DimBlack,
        NamedColor::DimRed,
        NamedColor::DimGreen,
        NamedColor::DimYellow,
        NamedColor::DimBlue,
        NamedColor::DimMagenta,
        NamedColor::DimCyan,
        NamedColor::DimWhite,
    ];
    match color {
        AnsiColor::Named(n) => DIM.get(n as usize).map(|d| AnsiColor::Named(*d)),
        _ => None,
    }
}

fn underline(flags: Flags) -> Underline {
    if flags.contains(Flags::UNDERCURL) {
        Underline::Curl
    } else if flags.contains(Flags::DOUBLE_UNDERLINE) {
        Underline::Double
    } else if flags.contains(Flags::DOTTED_UNDERLINE) {
        Underline::Dotted
    } else if flags.contains(Flags::DASHED_UNDERLINE) {
        Underline::Dashed
    } else if flags.contains(Flags::UNDERLINE) {
        Underline::Single
    } else {
        Underline::None
    }
}

/// The visible part of `term`, selection included; `links` are (line,
/// columns) to underline (a hovered link).
pub fn build<T: EventListener>(term: &Term<T>, p: &Palette, links: &[(usize, Range<usize>)]) -> Frame {
    let content = term.renderable_content();
    let colors = content.colors;
    let (cols, lines) = (term.columns(), term.screen_lines());
    let background = colors[NamedColor::Background].unwrap_or_else(|| to_rgb(p.bg));
    let foreground = colors[NamedColor::Foreground].unwrap_or_else(|| to_rgb(p.fg));
    let selection_bg = to_rgb(p.selection);
    let offset = content.display_offset;
    let mut cells = vec![Cell::blank(foreground); cols * lines];
    let selection = content.selection;

    for indexed in content.display_iter {
        // The display iterator stays on screen; checked anyway, so a cell can never land outside.
        let (line, col) =
            (usize::try_from(indexed.point.line.0 + offset as i32).unwrap_or(lines), indexed.point.column.0);
        let Some(slot) = (line < lines && col < cols).then_some(line * cols + col) else { continue };
        let cell = &indexed.cell;
        let flags = cell.flags;
        let fg_color = if flags.contains(Flags::DIM) { dimmed(cell.fg).unwrap_or(cell.fg) } else { cell.fg };
        let mut fg = resolve(fg_color, colors, p);
        if flags.contains(Flags::DIM) && dimmed(cell.fg).is_none() {
            fg = dim(fg, background);
        }
        let mut bg = match cell.bg {
            AnsiColor::Named(NamedColor::Background) => None,
            c => Some(resolve(c, colors, p)),
        };
        if flags.contains(Flags::INVERSE) {
            let old_bg = bg.unwrap_or(background);
            bg = Some(fg);
            fg = old_bg;
        }
        if selection.is_some_and(|s| s.contains(indexed.point)) {
            bg = Some(selection_bg);
        }
        let extra = cell.zerowidth().filter(|z| !z.is_empty()).map(|z| z.iter().collect::<String>().into_boxed_str());
        let mut u = underline(flags);
        if links.iter().any(|(l, r)| *l == line && r.contains(&col)) {
            u = Underline::Single;
        }
        cells[slot] = Cell {
            c: cell.c,
            extra,
            fg,
            bg,
            bold: flags.contains(Flags::BOLD),
            italic: flags.contains(Flags::ITALIC),
            strike: flags.contains(Flags::STRIKEOUT),
            hidden: flags.contains(Flags::HIDDEN),
            wide: flags.contains(Flags::WIDE_CHAR),
            spacer: flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
            underline: u,
            underline_color: cell.underline_color().map(|c| resolve(c, colors, p)),
        };
    }

    let cursor = (content.cursor.shape != CursorShape::Hidden)
        .then(|| {
            let line = usize::try_from(content.cursor.point.line.0 + offset as i32).ok()?;
            let col = content.cursor.point.column.0;
            let under = cells.get(line * cols + col).filter(|_| line < lines && col < cols)?;
            // Like Alacritty's default: the cursor takes the text's color and the
            // character under it the cell's background, so it shows on any line.
            let color = colors[NamedColor::Cursor].unwrap_or(under.fg);
            Some(Cursor {
                line,
                col,
                shape: content.cursor.shape,
                wide: under.wide,
                color,
                text: under.bg.unwrap_or(background),
            })
        })
        .flatten();

    Frame {
        cols,
        lines,
        cells,
        background,
        cursor,
        mode: content.mode,
        display_offset: offset,
        history: term.grid().history_size(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::PALETTES;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::Processor;

    fn term(cols: usize, lines: usize, input: &str) -> Term<VoidListener> {
        let mut term = Term::new(Config::default(), &TermSize::new(cols, lines), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, input.as_bytes());
        term
    }

    fn p() -> &'static Palette {
        &PALETTES[0]
    }

    /// The snapshot copies only the visible rows: with a full history it must
    /// beat cloning the grid (what iced_term did on every sync) by far. Timings:
    /// `MIRA_BENCH=1 cargo test --release snapshot_beats -- --nocapture`.
    #[test]
    fn snapshot_beats_cloning_the_grid() {
        let full = std::env::var_os("MIRA_BENCH").is_some();
        let mut text = String::new();
        for i in 0..if full { 10_050 } else { 2_050 } {
            text.push_str(&format!("\x1b[3{}m{i:>6} lorem ipsum \x1b[44m dolor \x1b[0m sit amet\r\n", i % 8));
        }
        let mut t = Term::new(Config::default(), &TermSize::new(120, 40), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut t, text.as_bytes());
        let n = if full { 1000 } else { 20 };
        let time = |f: &dyn Fn()| {
            let start = std::time::Instant::now();
            (0..n).for_each(|_| f());
            start.elapsed() / n
        };
        let ours = time(&|| drop(std::hint::black_box(build(&t, p(), &[]))));
        let clone = time(&|| drop(std::hint::black_box(t.grid().clone())));
        println!("visible rows: {ours:?}; whole grid: {clone:?}");
        assert!(ours * 3 < clone, "{ours:?} vs {clone:?}");
    }

    #[test]
    fn xterm_colors_dim_variants_and_every_underline() {
        assert_eq!(palette_rgb(16 + 36 + 6 + 1, p()), Rgb { r: 95, g: 95, b: 95 });
        assert_eq!(palette_rgb(196, p()), Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(palette_rgb(244, p()), Rgb { r: 128, g: 128, b: 128 });
        assert_eq!(palette_rgb(260, p()), dim(hex(p().ansi[1]), to_rgb(p().bg)));
        assert_eq!(palette_rgb(268, p()), to_rgb(p().muted));
        // Indexed and direct colors, dimmed by mixing when there is no named dim color.
        let t = term(6, 1, "\x1b[38;5;196mA\x1b[2;38;2;200;0;0mB\x1b[0m\x1b[4:2mC\x1b[4:4mD\x1b[4:5mE");
        let r = build(&t, p(), &[]);
        let r = r.row(0);
        assert_eq!(r[0].fg, Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(r[1].fg, dim(Rgb { r: 200, g: 0, b: 0 }, to_rgb(p().bg)));
        assert_eq!(
            [r[2].underline, r[3].underline, r[4].underline],
            [Underline::Double, Underline::Dotted, Underline::Dashed]
        );
        // An underline in its own color (SGR 58).
        let t = term(2, 1, "\x1b[4;58;2;1;2;3mx");
        assert_eq!(build(&t, p(), &[]).row(0)[0].underline_color, Some(Rgb { r: 1, g: 2, b: 3 }));
    }

    #[test]
    fn the_selection_is_highlighted() {
        use alacritty_terminal::index::{Column, Line, Point, Side};
        use alacritty_terminal::selection::{Selection, SelectionType};
        let mut t = term(8, 1, "abcdef");
        let mut sel = Selection::new(SelectionType::Simple, Point::new(Line(0), Column(1)), Side::Left);
        sel.update(Point::new(Line(0), Column(2)), Side::Right);
        t.selection = Some(sel);
        let f = build(&t, p(), &[]);
        let selected: Vec<bool> = f.row(0).iter().map(|c| c.bg == Some(to_rgb(p().selection))).collect();
        assert_eq!(selected, [false, true, true, false, false, false, false, false]);
    }

    #[test]
    fn text_colors_and_the_default_background() {
        let t = term(10, 3, "ab\x1b[31mc\x1b[0m\r\n\x1b[44m \x1b[0m");
        let f = build(&t, p(), &[]);
        assert_eq!((f.cols, f.lines, f.cells.len()), (10, 3, 30));
        assert_eq!(f.row(0)[0].c, 'a');
        assert_eq!(f.row(0)[0].fg, to_rgb(p().fg));
        assert_eq!(f.row(0)[0].bg, None, "default background is not drawn per cell");
        assert_eq!(f.row(0)[2].fg, hex(p().ansi[1]));
        assert_eq!(f.row(1)[0].bg, Some(hex(p().ansi[4])));
        assert!(f.row(2).iter().all(Cell::is_empty));
    }

    #[test]
    fn program_overrides_win_over_the_palette() {
        // OSC 4 sets color 1; OSC 11 the background.
        let t = term(5, 1, "\x1b]4;1;rgb:12/34/56\x07\x1b]11;rgb:01/02/03\x07\x1b[31mx");
        let f = build(&t, p(), &[]);
        assert_eq!(f.row(0)[0].fg, Rgb { r: 0x12, g: 0x34, b: 0x56 });
        assert_eq!(f.background, Rgb { r: 1, g: 2, b: 3 });
    }

    #[test]
    fn inverse_dim_and_styles() {
        let t = term(8, 1, "\x1b[7ma\x1b[0m\x1b[2;31mb\x1b[0m\x1b[1;3;9;4mc");
        let f = build(&t, p(), &[]);
        let r = f.row(0);
        assert_eq!(r[0].bg, Some(to_rgb(p().fg)), "inverse swaps");
        assert_eq!(r[0].fg, to_rgb(p().bg));
        assert_eq!(r[1].fg, dim(hex(p().ansi[1]), to_rgb(p().bg)));
        assert!(r[2].bold && r[2].italic && r[2].strike);
        assert_eq!(r[2].underline, Underline::Single);
        let curly = term(4, 1, "\x1b[4:3mx");
        assert_eq!(build(&curly, p(), &[]).row(0)[0].underline, Underline::Curl);
    }

    #[test]
    fn wide_characters_and_combining_marks() {
        let t = term(6, 1, "日e\u{301}");
        let f = build(&t, p(), &[]);
        let r = f.row(0);
        assert!(r[0].wide && r[0].c == '日');
        assert!(r[1].spacer);
        assert_eq!((r[2].c, r[2].extra.as_deref()), ('e', Some("\u{301}")));
    }

    #[test]
    fn the_cursor_shows_on_colored_lines() {
        // A line with its own background, as Codex's composer has: the
        // cursor takes the text color, the character under it the line's.
        let t = term(10, 2, "\x1b[48;2;229;236;237m> hi\x1b[K");
        let f = build(&t, p(), &[]);
        let c = f.cursor.expect("cursor visible");
        assert_eq!((c.line, c.col, c.shape), (0, 4, CursorShape::Block));
        assert_eq!(c.color, to_rgb(p().fg));
        assert_eq!(c.text, Rgb { r: 229, g: 236, b: 237 });
        // Hidden (DECTCEM) and bar shapes.
        assert!(build(&term(4, 1, "\x1b[?25l"), p(), &[]).cursor.is_none());
        assert_eq!(build(&term(4, 1, "\x1b[6 q"), p(), &[]).cursor.map(|c| c.shape), Some(CursorShape::Beam));
    }

    #[test]
    fn only_the_visible_rows_scrolled_back() {
        let lines: String = (0..10).map(|i| format!("{i}\r\n")).collect();
        let mut t = term(4, 3, &lines);
        let f = build(&t, p(), &[]);
        assert_eq!((f.history, f.display_offset), (8, 0));
        assert_eq!(f.row(0)[0].c, '8');
        t.scroll_display(alacritty_terminal::grid::Scroll::Delta(2));
        let f = build(&t, p(), &[]);
        assert_eq!((f.display_offset, f.row(0)[0].c, f.cells.len()), (2, '6', 12));
        // The cursor is below the view now.
        assert!(f.cursor.is_none());
    }

    #[test]
    fn hovered_links_are_underlined() {
        let t = term(20, 1, "see https://x.org ok");
        let f = build(&t, p(), &[(0, 4..17)]);
        assert_eq!(f.row(0)[4].underline, Underline::Single);
        assert_eq!(f.row(0)[18].underline, Underline::None);
    }
}
