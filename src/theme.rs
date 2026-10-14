//! Colors and widget styles. Default palette: LAGC Tech Light.

use iced::widget::{button, container, text_input};
use iced::font::{Family, Weight};
use iced::{Background, Border, Color, Font, Theme, color, theme::Palette};

pub const BG: Color = color!(0xEFF6F7);
pub const SIDEBAR: Color = color!(0xE3EEF1);
pub const SECTION: Color = color!(0xD4E3E8);
pub const BORDER: Color = color!(0xB2CBD3);
pub const FG: Color = color!(0x10242C);
pub const MUTED: Color = color!(0x37505C);
pub const LINE_NO: Color = color!(0x526571);
pub const ACCENT: Color = color!(0x12717C);
pub const SELECTION: Color = color!(0x8CC4CC);
pub const MODIFIED: Color = color!(0xB0662E);
pub const DELETED: Color = color!(0xC9614B);
pub const ADDED: Color = color!(0x12717C);

pub const INSERT_LINE: Color = color!(0x12717C, 0.13);
pub const INSERT_TEXT: Color = color!(0x12717C, 0.28);
pub const DELETE_LINE: Color = color!(0xC9614B, 0.13);
pub const DELETE_TEXT: Color = color!(0xC9614B, 0.28);
pub const FILLER: Color = color!(0xB2CBD3, 0.25);

/// UI font. Medium weight: the regular cut renders too thin on light themes.
/// Override the family with `MIRA_FONT`.
pub fn ui_font() -> Font {
    font_from_env("MIRA_FONT", "Inter")
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

pub fn theme() -> Theme {
    Theme::custom(
        "LAGC Tech Light",
        Palette {
            background: BG,
            text: FG,
            primary: ACCENT,
            success: ADDED,
            warning: MODIFIED,
            danger: DELETED,
        },
    )
}

/// Color for a git status letter.
pub fn status_color(code: char) -> Color {
    match code {
        'M' | 'R' | 'C' | 'T' => MODIFIED,
        'D' => DELETED,
        'A' | 'U' | '?' => ADDED,
        _ => MUTED,
    }
}

pub fn panel(bg: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(bg.into()),
        text_color: Some(FG),
        ..Default::default()
    }
}

pub fn bordered(bg: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(bg.into()),
        text_color: Some(FG),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

pub fn bg(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(color)),
        ..Default::default()
    }
}

/// Flat list row / icon button. `active` keeps it highlighted.
pub fn flat(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let background = match (active, status) {
            (true, _) => Some(SELECTION.scale_alpha(0.7).into()),
            (false, button::Status::Hovered | button::Status::Pressed) => {
                Some(SECTION.into())
            }
            _ => None,
        };
        button::Style {
            background,
            text_color: FG,
            border: Border::default().rounded(3),
            ..Default::default()
        }
    }
}

/// `flat(false)` as a plain fn, so it can share a type with `danger`.
pub fn flat_fn(t: &Theme, status: button::Status) -> button::Style {
    flat(false)(t, status)
}

/// Destructive action awaiting confirmation.
pub fn danger(_: &Theme, status: button::Status) -> button::Style {
    let base = match status {
        button::Status::Hovered | button::Status::Pressed => color!(0xB04F3B),
        button::Status::Disabled => DELETED.scale_alpha(0.45),
        button::Status::Active => DELETED,
    };
    button::Style {
        background: Some(base.into()),
        text_color: BG,
        border: Border::default().rounded(4),
        ..Default::default()
    }
}

pub fn tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: Some(
            if active {
                BG
            } else if matches!(status, button::Status::Hovered) {
                SECTION
            } else {
                SIDEBAR
            }
            .into(),
        ),
        text_color: if active { FG } else { MUTED },
        border: Border {
            color: if active { ACCENT } else { Color::TRANSPARENT },
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    let base = match status {
        button::Status::Hovered | button::Status::Pressed => color!(0x0E5F68),
        button::Status::Disabled => ACCENT.scale_alpha(0.45),
        button::Status::Active => ACCENT,
    };
    button::Style {
        background: Some(base.into()),
        text_color: BG,
        border: Border::default().rounded(4),
        ..Default::default()
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: BG.into(),
        border: Border {
            color: if focused { ACCENT } else { BORDER },
            width: 1.0,
            radius: 4.0.into(),
        },
        icon: MUTED,
        placeholder: LINE_NO,
        value: FG,
        selection: SELECTION,
    }
}

pub fn terminal_palette() -> iced_term::ColorPalette {
    let s = |v: &str| v.to_string();
    iced_term::ColorPalette {
        foreground: s("#1C3038"),
        background: s("#EFF6F7"),
        black: s("#0C1C28"),
        red: s("#B15837"),
        green: s("#23838C"),
        yellow: s("#CD6C44"),
        blue: s("#3B6ADF"),
        magenta: s("#294DAB"),
        cyan: s("#1B656B"),
        white: s("#D9ECF0"),
        bright_black: s("#5E707C"),
        bright_red: s("#E48458"),
        bright_green: s("#34BFCB"),
        bright_yellow: s("#EFB79D"),
        bright_blue: s("#628AE5"),
        bright_magenta: s("#81A0EA"),
        bright_cyan: s("#74D1D9"),
        bright_white: s("#E2E2E2"),
        bright_foreground: None,
        dim_foreground: s("#46606D"),
        dim_black: s("#0C1C28"),
        dim_red: s("#B15837"),
        dim_green: s("#23838C"),
        dim_yellow: s("#CD6C44"),
        dim_blue: s("#3B6ADF"),
        dim_magenta: s("#294DAB"),
        dim_cyan: s("#1B656B"),
        dim_white: s("#D9ECF0"),
    }
}
