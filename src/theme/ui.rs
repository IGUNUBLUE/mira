//! Shared native design primitives for product views and the visual specimen. See design.md.

use crate::theme::{self, Palette};
use iced::widget::{button, container, scrollable, text, text_editor, text_input};
use iced::{Border, Color, Element, Length, Pixels, Theme};

pub const BODY: u32 = 14;
pub const META: u32 = 12;
pub const CONTROL: f32 = 32.0;
pub const COMPACT: f32 = 28.0;
pub const DIALOG_CONTROL: f32 = 36.0;
pub const RADIUS: f32 = 6.0;
pub const GAP: f32 = 8.0;
pub const INSET: f32 = 12.0;
pub const PANEL_INSET: f32 = 16.0;
pub const DIALOG_INSET: f32 = 24.0;
pub const TITLE: u32 = 20;
pub const SECTION: u32 = 16;
pub const MENU_ROW: f32 = 32.0;
pub const BADGE_RADIUS: f32 = 4.0;

/// Includes menu padding, the one-pixel item gaps, and padded separators.
pub const fn menu_height(rows: usize, separators: usize) -> f32 {
    rows as f32 * MENU_ROW + separators as f32 * 7.0 + (rows + separators).saturating_sub(1) as f32 + 8.0
}

#[derive(Clone, Copy, Debug)]
pub enum Variant {
    Primary,
    Secondary,
    Outline,
    Ghost,
    Danger,
}

/// Explicit palette input keeps style checks independent of the global theme selection.
pub fn button_style(c: &Palette, variant: Variant, status: button::Status) -> button::Style {
    let (mut bg, mut fg, mut border) = match variant {
        Variant::Primary => (Some(c.button), c.on_accent, c.button),
        Variant::Secondary => (Some(c.section), c.fg, c.section),
        Variant::Outline => (Some(c.bg), c.fg, c.border),
        Variant::Ghost => (None, c.fg, Color::TRANSPARENT),
        // Text-safe red also provides enough contrast with on_accent on light palettes.
        Variant::Danger => (Some(c.error_text), c.on_accent, c.error_text),
    };
    match status {
        button::Status::Hovered | button::Status::Pressed => match variant {
            Variant::Primary => {
                bg = Some(c.button_hover);
                border = c.button_hover;
            }
            Variant::Danger => border = c.fg,
            _ => {
                bg = Some(c.section);
                border = c.accent;
            }
        },
        button::Status::Disabled => {
            bg = Some(c.sidebar);
            fg = c.muted;
            border = c.border;
        }
        button::Status::Active => {}
    }
    if status == button::Status::Pressed {
        border = c.fg;
    }
    button::Style {
        background: bg.map(Into::into),
        text_color: fg,
        border: Border { color: border, width: 1.0, radius: RADIUS.into() },
        ..Default::default()
    }
}

pub fn action<'a, M: Clone + 'a>(
    label: impl text::IntoFragment<'a>,
    variant: Variant,
    message: Option<M>,
) -> button::Button<'a, M> {
    button(text(label).size(BODY).line_height(1.25).center())
        .height(CONTROL)
        .padding([4, 12])
        .style(move |_, status| button_style(theme::p(), variant, status))
        .on_press_maybe(message)
}

pub fn primary_wide<'a, M: Clone + 'a>(label: &'a str, message: Option<M>) -> button::Button<'a, M> {
    button(text(label).size(BODY).line_height(1.25).width(iced::Length::Fill).center())
        .width(iced::Length::Fill)
        .height(CONTROL)
        .padding([4, 12])
        .style(|_, status| button_style(theme::p(), Variant::Primary, status))
        .on_press_maybe(message)
}

/// One field geometry for search, settings, quick open, and the gallery.
pub fn field<'a, M: Clone + 'a>(placeholder: &str, value: &str) -> text_input::TextInput<'a, M> {
    text_input(placeholder, value)
        .size(BODY)
        .line_height(text::LineHeight::Absolute(Pixels(18.0)))
        .padding([7, 12])
        .style(input)
}

pub fn row_style(c: &Palette, active: bool, status: button::Status) -> button::Style {
    let mut style = button_style(c, Variant::Ghost, status);
    if status != button::Status::Disabled {
        style.background = if active {
            Some(c.bg.into())
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            // Status-colored filenames are contrast-tested on bg/sidebar, not section.
            Some(c.sidebar.into())
        } else {
            None
        };
        style.border.color = if active || status == button::Status::Pressed {
            c.accent
        } else if status == button::Status::Hovered {
            c.border
        } else {
            Color::TRANSPARENT
        };
    } else {
        style.background = None;
        style.border.color = Color::TRANSPARENT;
    }
    style
}

/// Bounded floating surfaces; all callers retain their existing overlay/event ownership.
pub fn floating(_: &Theme) -> container::Style {
    container::Style {
        background: Some(theme::p().bg.into()),
        text_color: Some(theme::p().fg),
        border: Border { color: theme::p().border, width: 1.0, radius: 8.0.into() },
        shadow: iced::Shadow { color: theme::p().shadow, offset: iced::Vector::new(0.0, 4.0), blur_radius: 16.0 },
        ..Default::default()
    }
}

