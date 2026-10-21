//! Colors and widget styles. Palettes: the four LAGC themes (the author's
//! LAGC themes for VSCodium and kitty). `auto` follows the active HyDE theme.
//!
//! The current palette is a process-wide setting read through [`p`], so style
//! closures always draw with the palette active at draw time.

use iced::font::{Family, Weight};
use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Font, Theme, color};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::SystemTime;

pub struct Palette {
    pub name: &'static str,
    pub bg: Color,
    pub sidebar: Color,
    pub section: Color,
    pub border: Color,
    pub fg: Color,
    pub muted: Color,
    pub line_no: Color,
    pub accent: Color,
    /// Primary button background and its hover.
    pub button: Color,
    pub button_hover: Color,
    /// Text on accent/danger backgrounds.
    pub on_accent: Color,
    pub selection: Color,
    pub modified: Color,
    pub deleted: Color,
    pub added: Color,
    /// `modified` / `deleted` / `added` for text: the same hues, darkened on
    /// light palettes so small text reaches 4.5:1 on `bg` and `sidebar`.
    pub warn_text: Color,
    pub error_text: Color,
    pub added_text: Color,
    pub insert_line: Color,
    pub insert_text: Color,
    pub delete_line: Color,
    pub delete_text: Color,
    pub filler: Color,
    pub syntax: iced::highlighter::Theme,
    /// ANSI 0–15, from the matching kitty theme.
    pub ansi: [&'static str; 16],
}

const fn a(c: Color, alpha: f32) -> Color {
    Color { a: alpha, ..c }
}

pub const PALETTES: [Palette; 4] = [
    Palette {
        name: "LAGC Tech Light",
        bg: color!(0xEFF6F7),
        sidebar: color!(0xE3EEF1),
        section: color!(0xD4E3E8),
        border: color!(0xB2CBD3),
        fg: color!(0x10242C),
        muted: color!(0x37505C),
        line_no: color!(0x526571),
        accent: color!(0x12717C),
        button: color!(0x12717C),
        button_hover: color!(0x0E5F68),
        on_accent: color!(0xEFF6F7),
        selection: color!(0x8CC4CC),
        modified: color!(0xB0662E),
        deleted: color!(0xC9614B),
        added: color!(0x12717C),
        warn_text: color!(0x9A5928),
        error_text: color!(0xAF4A35),
        added_text: color!(0x12717C),
        insert_line: a(color!(0x12717C), 0.13),
        insert_text: a(color!(0x12717C), 0.28),
        delete_line: a(color!(0xC9614B), 0.13),
        delete_text: a(color!(0xC9614B), 0.28),
        filler: a(color!(0xB2CBD3), 0.25),
        syntax: iced::highlighter::Theme::InspiredGitHub,
        ansi: [
            "#435763", "#E48458", "#23838C", "#B15837", "#3B6ADF", "#3057B5", "#375967", "#0C1C28", "#5E707C",
            "#EA9E7B", "#34BFCB", "#CD6C44", "#628AE5", "#81A0EA", "#74D1D9", "#D9ECF0",
        ],
    },
    Palette {
        name: "LAGC Tech Dark",
        bg: color!(0x0C1C28),
        sidebar: color!(0x122638),
        section: color!(0x193448),
        border: color!(0x204756),
        fg: color!(0xDDEEF2),
        muted: color!(0xB1C2C9),
        line_no: color!(0x7A8E99),
        accent: color!(0x34BFCB),
        button: color!(0x2CA1AC),
        button_hover: color!(0x34BFCB),
        on_accent: color!(0x0C1C28),
        selection: color!(0x23838C),
        modified: color!(0xE0A35E),
        deleted: color!(0xE48458),
        added: color!(0x34BFCB),
        warn_text: color!(0xE0A35E),
        error_text: color!(0xE48458),
        added_text: color!(0x34BFCB),
        insert_line: a(color!(0x34BFCB), 0.13),
        insert_text: a(color!(0x34BFCB), 0.27),
        delete_line: a(color!(0xE48458), 0.13),
        delete_text: a(color!(0xE48458), 0.27),
        filler: a(color!(0x204756), 0.35),
        syntax: iced::highlighter::Theme::Base16Ocean,
        ansi: [
            "#122638", "#E48458", "#34BFCB", "#EFB79D", "#628AE5", "#9ADFE4", "#B1C2C9", "#DDEEF2", "#435763",
            "#EA9E7B", "#74D1D9", "#F6D8CB", "#81A0EA", "#C2D0F5", "#C7F0F3", "#D9ECF0",
        ],
    },
    Palette {
        name: "LAGC Calm Light",
        bg: color!(0xF4EEE2),
        sidebar: color!(0xEAE3D5),
        section: color!(0xDDD2BE),
        border: color!(0xC9BCA6),
        fg: color!(0x2E2923),
        muted: color!(0x574E43),
        line_no: color!(0x7A6C5C),
        accent: color!(0x5C7040),
        button: color!(0x5C7040),
        button_hover: color!(0x4A5B33),
        on_accent: color!(0xF4EEE2),
        selection: color!(0xCBDCA8),
        modified: color!(0xA06830),
        deleted: color!(0xB05A4E),
        added: color!(0x5C7040),
        warn_text: color!(0x885929),
        error_text: color!(0x994E44),
        added_text: color!(0x576A3C),
        insert_line: a(color!(0x5C7040), 0.13),
        insert_text: a(color!(0x5C7040), 0.28),
        delete_line: a(color!(0xB05A4E), 0.13),
        delete_text: a(color!(0xB05A4E), 0.28),
        filler: a(color!(0xC9BCA6), 0.3),
        syntax: iced::highlighter::Theme::InspiredGitHub,
        ansi: [
            "#DDD2BE", "#B05A4E", "#5C7040", "#8A5A1E", "#55655E", "#9E5F4A", "#6B8350", "#3A342C", "#8C7D6C",
            "#C96B5C", "#7F9A5E", "#A06830", "#71807A", "#B07156", "#8FA66B", "#FDF9EF",
        ],
    },
    Palette {
        name: "LAGC Calm Dark",
        bg: color!(0x2B2522),
        sidebar: color!(0x352E29),
        section: color!(0x3E362F),
        border: color!(0x4A423B),
        fg: color!(0xE9DFCE),
        muted: color!(0xB5A796),
        line_no: color!(0x8C7D6C),
        accent: color!(0xA9C080),
        button: color!(0x7F9A5E),
        button_hover: color!(0xA9C080),
        on_accent: color!(0x2B2522),
        selection: color!(0x455437),
        modified: color!(0xD4A55C),
        deleted: color!(0xD67A6E),
        added: color!(0xA9C080),
        warn_text: color!(0xD4A55C),
        error_text: color!(0xD88176),
        added_text: color!(0xA9C080),
        insert_line: a(color!(0xA9C080), 0.13),
        insert_text: a(color!(0xA9C080), 0.27),
        delete_line: a(color!(0xD67A6E), 0.13),
        delete_text: a(color!(0xD67A6E), 0.27),
        filler: a(color!(0x4A423B), 0.35),
        syntax: iced::highlighter::Theme::Base16Mocha,
        ansi: [
            "#352E29", "#D67A6E", "#A9C080", "#D4A55C", "#8FA8A0", "#C98A6B", "#9DB8A0", "#E9DFCE", "#6B5F53",
            "#E0877B", "#B9CC94", "#E0B876", "#A8BDB0", "#D8A188", "#ADD0B8", "#F4EEE2",
        ],
    },
];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The active palette.
pub fn p() -> &'static Palette {
    &PALETTES[CURRENT.load(Ordering::Relaxed).min(PALETTES.len() - 1)]
}

