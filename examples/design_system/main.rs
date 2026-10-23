//! Isolated visual specimen: `WGPU_BACKEND=gl cargo run --example design_system`.
//! Uses static Git fixtures; never starts a terminal, contacts herdr, or reads credentials.

mod components;

// Share the exact product palettes. The specimen deliberately exercises only part of this module.
#[allow(dead_code)]
#[path = "../../src/theme.rs"]
mod theme;

use components::{BODY, COMPACT, GAP, INSET, META, Variant, action, badge, empty_state};
use iced::keyboard::{self, key::Named};
use iced::widget::{column, container, opaque, row, scrollable, space, stack, text, text_input};
use iced::{Alignment, Element, Length, Subscription, Task, window};
use std::path::PathBuf;

#[derive(Clone)]
struct Options {
    theme: usize,
    inspector_width: f32,
    capture: Option<PathBuf>,
    dialog: bool,
}

impl Options {
    fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let theme = std::env::var("MIRA_DESIGN_THEME").unwrap_or_else(|_| "0".into()).parse::<usize>()?;
        let inspector_width = std::env::var("MIRA_DESIGN_WIDTH").unwrap_or_else(|_| "360".into()).parse::<u16>()?;
        if theme >= theme::PALETTES.len() || !(240..=480).contains(&inspector_width) {
            return Err("MIRA_DESIGN_THEME must be 0..3; MIRA_DESIGN_WIDTH must be 240..480".into());
        }
        Ok(Self {
            theme,
            inspector_width: f32::from(inspector_width),
            capture: std::env::var_os("MIRA_DESIGN_CAPTURE").map(PathBuf::from),
            dialog: std::env::var("MIRA_DESIGN_DIALOG").is_ok_and(|v| v == "1"),
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::from_env()?;
    theme::set_current(options.theme);
    iced::application(move || Specimen::new(options.clone()), Specimen::update, Specimen::view)
        .title("mira — Design system preview")
        .theme(|_: &Specimen| theme::theme())
        .default_font(theme::ui_font())
        .subscription(Specimen::subscription)
        .window(window::Settings {
            size: iced::Size::new(1160.0, 1040.0),
            min_size: Some(iced::Size::new(1000.0, 700.0)),
            platform_specific: window::settings::PlatformSpecific {
                application_id: "mira-design-preview".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .antialiasing(true)
        .run()?;
    Ok(())
}

struct Specimen {
    options: Options,
    input: String,
    tab: usize,
    staged: bool,
    busy: bool,
    feedback: &'static str,
}

#[derive(Debug, Clone)]
enum Msg {
    Theme(usize),
    Width(f32),
    Input(String),
    Tab(usize),
    Stage,
    Commit,
    Committed,
    Dialog(bool),
    Dismissed,
    Feedback(&'static str),
    Opened(window::Id),
    Capture(window::Id),
    Captured(window::Screenshot),
    Saved(Result<(), String>),
    Key(keyboard::Event),
}

impl Specimen {
    fn new(options: Options) -> Self {
        Self {
            options,
            input: "Polish the review panel".into(),
            tab: 0,
            staged: false,
            busy: false,
            feedback: "Static fixtures · actions affect this preview only",
        }
    }

    fn subscription(&self) -> Subscription<Msg> {
        Subscription::batch([window::open_events().map(Msg::Opened), keyboard::listen().map(Msg::Key)])
    }

    fn update(&mut self, message: Msg) -> Task<Msg> {
        match message {
            Msg::Theme(i) => {
                self.options.theme = i;
                theme::set_current(i);
            }
            Msg::Width(width) => self.options.inspector_width = width,
            Msg::Input(value) => self.input = value,
            Msg::Tab(tab) => self.tab = tab,
            Msg::Stage => {
                if !self.busy {
                    self.staged = !self.staged;
                }
            }
            Msg::Commit => {
                if self.busy || self.input.trim().is_empty() {
                    return Task::none();
                }
                self.busy = true;
                return Task::perform(
                    async { tokio::time::sleep(std::time::Duration::from_millis(900)).await },
                    |_| Msg::Committed,
                );
            }
            Msg::Committed => {
                self.busy = false;
                self.feedback = "Preview commit completed. No repository was changed.";
            }
            Msg::Dialog(show) => self.options.dialog = show,
            Msg::Dismissed => {
                self.options.dialog = false;
                self.feedback = "Preview discard confirmed. No files were changed.";
            }
            Msg::Feedback(message) => self.feedback = message,
            Msg::Opened(id) if self.options.capture.is_some() => {
                return Task::perform(
                    async { tokio::time::sleep(std::time::Duration::from_millis(700)).await },
                    move |_| Msg::Capture(id),
                );
            }
            Msg::Opened(_) => {}
            Msg::Capture(id) => return window::screenshot(id).map(Msg::Captured),
            Msg::Captured(screenshot) => {
                if let Some(path) = self.options.capture.take() {
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                image::save_buffer(
                                    path,
                                    &screenshot.rgba,
                                    screenshot.size.width,
                                    screenshot.size.height,
                                    image::ColorType::Rgba8,
                                )
                                .map_err(|e| e.to_string())
                            })
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|r| r)
                        },
                        Msg::Saved,
                    );
                }
            }
            Msg::Saved(result) => {
                if let Err(error) = result {
                    eprintln!("design preview capture: {error}");
                    std::process::exit(1);
                }
                return iced::exit();
            }
            Msg::Key(keyboard::Event::KeyPressed { key: keyboard::Key::Named(Named::Escape), .. }) => {
                self.options.dialog = false;
            }
            Msg::Key(_) => {}
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Msg> {
        let themes = row(theme::PALETTES.iter().enumerate().map(|(i, palette)| {
            action(palette.name, Variant::Ghost, Some(Msg::Theme(i)))
                .style(components::selected(i == self.options.theme))
                .into()
        }))
        .spacing(4);
        let header = column![
            row![
                text("mira").size(24),
                text("Visual system").size(24).color(theme::p().muted),
                space::horizontal(),
                badge("Design preview")
            ]
            .spacing(12)
            .align_y(Alignment::Center),
            text("Rust/UI proportions · LAGC colors · compact desktop controls").size(BODY).color(theme::p().muted),
            themes,
        ]
        .spacing(GAP);
        let body = row![
            scrollable(self.catalog())
                .direction(scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(4).scroller_width(4)))
                .width(Length::Fill)
                .height(Length::Fill),
            self.inspector()
        ]
        .spacing(24)
        .height(Length::Fill);
        let base: Element<'_, Msg> =
            container(column![header, body, text(self.feedback).size(META).color(theme::p().muted)].spacing(20))
                .padding(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::panel(theme::p().sidebar))
                .into();
        if self.options.dialog {
            stack![
                base,
                opaque(container(self.dialog()).center(Length::Fill).style(|_| container::Style {
                    background: Some(theme::p().bg.scale_alpha(0.90).into()),
                    ..Default::default()
                }))
            ]
            .into()
        } else {
            base
        }
    }

    fn catalog(&self) -> Element<'_, Msg> {
        let variants = column![
            row![
                action("Primary", Variant::Primary, Some(Msg::Feedback("Primary action"))),
                action("Secondary", Variant::Secondary, Some(Msg::Feedback("Secondary action"))),
                action("Outline", Variant::Outline, Some(Msg::Feedback("Outline action")))
            ]
            .spacing(GAP),
            row![
                action("Ghost", Variant::Ghost, Some(Msg::Feedback("Ghost action"))),
                action("Destructive", Variant::Danger, Some(Msg::Dialog(true))),
                action("Disabled", Variant::Primary, None)
            ]
            .spacing(GAP),
        ]
        .spacing(GAP);
        let sizes = row![
            action("Compact · 28", Variant::Outline, Some(Msg::Feedback("Compact control"))).height(COMPACT),
            action("Default · 32", Variant::Outline, Some(Msg::Feedback("Default control"))),
            action("Dialog · 36", Variant::Outline, Some(Msg::Dialog(true))).height(36),
        ]
        .spacing(GAP)
        .align_y(Alignment::Center);
        let field = column![
            text("Commit message").size(BODY),
            text_input("Describe the change", &self.input)
                .size(BODY)
                .line_height(iced::widget::text::LineHeight::Absolute(iced::Pixels(18.0)))
                .padding([7, 12])
                .style(components::input)
                .on_input(Msg::Input),
            text("Click the field to inspect its focus border.").size(META).color(theme::p().muted),
        ]
        .spacing(GAP);
        let statuses = column![
            row![badge("main"), badge("Staged"), badge("3 files")].spacing(GAP),
            text("Modified · src/view.rs").size(BODY).color(theme::p().warn_text),
            text("Added · design.md").size(BODY).color(theme::p().added_text),
            text("Error · Remote unavailable").size(BODY).color(theme::p().error_text),
        ]
        .spacing(GAP);
        column![
            section("Buttons", "One primary action per working area", variants),
            section("Density", "Shared geometry for every control", sizes),
            section("Fields", "Persistent labels and readable hints", field),
            section("Metadata & status", "Color supports a written label", statuses),
            section(
                "Empty state",
                "A useful explanation within the panel",
                empty_state("Working tree clean", "Saved changes from you or your agents will appear here.")
            ),
        ]
        .spacing(16)
        .padding(iced::Padding { right: 8.0, ..iced::Padding::ZERO })
        .into()
    }

    fn inspector(&self) -> Element<'_, Msg> {
        let tabs = row([("Changes", 0), ("Search", 1), ("Files", 2)].into_iter().map(|(label, index)| {
            action(label, Variant::Ghost, Some(Msg::Tab(index)))
                .padding([4, 8])
                .style(components::selected(self.tab == index))
                .width(Length::Fill)
                .into()
        }))
        .spacing(2);
        let widths = row([240.0, 360.0, 480.0].into_iter().map(|width| {
            let label = if width == 240.0 {
                "240"
            } else if width == 360.0 {
                "360"
            } else {
                "480"
            };
            action(label, Variant::Ghost, Some(Msg::Width(width)))
                .style(components::selected(self.options.inspector_width == width))
                .height(COMPACT)
                .into()
        }))
        .spacing(4);
        let content: Element<'_, Msg> = if self.tab == 0 {
            column![self.changes(), self.history()].spacing(16).into()
        } else {
            section(
                if self.tab == 1 { "Search" } else { "Files" },
                "Layout specimen",
                text("Return to Changes to inspect the review composition.").size(BODY),
            )
        };
        column![
            row![text("Inspector").size(BODY), space::horizontal(), widths].align_y(Alignment::Center),
            tabs,
            scrollable(content)
                .direction(scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(4).scroller_width(4)))
                .height(Length::Fill),
        ]
        .spacing(INSET)
        .width(self.options.inspector_width)
        .height(Length::Fill)
        .into()
    }

    fn changes(&self) -> Element<'_, Msg> {
        let commit = components::primary_wide(
            if self.busy { "Committing…" } else { "Commit 3 files" },
            (!self.busy && !self.input.trim().is_empty()).then_some(Msg::Commit),
        )
        .width(Length::Fill);
        let files = column![
            file_row("view.rs", "src/", "Modified", theme::p().warn_text),
            file_row("theme.rs", "src/", "Modified", theme::p().warn_text),
            file_row("design.md", "Project root", "Added", theme::p().added_text),
        ]
        .spacing(GAP);
        section(
            "Changes",
            "Review the next commit",
            column![
                row![
                    badge("main"),
                    space::horizontal(),
                    action("Pull", Variant::Ghost, (!self.busy).then_some(Msg::Feedback("Preview pull")))
                        .height(COMPACT),
                    action("Push", Variant::Outline, (!self.busy).then_some(Msg::Feedback("Preview push")))
                        .height(COMPACT)
                ]
                .spacing(4),
                text("Message").size(META).color(theme::p().muted),
                text_input("Describe the change", &self.input)
                    .size(BODY)
                    .line_height(iced::widget::text::LineHeight::Absolute(iced::Pixels(18.0)))
                    .padding([7, 12])
                    .style(components::input)
                    .on_input(Msg::Input),
                commit,
                row![
                    text(if self.staged { "Staged · 3" } else { "Working tree · 3" })
                        .size(META)
                        .color(theme::p().muted),
                    space::horizontal(),
                    action(
                        if self.staged { "Unstage all" } else { "Stage all" },
                        Variant::Ghost,
                        (!self.busy).then_some(Msg::Stage)
                    )
                    .height(COMPACT)
                ]
                .align_y(Alignment::Center),
                files,
            ]
            .spacing(INSET),
        )
    }

    fn history(&self) -> Element<'_, Msg> {
        section(
            "History",
            "Recent commits",
            column![
                text("Keep long branch names readable").size(BODY),
                text("a1b2c3d · 12 minutes ago").size(META).color(theme::p().muted),
                badge("feature/preserve-full-branch-labels-in-narrow-inspectors"),
                separator(),
                text("Polish asynchronous feedback").size(BODY),
                text("d4e5f6a · 38 minutes ago").size(META).color(theme::p().muted),
                badge("main"),
            ]
            .spacing(GAP),
        )
    }

    fn dialog(&self) -> Element<'_, Msg> {
        container(
            column![
                text("Discard changes?").size(20),
                text("This specimen shows a destructive confirmation. The action only updates the preview.").size(BODY),
                badge("src/view.rs"),
                row![
                    space::horizontal(),
                    action("Cancel", Variant::Outline, Some(Msg::Dialog(false))),
                    action("Discard", Variant::Danger, Some(Msg::Dismissed))
                ]
                .spacing(GAP),
                text("Escape closes this preview.").size(META).color(theme::p().muted),
            ]
            .spacing(16),
        )
        .padding(24)
        .max_width(460)
        .style(components::surface)
        .into()
    }
}