pub fn editor(_: &Theme, status: text_editor::Status) -> text_editor::Style {
    let c = theme::p();
    text_editor::Style {
        background: c.bg.into(),
        border: Border {
            color: if matches!(status, text_editor::Status::Focused { .. }) { c.accent } else { c.border },
            width: 1.0,
            radius: RADIUS.into(),
        },
        placeholder: c.muted,
        value: c.fg,
        selection: c.selection,
    }
}

/// A narrow scrollbar with a reserved gutter, so it never covers row actions.
pub fn scroll<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> scrollable::Scrollable<'a, M> {
    scrollable(content)
        .width(Length::Fill)
        .direction(scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(6).scroller_width(6)))
        .spacing(4)
}

pub fn surface(_: &Theme) -> container::Style {
    let c = theme::p();
    container::Style {
        background: Some(c.bg.into()),
        text_color: Some(c.fg),
        border: Border { color: c.border, width: 1.0, radius: 8.0.into() },
        ..Default::default()
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let c = theme::p();
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: c.bg.into(),
        border: Border { color: if focused { c.accent } else { c.border }, width: 1.0, radius: RADIUS.into() },
        icon: c.muted,
        placeholder: c.muted,
        value: c.fg,
        selection: c.selection,
    }
}

pub fn badge<'a, M: 'a>(label: &'a str) -> Element<'a, M> {
    reference(text(label).size(META).wrapping(text::Wrapping::WordOrGlyph))
}

pub fn reference<'a, M: 'a>(label: impl Into<Element<'a, M>>) -> Element<'a, M> {
    container(label)
        .padding([2, 6])
        .style(|_| container::Style {
            background: Some(theme::p().sidebar.into()),
            text_color: Some(theme::p().fg),
            border: Border { color: theme::p().border, width: 1.0, radius: BADGE_RADIUS.into() },
            ..Default::default()
        })
        .into()
}

/// Selection stays visible independently of pointer hover.
pub fn selected(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let c = theme::p();
        let mut style = button_style(c, Variant::Ghost, status);
        if active {
            style.background = Some(c.bg.into());
            style.border.color = c.border;
        }
        style
    }
}

/// The document tab's outer shell owns selection; its title and close button stay quiet.
pub fn document_tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let c = theme::p();
        let mut style = row_style(c, false, status);
        if !active && status == button::Status::Active {
            style.text_color = c.muted;
        }
        style
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Color) -> f32 {
        let f = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b)
    }

    #[test]
    fn button_labels_reach_aa_in_every_palette_and_state() {
        for palette in &theme::PALETTES {
            for variant in [Variant::Primary, Variant::Secondary, Variant::Outline, Variant::Ghost, Variant::Danger] {
                for state in
                    [button::Status::Active, button::Status::Hovered, button::Status::Pressed, button::Status::Disabled]
                {
                    let style = button_style(palette, variant, state);
                    let bg = match style.background {
                        Some(iced::Background::Color(c)) => c,
                        None => palette.bg,
                        _ => panic!("expected a solid surface"),
                    };
                    let (a, b) = (luminance(style.text_color), luminance(bg));
                    let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                    assert!(contrast >= 4.5, "{} {variant:?} {state:?}: {contrast:.2}:1", palette.name);
                }
            }
        }
    }

    #[test]
    fn semantic_row_text_stays_readable_on_hover_and_selection() {
        for palette in &theme::PALETTES {
            for active in [false, true] {
                for state in
                    [button::Status::Active, button::Status::Hovered, button::Status::Pressed, button::Status::Disabled]
                {
                    let style = row_style(palette, active, state);
                    let bg = match style.background {
                        Some(iced::Background::Color(c)) => c,
                        None => palette.sidebar,
                        _ => panic!("expected a solid row surface"),
                    };
                    for fg in [palette.fg, palette.muted, palette.warn_text, palette.error_text, palette.added_text] {
                        let (a, b) = (luminance(fg), luminance(bg));
                        let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                        assert!(contrast >= 4.5, "{} selected={active} {state:?}: {contrast:.2}:1", palette.name);
                    }
                }
            }
        }
    }

    #[test]
    fn long_references_wrap_within_narrow_inspectors() {
        use iced::advanced::{layout, renderer::Headless, widget::Tree};
        use iced::{Pixels, Size};
        use std::future::Future as _;

        let mut new =
            std::pin::pin!(<iced::Renderer as Headless>::new(theme::ui_font(), Pixels(14.0), Some("tiny-skia")));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(Some(renderer)) = new.as_mut().poll(&mut cx) else {
            panic!("headless renderer");
        };
        // Fonts have different advances: first measure an overflowing fixture, then compare heights.
        let label =
            "origin/feature/preserve-full-branch-labels-in-narrow-inspectors, feature/keep-reference-names-visible";
        let measure = |width| {
            let mut reference: Element<'_, ()> = badge(label);
            let mut tree = Tree::new(&reference);
            reference
                .as_widget_mut()
                .layout(&mut tree, &renderer, &layout::Limits::new(Size::ZERO, Size::new(width, 400.0)))
                .size()
        };
        let unwrapped = measure(4096.0);
        assert!(unwrapped.width > 328.0, "fixture must overflow the widest inspector");
        for width in [140.0, 208.0, 328.0] {
            let size = measure(width);
            assert!(size.width <= width, "reference escaped {width}px");
            assert!(size.height > unwrapped.height, "long reference failed to wrap at {width}px");
            assert!(size.height < 400.0, "reference exceeded the available height");
        }
    }
}