/// Index of the active palette (part of cached views' dependencies: their
/// colors are read when they are built).
pub fn current() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

/// Make palette `i` active. Returns whether it changed.
pub fn set_current(i: usize) -> bool {
    CURRENT.swap(i.min(PALETTES.len() - 1), Ordering::Relaxed) != i
}

// ------------------------------------------------------------------ choice

/// What the user picked: follow HyDE, or a fixed palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Auto,
    Fixed(usize),
}

impl Choice {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("auto") {
            return Some(Self::Auto);
        }
        PALETTES.iter().position(|p| p.name.eq_ignore_ascii_case(s)).map(Self::Fixed)
    }

    pub fn label(self) -> String {
        match self {
            Self::Auto => "auto".into(),
            Self::Fixed(i) => PALETTES[i].name.into(),
        }
    }

    /// Auto → each palette → Auto.
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Fixed(0),
            Self::Fixed(i) if i + 1 < PALETTES.len() => Self::Fixed(i + 1),
            Self::Fixed(_) => Self::Auto,
        }
    }

    /// Palette index this choice means right now.
    pub fn resolve(self) -> usize {
        match self {
            Self::Fixed(i) => i,
            Self::Auto => hyde_theme().and_then(|t| PALETTES.iter().position(|p| p.name == t)).unwrap_or(0),
        }
    }
}