fn section<'a>(title: &'a str, subtitle: &'a str, content: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    container(
        column![text(title).size(16), text(subtitle).size(META).color(theme::p().muted), content.into()].spacing(INSET),
    )
    .padding(16)
    .width(Length::Fill)
    .style(components::surface)
    .into()
}

fn separator<'a>() -> Element<'a, Msg> {
    container(space()).width(Length::Fill).height(1).style(theme::bg(theme::p().border)).into()
}

fn file_row<'a>(name: &'a str, path: &'a str, status: &'a str, color: iced::Color) -> Element<'a, Msg> {
    row![
        column![text(name).size(BODY), text(path).size(META).color(theme::p().muted)].spacing(2).width(Length::Fill),
        text(status).size(META).color(color)
    ]
    .spacing(GAP)
    .align_y(Alignment::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::{layout, renderer::Headless, widget::Tree};
    use std::future::Future as _;

    #[test]
    fn inspector_controls_stay_inside_the_available_width() {
        let mut new =
            std::pin::pin!(<iced::Renderer as Headless>::new(theme::ui_font(), iced::Pixels(14.0), Some("tiny-skia")));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(Some(renderer)) = new.as_mut().poll(&mut cx) else {
            panic!("headless renderer");
        };
        fn check(node: &layout::Node, origin: f32, width: f32, depth: usize) {
            let bounds = node.bounds();
            let x = origin + bounds.x;
            assert!(
                x + bounds.width <= width + 0.1,
                "depth {depth}: x={x}, node width={}, available={width}",
                bounds.width
            );
            for child in node.children() {
                check(child, x, width, depth + 1);
            }
        }
        for width in [240.0, 360.0, 480.0] {
            let specimen = Specimen::new(Options { theme: 0, inspector_width: width, capture: None, dialog: false });
            let mut element = specimen.view();
            let mut tree = Tree::new(&element);
            let node = element.as_widget_mut().layout(
                &mut tree,
                &renderer,
                &layout::Limits::new(iced::Size::ZERO, iced::Size::new(1160.0, 1040.0)),
            );
            check(&node, 0.0, 1160.0, 0);
        }
    }
}