/// Active HyDE theme name, from its state file (live) or the environment.
fn hyde_theme() -> Option<String> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?
        .join("hyde/staterc");
    staterc_theme(&state).or_else(|| std::env::var("HYDE_THEME").ok()).filter(|t| !t.is_empty())
}

/// `HYDE_THEME` from HyDE's staterc. Asked every tick, so the file is only
/// read again when its mtime changes.
fn staterc_theme(path: &Path) -> Option<String> {
    type Cached = Option<(PathBuf, Option<SystemTime>, Option<String>)>;
    static CACHE: Mutex<Cached> = Mutex::new(None);
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((p, m, theme)) = cache.as_ref()
        && p == path
        && *m == mtime
    {
        return theme.clone();
    }
    let theme = std::fs::read_to_string(path).ok().and_then(|text| {
        text.lines().find_map(|l| l.strip_prefix("HYDE_THEME=")).map(|v| v.trim().trim_matches('"').to_string())
    });
    *cache = Some((path.to_path_buf(), mtime, theme.clone()));
    theme
}

// ------------------------------------------------------------------- fonts

/// UI font. Medium weight: the regular cut renders too thin on light themes.
/// Override the family with `MIRA_FONT`.
pub fn ui_font() -> Font {
    static FONT: LazyLock<Font> = LazyLock::new(|| font_from_env("MIRA_FONT", "Inter"));
    *FONT
}

/// Code and terminal font. Override with `MIRA_MONO_FONT`.
pub fn mono_font() -> Font {
    font_from_env("MIRA_MONO_FONT", "JetBrains Mono")
}

fn font_from_env(var: &str, default: &'static str) -> Font {
    let family: &'static str = match std::env::var(var) {
        Ok(v) if !v.trim().is_empty() => Box::leak(v.trim().to_string().into_boxed_str()),
        _ => default,
    };
    Font { family: Family::Name(family), weight: Weight::Medium, ..Font::DEFAULT }
}

// ------------------------------------------------------------------ styles

pub fn theme() -> Theme {
    let c = p();
    Theme::custom(
        c.name,
        iced::theme::Palette {
            background: c.bg,
            text: c.fg,
            primary: c.accent,
            success: c.added,
            warning: c.modified,
            danger: c.deleted,
        },
    )
}

/// Text color for a git status letter (`!`: merge conflict).
pub fn status_color(code: char) -> Color {
    let c = p();
    match code {
        'M' | 'R' | 'C' | 'T' => c.warn_text,
        'D' | '!' => c.error_text,
        'A' | 'U' | '?' => c.added_text,
        _ => c.muted,
    }
}

pub fn panel(bg: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: Some(bg.into()), text_color: Some(p().fg), ..Default::default() }
}

pub fn bordered(bg: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(bg.into()),
        text_color: Some(p().fg),
        border: Border { color: p().border, width: 1.0, radius: 4.0.into() },
        ..Default::default()
    }
}

pub fn bg(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: Some(Background::Color(color)), ..Default::default() }
}

/// Flat list row / icon button. `active` keeps it highlighted.
pub fn flat(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let c = p();
        let background = match (active, status) {
            (true, _) => Some(c.selection.scale_alpha(0.7).into()),
            (false, button::Status::Hovered | button::Status::Pressed) => Some(c.section.into()),
            _ => None,
        };
        // Disabled (e.g. while git is busy): dimmed, so it doesn't read as clickable.
        let text_color = if status == button::Status::Disabled { c.muted.scale_alpha(0.55) } else { c.fg };
        button::Style { background, text_color, border: Border::default().rounded(3), ..Default::default() }
    }
}

/// `flat(false)` as a plain fn, so it can share a type with `danger`.
pub fn flat_fn(t: &Theme, status: button::Status) -> button::Style {
    flat(false)(t, status)
}

/// Outlined icon button (e.g. generate commit message). `busy` keeps the
/// accent border while its action runs.
pub fn outline(busy: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let c = p();
        let (background, border) = match status {
            button::Status::Hovered => (Some(c.section.into()), c.accent),
            button::Status::Pressed => (Some(mix(c.section, c.fg, 0.08).into()), c.accent),
            button::Status::Disabled => (None, c.border.scale_alpha(0.5)),
            button::Status::Active => (Some(c.bg.into()), if busy { c.accent } else { c.border }),
        };
        button::Style {
            background,
            text_color: c.fg,
            border: Border { color: border, width: 1.0, radius: 4.0.into() },
            ..Default::default()
        }
    }
}

fn filled(base: Color, status: button::Status) -> button::Style {
    let c = p();
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => mix(base, c.fg, 0.15),
        button::Status::Disabled => base.scale_alpha(0.45),
        button::Status::Active => base,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: c.on_accent,
        border: Border::default().rounded(4),
        ..Default::default()
    }
}

/// Destructive action awaiting confirmation.
pub fn danger(_: &Theme, status: button::Status) -> button::Style {
    filled(p().deleted, status)
}

pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    let c = p();
    match status {
        button::Status::Hovered | button::Status::Pressed => {
            button::Style { background: Some(c.button_hover.into()), ..filled(c.button, button::Status::Active) }
        }
        s => filled(c.button, s),
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a }
}

pub fn tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let c = p();
        button::Style {
            background: Some(
                if active {
                    c.bg
                } else if matches!(status, button::Status::Hovered) {
                    c.section
                } else {
                    c.sidebar
                }
                .into(),
            ),
            text_color: if active { c.fg } else { c.muted },
            border: Border::default(),
            ..Default::default()
        }
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let c = p();
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: c.bg.into(),
        border: Border { color: if focused { c.accent } else { c.border }, width: 1.0, radius: 4.0.into() },
        icon: c.muted,
        placeholder: c.muted,
        value: c.fg,
        selection: c.selection,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_parses_and_cycles_through_every_palette() {
        assert_eq!(Choice::parse(" lagc tech dark "), Some(Choice::Fixed(1)));
        assert_eq!(Choice::parse("AUTO"), Some(Choice::Auto));
        assert_eq!(Choice::parse("nope"), None);
        let mut c = Choice::Auto;
        for _ in 0..=PALETTES.len() {
            c = c.next();
        }
        assert_eq!(c, Choice::Auto);
    }

    /// WCAG relative luminance.
    fn luminance(c: Color) -> f32 {
        let f = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn text_colors_reach_aa_contrast_in_every_palette() {
        for c in &PALETTES {
            let texts = [
                ("fg", c.fg),
                ("muted", c.muted),
                ("warn_text", c.warn_text),
                ("error_text", c.error_text),
                ("added_text", c.added_text),
            ];
            for (name, fg) in texts {
                for (surface, bg) in [("bg", c.bg), ("sidebar", c.sidebar)] {
                    let ratio = contrast(fg, bg);
                    assert!(ratio >= 4.5, "{}: {name} on {surface} is {ratio:.2}:1", c.name);
                }
            }
        }
    }
}
