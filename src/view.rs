//! Views. Layout:
//!
//! ```text
//! ┌ document (optional) ──────────┬ inspector ─────────────┐
//! ├ herdr (agents) ───────────────┤ Review·Git·Search·Files │
//! └───────────────────────────────┴─────────────────────────┘
//! ```

use crate::app::*;
use crate::diff::{self, Item, Kind};
use crate::git::Change;
use crate::herdr::AgentStatus;
use crate::icons::{self, Icon};
use crate::search;
use crate::theme::{self, *};

use iced::keyboard::{Key, key::Named};
use iced::widget::pane_grid;
use iced::widget::text::Span;
use iced::widget::text_editor::{self, Binding};
use iced::widget::{
    Column, Stack, button, column, container, lazy, mouse_area, pane_grid as grid, rich_text, row, scrollable, space,
    span, text, text_input, tooltip,
};
use iced::{Alignment, Color, Element, Font, Length, Padding};
use std::borrow::Cow;
use std::sync::Arc;

const UI: u32 = 14;
const SMALL: u32 = 12;
const MAX_RESULT_LINES: usize = 1_500;

fn hrule<'a>() -> Element<'a, Msg> {
    container(space()).width(Length::Fill).height(1).style(theme::bg(p().border)).into()
}

fn vrule<'a>() -> Element<'a, Msg> {
    container(space()).width(1).height(Length::Fill).style(theme::bg(p().border)).into()
}

/// Reserve the same square for icons and activity frames, regardless of font metrics.
fn icon_glyph<'a>(glyph: impl text::IntoFragment<'a>, size: u32) -> text::Text<'a> {
    text(glyph)
        .size(size)
        .line_height(1.0)
        .font(icons::font().unwrap_or_else(theme::mono_font))
        .width(size)
        .height(size)
        .center()
}

/// An icon glyph as text.
fn ico<'a>(i: Icon, size: u32) -> text::Text<'a> {
    icon_glyph(i.glyph(), size)
}

/// One frame of the activity indicator, sharing the icon's footprint.
fn spinner_frame<'a>(frame: usize, size: u32) -> text::Text<'a> {
    icon_glyph(SPINNER[frame], size).color(p().accent)
}

fn icon_button<'a>(i: Icon, msg: Msg) -> Element<'a, Msg> {
    icon_button_maybe(i, Some(msg))
}

/// Icon button, disabled (dimmed) without a message.
fn icon_button_maybe<'a>(i: Icon, msg: Option<Msg>) -> Element<'a, Msg> {
    button(ico(i, 15)).padding([3, 5]).style(theme::flat(false)).on_press_maybe(msg).into()
}

/// Small button with a leading icon.
fn icon_label<'a>(i: Icon, label: impl text::IntoFragment<'a>, msg: Option<Msg>) -> Element<'a, Msg> {
    button(row![ico(i, 13), text(label).size(12)].spacing(5).align_y(Alignment::Center))
        .padding([2, 8])
        .style(theme::flat(false))
        .on_press_maybe(msg)
        .into()
}

/// Draggable split between a side panel and the center.
fn divider<'a>(d: Divider, active: bool) -> Element<'a, Msg> {
    let line = container(space()).width(if active { 2 } else { 1 }).height(Length::Fill).style(theme::bg(if active {
        p().accent
    } else {
        p().border
    }));
    mouse_area(container(line).width(5).height(Length::Fill).center_x(5))
        .interaction(iced::mouse::Interaction::ResizingHorizontally)
        .on_press(Msg::DividerPress(d))
        .on_enter(Msg::DividerHover(Some(d)))
        .on_exit(Msg::DividerHover(None))
        .into()
}

/// Draggable split between two stacked sections (Changes over History).
fn hdivider<'a>(d: Divider, active: bool) -> Element<'a, Msg> {
    let line = container(space()).height(if active { 2 } else { 1 }).width(Length::Fill).style(theme::bg(if active {
        p().accent
    } else {
        p().border
    }));
    mouse_area(container(line).height(5).width(Length::Fill).center_y(5))
        .interaction(iced::mouse::Interaction::ResizingVertically)
        .on_press(Msg::DividerPress(d))
        .on_enter(Msg::DividerHover(Some(d)))
        .on_exit(Msg::DividerHover(None))
        .into()
}

/// A search option toggle (`Aa`, `ab`, `.*`), as in VSCodium.
fn option_toggle<'a>(label: &'static str, on: bool, msg: Msg, name: &'static str) -> Element<'a, Msg> {
    tip(
        button(text(label).size(12).font(theme::mono_font())).padding([2, 5]).style(theme::flat(on)).on_press(msg),
        name,
    )
}

/// The icon of a file (by name) in a list row, if a Nerd Font is installed.
fn file_glyph<'a>(name: &str, size: u32) -> Element<'a, Msg> {
    match icons::font() {
        Some(f) => {
            let (glyph, c) = icons::file_icon(name);
            text(glyph).font(f).size(size).line_height(1.0).color(c).width(size as f32 + 3.0).into()
        }
        None => space().width(0).into(),
    }
}

/// A folder or file icon for a Files row (file icons need a Nerd Font).
fn entry_icon<'a>(is_dir: bool, expanded: bool, name: &str, ignored: bool) -> Element<'a, Msg> {
    match (is_dir, icons::font()) {
        (true, _) => ico(if expanded { Icon::FolderOpen } else { Icon::Folder }, 15).color(p().accent).width(18).into(),
        (false, Some(f)) => {
            let (glyph, c) = icons::file_icon(name);
            let c = if ignored { c.scale_alpha(0.55) } else { c };
            text(glyph).font(f).size(15).line_height(1.0).color(c).width(18).into()
        }
        (false, None) => space().width(0).into(),
    }
}

/// mira's icon (the SVG of the desktop entry).
fn mira_icon<'a>(size: f32) -> Element<'a, Msg> {
    // Hashed once: the title bar shows it on every frame.
    static ICON: std::sync::LazyLock<iced::widget::svg::Handle> = std::sync::LazyLock::new(|| {
        iced::widget::svg::Handle::from_memory(include_bytes!("../assets/mira.svg").as_slice())
    });
    iced::widget::svg(ICON.clone()).width(size).height(size).into()
}

/// A floating panel for a menu.
fn popup<'a>(items: impl Into<Element<'a, Msg>>, width: f32) -> Element<'a, Msg> {
    container(items)
        .width(width)
        .padding(4)
        .style(|_| container::Style {
            background: Some(p().sidebar.into()),
            text_color: Some(p().fg),
            border: iced::Border { color: p().border, width: 1.0, radius: 6.0.into() },
            shadow: iced::Shadow {
                color: Color::BLACK.scale_alpha(0.25),
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..Default::default()
        })
        .into()
}

fn choice_button<'a>(label: String, on: bool, choice: Setting) -> Element<'a, Msg> {
    button(text(label).size(UI)).padding([3, 10]).style(theme::flat(on)).on_press(Msg::SettingChoice(choice)).into()
}

fn setting_group<'a>(title: &'static str) -> Element<'a, Msg> {
    container(text(title.to_uppercase()).size(SMALL).color(p().accent))
        .padding(Padding { top: 6.0, ..Padding::ZERO })
        .into()
}

/// One setting: its name, its control, and a hint under the control.
fn setting<'a>(label: &'static str, control: Element<'a, Msg>, hint: &'static str) -> Element<'a, Msg> {
    let right = if hint.is_empty() {
        column![control]
    } else {
        column![control, text(hint).size(SMALL).color(p().muted).wrapping(text::Wrapping::Word)].spacing(3)
    };
    row![text(label).size(UI).width(170), right.width(Length::Fill)].spacing(12).align_y(Alignment::Start).into()
}

fn view_shortcuts<'a>() -> Element<'a, Msg> {
    let rows = SHORTCUTS.iter().fold(Column::new().spacing(6), |c, (keys, what)| {
        c.push(
            row![
                text(*keys).size(UI).font(theme::mono_font()).color(p().accent).width(240),
                text(*what).size(UI).wrapping(text::Wrapping::Word),
            ]
            .spacing(12),
        )
    });
    column![
        scrollable(container(rows).padding(Padding { right: 12.0, ..Padding::ZERO })).height(Length::Fill),
        text("While the terminal has the keyboard, plain Ctrl+… goes to the program in it; Ctrl+Shift+… and Ctrl+` stay with mira.")
            .size(SMALL)
            .color(p().muted)
            .wrapping(text::Wrapping::Word),
    ]
    .spacing(12)
    .into()
}

fn small_button<'a>(label: impl text::IntoFragment<'a>, msg: Option<Msg>) -> Element<'a, Msg> {
    button(text(label).size(12)).padding([2, 8]).style(theme::flat(false)).on_press_maybe(msg).into()
}

fn tip<'a>(content: impl Into<Element<'a, Msg>>, label: impl text::IntoFragment<'a>) -> Element<'a, Msg> {
    tooltip(
        content,
        container(text(label).size(12)).padding(6).max_width(420).style(theme::bordered(p().bg)),
        tooltip::Position::Bottom,
    )
    .into()
}

/// Tooltip with the full text of a list row that may be clipped; it waits a
/// moment, so moving over the list doesn't flash one per row.
fn row_tip<'a>(content: impl Into<Element<'a, Msg>>, label: impl text::IntoFragment<'a>) -> Element<'a, Msg> {
    tooltip(
        content,
        container(text(label).size(12)).padding(6).max_width(420).style(theme::bordered(p().bg)),
        tooltip::Position::Bottom,
    )
    .delay(std::time::Duration::from_millis(700))
    .into()
}

fn section<'a>(title: String, actions: Vec<Element<'a, Msg>>) -> Element<'a, Msg> {
    let mut r = row![text(title).size(SMALL).color(p().muted), space::horizontal()].align_y(Alignment::Center);
    for a in actions {
        r = r.push(a);
    }
    container(r).padding([4, 10]).width(Length::Fill).style(theme::panel(p().section)).into()
}

/// A Changes row's buttons, built only when they show (hovered or open row):
/// the list can hold thousands of files and `view` runs very often.
fn row_actions<'a>(show: bool, build: impl FnOnce() -> Vec<Element<'a, Msg>>) -> Vec<Element<'a, Msg>> {
    if show { build() } else { Vec::new() }
}

fn badge<'a>(label: impl text::IntoFragment<'a>, color: Color) -> Element<'a, Msg> {
    container(
        text(label)
            .size(SMALL)
            .font(Font { weight: iced::font::Weight::Semibold, ..theme::ui_font() })
            .color(p().on_accent)
            .wrapping(text::Wrapping::WordOrGlyph),
    )
    .padding([2, 6])
    .style(move |_| container::Style {
        background: Some(color.into()),
        border: iced::Border::default().rounded(4),
        ..Default::default()
    })
    .into()
}

fn history_file_label(file: &Change) -> Element<'_, Msg> {
    let status = match file.code {
        'A' => "A",
        'D' => "D",
        'R' => "R",
        'C' => "C",
        'T' => "T",
        _ => "M",
    };
    let content = row![
        file_glyph(&file.path, 14),
        container(text(file.path.as_str()).size(UI).wrapping(text::Wrapping::None)).width(Length::Fill).clip(true),
        text(status)
            .size(SMALL)
            .color(theme::status_color(file.code))
            .width(18)
            .align_x(iced::alignment::Horizontal::Center),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let mut file_row = column![content];
    if let Some(orig) = &file.orig {
        file_row = file_row.push(text(orig.as_str()).size(SMALL).color(p().muted));
    }
    file_row.into()
}

fn split_path(p: &str) -> (&str, &str) {
    match p.rsplit_once('/') {
        Some((d, n)) => (n, d),
        None => (p, ""),
    }
}

fn agent_style(status: AgentStatus) -> (Color, &'static str) {
    match status {
        AgentStatus::Working => (p().accent, "working"),
        AgentStatus::Blocked => (p().modified, "needs you"),
        AgentStatus::Done => (p().added, "done"),
        AgentStatus::Idle => (p().muted, "idle"),
        AgentStatus::Unknown => (p().muted, "unknown"),
    }
}

/// `now - then` (unix seconds) as `12s`, `5m`, `3h`, `2d`.
fn ago(then: u64) -> String {
    let s = crate::persist::now().saturating_sub(then);
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

impl App {
    pub fn view(&self) -> Element<'_, Msg> {
        let center = grid(&self.center, |_, pane, _| {
            pane_grid::Content::new(match pane {
                CenterPane::Document => self.view_document(),
                CenterPane::Terminal => self.view_terminal(),
            })
        })
        .on_resize(6, Msg::CenterResized)
        .spacing(1)
        .style(|_| pane_grid::Style {
            hovered_region: pane_grid::Highlight {
                background: p().selection.scale_alpha(0.3).into(),
                border: iced::Border::default(),
            },
            picked_split: pane_grid::Line { color: p().accent, width: 2.0 },
            hovered_split: pane_grid::Line { color: p().accent, width: 2.0 },
        });

        let mut body = row![container(center).style(theme::bg(p().border)).width(Length::Fill).height(Length::Fill)];
        if self.show_inspector && !self.focus_mode {
            let active = self.dragging == Some(Divider::Inspector) || self.hovered_divider == Some(Divider::Inspector);
            body = body.push(divider(Divider::Inspector, active)).push(self.view_inspector());
        }
        let base: Element<'_, Msg> =
            column![self.view_title_bar(), body.height(Length::Fill), self.view_status_bar()].into();
        // Always the same two layers: changing the root's shape when a dialog opens would
        // make iced drop every widget's state (the terminal's focus, scroll positions).
        let close_on_click = |msg: Msg| {
            iced::widget::opaque(
                mouse_area(space().width(Length::Fill).height(Length::Fill)).on_press(msg.clone()).on_right_press(msg),
            )
        };
        let overlay: Element<'_, Msg> = if let Some(pending) = &self.confirmation {
            let panel = iced::widget::opaque(self.view_confirmation(pending));
            iced::widget::opaque(
                mouse_area(iced::widget::center(panel).padding(24).style(|_| container::Style {
                    background: Some(p().bg.scale_alpha(0.75).into()),
                    ..Default::default()
                }))
                .on_press(Msg::ConfirmCancel),
            )
        } else if let Some(dialog) = &self.remote_dialog {
            iced::widget::opaque(
                container(self.view_remote_dialog(dialog))
                    .padding(24)
                    .center(Length::Fill)
                    .style(theme::panel(p().bg.scale_alpha(0.55))),
            )
        } else if let Some(picker) = &self.branch_picker {
            iced::widget::opaque(
                container(self.view_branches(picker))
                    .padding(24)
                    .center(Length::Fill)
                    .style(theme::panel(p().bg.scale_alpha(0.55))),
            )
        } else if self.gear_menu {
            iced::widget::stack![close_on_click(Msg::GearMenu(false)), self.view_gear_menu()].into()
        } else if let Some(d) = self.dialog {
            // Modal: a click outside closes it, as Escape does.
            let panel = iced::widget::opaque(self.view_dialog(d));
            iced::widget::opaque(
                mouse_area(iced::widget::center(panel).padding(24).style(|_| container::Style {
                    background: Some(p().bg.scale_alpha(0.6).into()),
                    ..Default::default()
                }))
                .on_press(Msg::CloseDialog),
            )
        } else if let Some(menu) = &self.explorer_menu {
            // Any click outside closes it, like a desktop menu.
            iced::widget::stack![
                iced::widget::opaque(
                    mouse_area(space().width(Length::Fill).height(Length::Fill))
                        .on_press(Msg::ExplorerMenuClose)
                        .on_right_press(Msg::ExplorerMenuClose)
                ),
                self.view_explorer_menu(menu),
            ]
            .into()
        } else if let Some(palette) = &self.palette {
            // Near the top like VSCodium's, so the input stays put while the list
            // grows. A click outside closes it; `opaque` keeps it from what is behind
            // (and, inside, keeps clicks on the palette from closing it).
            let panel = iced::widget::opaque(self.view_palette(palette));
            iced::widget::opaque(
                mouse_area(
                    container(panel)
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .align_x(Alignment::Center)
                        .padding(Padding { top: 56.0, right: 16.0, bottom: 16.0, left: 16.0 })
                        .style(|_| container::Style {
                            background: Some(p().bg.scale_alpha(0.35).into()),
                            ..Default::default()
                        }),
                )
                .on_press(Msg::PaletteClose),
            )
        } else if let Some(signin) = &self.signin {
            // Modal: `opaque` keeps clicks and the wheel from reaching what is behind.
            iced::widget::opaque(iced::widget::center(self.view_signin(&signin.stage)).style(|_| container::Style {
                background: Some(p().bg.scale_alpha(0.75).into()),
                ..Default::default()
            }))
        } else {
            space().into()
        };
        iced::widget::stack![base, overlay].into()
    }

    /// mira's own title bar (`window_controls = "on"`): drag it to move the
    /// window, double-click to maximize; minimize, maximize and close at the right.
    fn view_title_bar(&self) -> Element<'_, Msg> {
        if self.window_controls != WindowControls::On {
            return space().height(0).into();
        }
        let win = |icon: Icon, msg: Msg, label: &'static str, close: bool| {
            tip(
                button(container(ico(icon, 14)).center(Length::Fill))
                    .width(42)
                    .height(Length::Fill)
                    .padding(0)
                    .style(move |t, s| {
                        if close && matches!(s, button::Status::Hovered | button::Status::Pressed) {
                            theme::danger(t, s)
                        } else {
                            theme::flat(false)(t, s)
                        }
                    })
                    .on_press(msg),
                label,
            )
        };
        let title = mouse_area(
            container(
                row![mira_icon(16.0), text(self.title()).size(SMALL).color(p().muted)]
                    .spacing(8)
                    .align_y(Alignment::Center),
            )
            .padding([0, 12])
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(Alignment::Center),
        )
        .on_press(Msg::WindowDrag)
        .on_double_click(Msg::WindowMaximize);
        container(
            row![
                title,
                win(Icon::WinMinimize, Msg::WindowMinimize, "Minimize", false),
                win(Icon::WinMaximize, Msg::WindowMaximize, "Maximize or restore", false),
                win(Icon::WinClose, Msg::CloseRequested, "Close", true),
            ]
            .height(30),
        )
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(p().sidebar.into()),
            border: iced::Border { color: p().border, width: 1.0, radius: 0.0.into() },
            ..Default::default()
        })
        .into()
    }

    /// The gear's menu, above it at the bottom right.
    fn view_gear_menu(&self) -> Element<'_, Msg> {
        let item = |label: &'static str, hint: &'static str, msg: Msg| -> Element<'_, Msg> {
            button(
                row![text(label).size(UI), space::horizontal(), text(hint).size(SMALL).color(p().muted)]
                    .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([4, 12])
            .style(theme::flat(false))
            .on_press(msg)
            .into()
        };
        let items = column![
            item("Settings", "Ctrl+,", Msg::Gear(MenuItem::Settings)),
            item("Keyboard Shortcuts", "", Msg::Gear(MenuItem::Shortcuts)),
            container(hrule()).padding([3, 6]),
            item("Check for Updates", "browser", Msg::Gear(MenuItem::Updates)),
            item("About mira", "", Msg::Gear(MenuItem::About)),
        ]
        .spacing(1);
        const WIDTH: f32 = 260.0;
        const HEIGHT: f32 = 4.0 * 27.0 + 7.0 + 10.0;
        // Over the status bar's right end, where the gear is.
        let x = (self.window_width - WIDTH - 8.0).max(0.0);
        let y = (self.window_height - HEIGHT - 30.0).max(0.0);
        iced::widget::pin(iced::widget::opaque(popup(items, WIDTH))).x(x).y(y).into()
    }

    fn view_branches<'a>(&self, picker: &'a BranchPicker) -> Element<'a, Msg> {
        let mut list = Column::new().spacing(3);
        for (position, &index) in picker.matches.iter().enumerate() {
            let branch = &picker.listing.branches[index];
            let label = column![
                text(branch.name.as_str()).size(UI).wrapping(text::Wrapping::None),
                text(branch.unavailable.as_deref().unwrap_or(if branch.remote {
                    "Remote — creates a local tracking branch"
                } else {
                    "Local branch"
                }))
                .size(12)
                .wrapping(text::Wrapping::None)
                .color(p().muted)
            ]
            .spacing(2);
            list = list.push(row_tip(
                button(container(label).width(Length::Fill).clip(true))
                    .height(52)
                    .width(Length::Fill)
                    .padding([7, 10])
                    .style(theme::outline(position == picker.selected))
                    .on_press_maybe(
                        branch.unavailable.is_none().then(|| Msg::BranchPick(picker.id, branch.reference.clone())),
                    ),
                branch.unavailable.as_deref().unwrap_or(branch.name.as_str()),
            ));
        }
        if picker.matches.is_empty() {
            list = list.push(text("No matching branches").size(UI).color(p().muted));
        }
        let mut body=column![text("Switch branch").size(20),
            text("Local edits are kept when Git can switch safely. If they conflict, commit or stash them in a shell first. Remote refs are from the last fetch.").size(UI),
            text_input("Find a local or remote branch…",&picker.query).id("branch-search")
                .on_input(move |q|Msg::BranchQuery(picker.id,q)).on_submit(Msg::BranchSubmit(picker.id)).padding(8),
            scrollable(list).id("branch-list").spacing(4).height((picker.matches.len().max(1) as f32*55.0-3.0).min((self.window_height-300.0).clamp(100.0,400.0))),
        ].spacing(12);
        if picker.capped || picker.listing.truncated {
            body = body.push(
                text("Branch list limited — refine the search or use the terminal for additional refs.")
                    .size(12)
                    .color(p().muted),
            );
        }
        body = body.push(
            row![
                text("↑/↓ select · Enter switches · Escape closes").size(12).color(p().muted),
                space::horizontal(),
                button(text("Cancel")).padding([6, 12]).style(theme::outline(false)).on_press(Msg::BranchesClose)
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );
        container(scrollable(body).spacing(4).height(Length::Shrink))
            .padding(24)
            .width(Length::Fill)
            .max_width(640)
            .max_height((self.window_height - 48.0).max(120.0))
            .style(theme::bordered(p().sidebar))
            .into()
    }

    fn view_remote_dialog<'a>(&self, dialog: &'a RemoteDialog) -> Element<'a, Msg> {
        let mut body = column![
            text(dialog.title.as_str()).size(20),
            text(dialog.summary.as_str()).size(UI),
            text(dialog.context.as_str()).size(12).font(theme::mono_font()).color(p().muted),
            text(dialog.next.as_str()).size(UI),
        ]
        .spacing(12);
        if !dialog.remotes.is_empty() {
            body = body.push(
                iced::widget::pick_list(dialog.remotes.as_slice(), dialog.selected_remote.as_ref(), move |remote| {
                    Msg::RemoteSelect(dialog.id, remote)
                })
                .placeholder("Choose a remote…")
                .width(Length::Fill),
            );
        }
        if !dialog.detail.is_empty() {
            body = body.push(
                container(
                    scrollable(text(dialog.detail.as_str()).size(12).font(theme::mono_font())).spacing(4).height(160),
                )
                .padding(10)
                .width(Length::Fill)
                .style(theme::bordered(p().bg)),
            );
        }
        for (i, (label, choice)) in dialog.choices.iter().enumerate() {
            body = body.push(
                button(text(*label).size(UI))
                    .width(Length::Fill)
                    .padding([6, 12])
                    .style(theme::outline(dialog.selected == i))
                    .on_press_maybe(
                        (*choice != RemoteChoice::Publish || dialog.selected_remote.is_some())
                            .then_some(Msg::RemoteAction(dialog.id, *choice)),
                    ),
            );
        }
        body = body.push(text("Escape closes · Tab selects an action · Enter activates it").size(12).color(p().muted));
        container(scrollable(body).spacing(4).height(Length::Shrink))
            .padding(24)
            .width(Length::Fill)
            .max_width(680)
            .max_height((self.window_height - 48.0).max(120.0))
            .style(theme::bordered(p().sidebar))
            .into()
    }

    fn view_confirmation<'a>(&self, pending: &'a Confirmation) -> Element<'a, Msg> {
        let mut content = column![
            row![ico(Icon::Discard, 20).color(p().warn_text), text(pending.title).size(20)]
                .spacing(10)
                .align_y(Alignment::Center),
            text(pending.detail.as_str()).size(UI).color(p().fg),
        ]
        .spacing(16);
        if !pending.targets.is_empty() {
            let list = pending.targets.iter().fold(Column::new().spacing(5), |list, target| {
                list.push(text(target.as_str()).size(12).font(theme::mono_font()).color(p().fg))
            });
            content = content.push(
                container(scrollable(list).height((pending.targets.len() as f32 * 23.0).min(184.0)))
                    .padding(10)
                    .width(Length::Fill)
                    .style(theme::bordered(p().bg)),
            );
        }
        if let Some(remaining) = &pending.remaining {
            content = content.push(text(remaining.as_str()).size(12).color(p().muted));
        }
        let selected = pending.accept_selected;
        content = content.push(
            row![
                space::horizontal(),
                button(text("Cancel").size(UI))
                    .padding([7, 14])
                    .style(theme::outline(!selected))
                    .on_press(Msg::ConfirmCancel),
                button(text(pending.accept).size(UI))
                    .padding([7, 14])
                    .style(move |theme, status| {
                        let mut style = theme::danger(theme, status);
                        if selected {
                            style.border.color = p().accent;
                            style.border.width = 2.0;
                        }
                        style
                    })
                    .on_press(Msg::ConfirmProceed(pending.id)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );
        content =
            content.push(text("Escape cancels · Tab selects an action · Enter activates it").size(12).color(p().muted));
        container(content).padding(24).width(Length::Fill).max_width(580).style(theme::bordered(p().sidebar)).into()
    }

    fn view_dialog(&self, d: Dialog) -> Element<'_, Msg> {
        let (title, body): (&str, Element<'_, Msg>) = match d {
            Dialog::Settings => ("Settings", self.view_settings()),
            Dialog::Shortcuts => ("Keyboard Shortcuts", view_shortcuts()),
            Dialog::About => ("About mira", self.view_about()),
        };
        let header = row![
            text(title).size(18),
            space::horizontal(),
            tip(icon_button(Icon::Close, Msg::CloseDialog), "Close (Escape)"),
        ]
        .align_y(Alignment::Center);
        // Long dialogs scroll inside, with their header and footer in place.
        let tall = d != Dialog::About;
        container(column![header, body].spacing(14))
            .padding(20)
            .width(Length::Fill)
            .height(if tall { Length::Fill } else { Length::Shrink })
            .max_width(if tall { 640 } else { 460 })
            .max_height((self.window_height - 60.0).max(240.0))
            .style(|_| container::Style {
                background: Some(p().sidebar.into()),
                text_color: Some(p().fg),
                border: iced::Border { color: p().border, width: 1.0, radius: 10.0.into() },
                shadow: iced::Shadow {
                    color: Color::BLACK.scale_alpha(0.3),
                    offset: iced::Vector::new(0.0, 8.0),
                    blur_radius: 28.0,
                },
                ..Default::default()
            })
            .into()
    }

    fn view_settings(&self) -> Element<'_, Msg> {
        let d = &self.drafts;
        let field = |f: Field, placeholder: &'static str| -> Element<'_, Msg> {
            text_input(placeholder, d.get(f))
                .on_input(move |v| Msg::SettingText(f, v))
                .on_submit(Msg::SettingSubmit)
                .size(UI)
                .padding([4, 8])
                .style(theme::input)
                .into()
        };
        let theme_names: Vec<String> =
            std::iter::once("auto".to_string()).chain(theme::PALETTES.iter().map(|p| p.name.to_string())).collect();
        let current_theme = self.theme.label();
        let themes = theme_names.into_iter().fold(row![].spacing(4), |r, name| {
            let on = name.eq_ignore_ascii_case(&current_theme);
            r.push(choice_button(name.clone(), on, Setting::Theme(name)))
        });
        let wc = |w: WindowControls| {
            choice_button(w.label().to_string(), self.window_controls == w, Setting::WindowControls(w))
        };
        let opts = |list: &[(&'static str, &'static str)], now: &str, make: fn(&'static str) -> Setting| {
            list.iter().fold(row![].spacing(4), |r, (value, label)| {
                r.push(choice_button((*label).to_string(), now.eq_ignore_ascii_case(value), make(value)))
            })
        };
        let autosave = opts(
            &[
                ("afterDelay", "After a delay"),
                ("onFocusChange", "On focus change"),
                ("onWindowChange", "On window change"),
                ("off", "Off"),
            ],
            &d.autosave,
            Setting::Autosave,
        );
        let terminal = opts(&[("herdr", "herdr"), ("shell", "Plain shell")], &d.terminal, Setting::Terminal);
        let provider = opts(
            &[("auto", "auto"), ("chatgpt", "ChatGPT"), ("openai", "OpenAI API key"), ("command", "Command")],
            &d.provider,
            Setting::CommitProvider,
        );
        let changes = row![
            button(text("By file").size(UI))
                .padding([3, 10])
                .style(theme::flat(!self.group_by_run))
                .on_press(Msg::GroupByRun(false)),
            button(text("By agent turn").size(UI))
                .padding([3, 10])
                .style(theme::flat(self.group_by_run))
                .on_press(Msg::GroupByRun(true)),
        ]
        .spacing(4);
        let account: Element<'_, Msg> = match &self.chatgpt_account {
            Some(email) => row![
                text(if email.is_empty() { "Signed in".to_string() } else { format!("Signed in as {email}") }).size(UI),
                space::horizontal(),
                small_button("Sign out", Some(Msg::ChatGptSignOut)),
            ]
            .align_y(Alignment::Center)
            .into(),
            None => text("Not signed in (✦ in Changes asks when needed)").size(UI).color(p().muted).into(),
        };
        let body = column![
            setting_group("Appearance"),
            setting("Theme", themes.wrap().into(), "auto follows HyDE when it is installed"),
            setting(
                "Window controls",
                row![wc(WindowControls::Auto), wc(WindowControls::On), wc(WindowControls::Off)].spacing(4).into(),
                "auto: the desktop draws them (tiling ones like Hyprland draw none) · on: mira draws its own title bar",
            ),
            setting_group("Editor"),
            setting("Autosave", autosave.wrap().into(), ""),
            setting("Autosave delay (ms)", field(Field::AutosaveDelay, "1000"), ""),
            setting_group("Terminal"),
            setting("Runs", terminal.into(), "Takes effect the next time mira starts"),
            setting("herdr session", field(Field::HerdrSession, "default"), "Takes effect the next time mira starts"),
            setting_group("Commit messages"),
            setting("Provider", provider.wrap().into(), "Only used when you click ✦; nothing else goes to the network"),
            setting(
                "Model",
                field(Field::CommitModel, crate::commitgen::DEFAULT_MODEL),
                "For ChatGPT and the OpenAI API"
            ),
            setting(
                "Command",
                field(Field::CommitCommand, "e.g. ollama run qwen2.5-coder:7b"),
                "Gets the prompt on stdin, prints the message"
            ),
            setting("ChatGPT", account, ""),
            setting_group("Changes"),
            setting("Show changes", changes.into(), ""),
        ]
        .spacing(10);
        let footer = row![
            self.settings_error.as_deref().map(|e| text(e).size(SMALL).color(p().error_text)),
            space::horizontal(),
            small_button("Open config file", Some(Msg::OpenConfigFile)),
            button(text("Done").size(12)).padding([3, 12]).style(theme::primary).on_press(Msg::CloseDialog),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        column![
            scrollable(container(body).padding(Padding { right: 12.0, ..Padding::ZERO })).height(Length::Fill),
            footer
        ]
        .spacing(14)
        .height(Length::Fill)
        .into()
    }

    fn view_about(&self) -> Element<'_, Msg> {
        let link = |label: &'static str, url: String| small_button(label, Some(Msg::OpenLink(url)));
        let third_party = self.licenses_url.clone().unwrap_or_else(|| format!("{REPO_URL}/releases"));
        column![
            row![
                mira_icon(64.0),
                column![
                    text(format!("mira {}", env!("CARGO_PKG_VERSION"))).size(20),
                    text(env!("CARGO_PKG_DESCRIPTION")).size(UI).color(p().muted).wrapping(text::Wrapping::Word),
                ]
                .spacing(4),
            ]
            .spacing(16)
            .align_y(Alignment::Center),
            row![
                text(self.about_text()).size(SMALL).font(theme::mono_font()).color(p().muted),
                space::horizontal(),
                tip(small_button("Copy info", Some(Msg::CopyAbout)), "For a bug report"),
            ]
            .align_y(Alignment::End),
            text("MIT OR Apache-2.0 · no telemetry · in early development").size(SMALL).color(p().muted),
            row![
                link("Repository", REPO_URL.to_string()),
                link("User guide", format!("{REPO_URL}/blob/main/docs/guide.md")),
                link("Third-party licenses", third_party),
                link("Report a bug", format!("{REPO_URL}/issues/new")),
            ]
            .spacing(6)
            .wrap(),
        ]
        .spacing(12)
        .into()
    }

    /// The ChatGPT sign-in dialog: what will happen before it happens, then
    /// the wait for the browser (with the link, in case it didn't open).
    fn view_signin<'a>(&'a self, stage: &'a SignInStage) -> Element<'a, Msg> {
        let para = |s: &'a str| text(s).size(UI).color(p().fg).wrapping(text::Wrapping::Word);
        let note = |s: &'a str| text(s).size(SMALL).color(p().muted).wrapping(text::Wrapping::Word);
        let step = |n: &'a str, s: &'a str| {
            row![text(n).size(UI).color(p().accent).width(18), para(s)].spacing(4).align_y(Alignment::Start)
        };
        let cancel = button(text("Cancel").size(UI)).padding([6, 14]).style(theme::flat_fn).on_press(Msg::SignInCancel);
        let (title, body, actions): (Element<'a, Msg>, Column<'a, Msg>, iced::widget::Row<'a, Msg>) = match stage {
            SignInStage::Ask => (
                row![ico(Icon::Sparkle, 18).color(p().accent), text("Sign in with ChatGPT").size(18)]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .into(),
                column![
                    para("mira writes commit messages with your ChatGPT subscription. To connect it:"),
                    step("1", "Your browser opens auth.openai.com: the same sign-in as Codex CLI."),
                    step("2", "You approve there, and the browser hands the session back to mira on this computer (localhost:1455)."),
                    step("3", "mira keeps it in ~/.local/state/mira/chatgpt.json, readable only by you. Sign out any time from the account icon."),
                    para("When you generate a message, only the next commit's diff and its file list are sent."),
                    note("Rather use an API key or a local model? Set commit_provider in ~/.config/mira/config."),
                ]
                .spacing(10),
                row![
                    space::horizontal(),
                    cancel,
                    button(text("Open browser").size(UI)).padding([6, 14]).style(theme::primary).on_press(Msg::SignInStart),
                ]
                .spacing(8),
            ),
            SignInStage::Waiting { url, opened } => {
                let mut body = column![para(
                    "Finish signing in at auth.openai.com, then come back: this closes by itself and the commit message starts."
                )]
                .spacing(10);
                if url.is_some() && !opened {
                    body = body.push(text("mira couldn't open a browser.").size(UI).color(p().warn_text));
                }
                body = body.push(note("Browser didn't open, or opened the wrong one? Copy the link and paste it there."));
                let copy = button(row![ico(Icon::GoToFile, 13), text("Copy link").size(UI)].spacing(6).align_y(Alignment::Center))
                    .padding([6, 14])
                    .style(theme::flat_fn)
                    .on_press_maybe(url.is_some().then_some(Msg::SignInCopyLink));
                (
                    row![
                        self.spinner(18),
                        text("Waiting for your browser…").size(18)
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .into(),
                    body,
                    row![copy, space::horizontal(), cancel].spacing(8),
                )
            }
        };
        container(column![title, body, actions].spacing(18))
            .padding(24)
            .max_width(480)
            .style(|_| container::Style {
                background: Some(p().sidebar.into()),
                text_color: Some(p().fg),
                border: iced::Border { color: p().border, width: 1.0, radius: 8.0.into() },
                shadow: iced::Shadow {
                    color: Color::BLACK.scale_alpha(0.25),
                    offset: iced::Vector::new(0.0, 6.0),
                    blur_radius: 24.0,
                },
                ..Default::default()
            })
            .into()
    }

    /// The spinner's current frame, in the accent color.
    fn spinner<'a>(&self, size: u32) -> text::Text<'a> {
        spinner_frame(self.spin, size)
    }

    fn action_icon<'a>(&self, icon: Icon, msg: &Msg, size: u32) -> text::Text<'a> {
        if self.busy_op.as_ref().is_some_and(|op| op.matches(msg)) { self.spinner(size) } else { ico(icon, size) }
    }

    /// The working button keeps its place and cannot be clicked twice.
    fn git_button<'a>(&self, icon: Icon, msg: Msg) -> Element<'a, Msg> {
        button(self.action_icon(icon, &msg, 15))
            .padding([3, 5])
            .style(theme::flat(false))
            .on_press_maybe((!self.git_busy).then_some(msg))
            .into()
    }

    fn git_label<'a>(&self, icon: Icon, label: &'a str, msg: Msg) -> Element<'a, Msg> {
        button(row![self.action_icon(icon, &msg, 13), text(label).size(12)].spacing(5).align_y(Alignment::Center))
            .padding([2, 8])
            .style(theme::flat(false))
            .on_press_maybe((!self.git_busy).then_some(msg))
            .into()
    }

    /// Quick open: the query, then the best matching files (or go to line after `:`).
    fn view_palette<'a>(&'a self, pal: &'a QuickOpen) -> Element<'a, Msg> {
        let input = text_input("Go to file by name · : to go to a line", &pal.query)
            .id(PALETTE_INPUT)
            .on_input(Msg::PaletteQuery)
            .on_submit(Msg::PaletteSubmit)
            .size(UI)
            .padding(8)
            .style(theme::input);
        let note = |s: Cow<'a, str>, color: Color| container(text(s).size(SMALL).color(color)).padding([6, 10]);
        let mut list = Column::new().spacing(1);
        if let Some(line) = pal.line_query() {
            let hint: Cow<'a, str> = match self.active_file() {
                None => "Open a file to go to a line".into(),
                Some(t) => {
                    let count = t.content.line_count();
                    match line.parse::<usize>() {
                        Ok(n) => {
                            format!("Go to line {} of {} (Enter)", n.clamp(1, count.max(1)), file_name(&t.path)).into()
                        }
                        Err(_) => format!("Type a line number, 1 to {count}").into(),
                    }
                }
            };
            list = list.push(note(hint, p().muted));
        } else {
            match &pal.files {
                None => list = list.push(note("Listing files…".into(), p().muted)),
                Some(_) if pal.results.is_empty() => list = list.push(note("No matching files".into(), p().muted)),
                Some(files) => {
                    for (i, &fi) in pal.results.iter().enumerate() {
                        let Some(rel) = files.get(fi) else { continue };
                        let (name, dir) = split_path(rel);
                        let line = row![
                            file_glyph(name, 15),
                            container(
                                row![
                                    text(name).size(UI).color(p().fg).wrapping(text::Wrapping::None),
                                    text(dir).size(SMALL).color(p().muted).wrapping(text::Wrapping::None),
                                ]
                                .spacing(8)
                                .align_y(Alignment::Center),
                            )
                            .width(Length::Fill)
                            .clip(true),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center);
                        list = list.push(
                            button(line)
                                .width(Length::Fill)
                                .padding([4, 10])
                                .style(theme::flat(i == pal.selected))
                                .on_press(Msg::PalettePick(i)),
                        );
                    }
                }
            }
            if pal.capped {
                list = list.push(note(
                    format!("Only the first {} files are listed", crate::quickopen::MAX_FILES).into(),
                    p().warn_text,
                ));
            }
        }
        container(column![input, list].spacing(6))
            .padding(8)
            .width(Length::Fill)
            .max_width(600)
            .style(|_| container::Style {
                background: Some(p().sidebar.into()),
                text_color: Some(p().fg),
                border: iced::Border { color: p().border, width: 1.0, radius: 8.0.into() },
                shadow: iced::Shadow {
                    color: Color::BLACK.scale_alpha(0.25),
                    offset: iced::Vector::new(0.0, 6.0),
                    blur_radius: 24.0,
                },
                ..Default::default()
            })
            .into()
    }

    fn view_terminal(&self) -> Element<'_, Msg> {
        let herdr = self.term_kind == TermKind::Herdr;
        let mut header =
            row![ico(Icon::Terminal, 13).color(p().accent), text(if herdr { "herdr" } else { "shell" }).size(UI),]
                .spacing(8)
                .align_y(Alignment::Center);
        // Something in the header already takes the free width (clipped or scrolled).
        let mut filled = false;
        if herdr {
            if let Some(e) = &self.herdr_error {
                header = header.push(tip(
                    container(text(e.as_str()).size(SMALL).color(p().error_text).wrapping(text::Wrapping::None))
                        .width(Length::Fill)
                        .clip(true),
                    e.as_str(),
                ));
                filled = true;
            } else if self.agents.is_empty() {
                header = header.push(text("Ctrl+B ? for herdr keys").size(SMALL).color(p().muted));
            }
            // Agents working in this folder; click to jump to their pane. Many
            // agents scroll sideways instead of pushing the buttons out.
            if !self.agents.is_empty() {
                let mut chips = row![].spacing(2).align_y(Alignment::Center);
                for a in &self.agents {
                    let (color, label) = agent_style(a.agent_status);
                    let chip = row![
                        ico(Icon::DotFilled, 9).color(color),
                        text(a.label()).size(SMALL).wrapping(text::Wrapping::None)
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center);
                    chips = chips.push(tip(
                        button(chip)
                            .padding([1, 6])
                            .style(theme::flat(false))
                            .on_press(Msg::FocusAgent(a.pane_id.clone())),
                        format!("{label} · {} — click to show its pane", a.pane_id),
                    ));
                }
                header = header.push(
                    scrollable(chips)
                        .direction(scrollable::Direction::Horizontal(
                            scrollable::Scrollbar::new().width(3).scroller_width(3),
                        ))
                        .width(Length::Fill),
                );
                filled = true;
            }
        } else if self.herdr_missing {
            header = header.push(tip(
                text("herdr not found: agent tracking off").size(SMALL).color(p().muted),
                "Install herdr (herdr.dev) for agents, runs and review, \
                 or set terminal = \"shell\" in ~/.config/mira/config to hide this",
            ));
        }
        if !filled {
            header = header.push(space::horizontal());
        }
        header = header.push(tip(
            icon_button(if self.focus_mode { Icon::ScreenNormal } else { Icon::ScreenFull }, Msg::FocusMode),
            if self.focus_mode { "Exit focus (Ctrl+Shift+M)" } else { "Focus (Ctrl+Shift+M)" },
        ));
        if !self.focus_mode {
            header = header.push(tip(
                icon_button(
                    if self.show_inspector { Icon::PanelRight } else { Icon::PanelRightOff },
                    Msg::ToggleInspector,
                ),
                if self.show_inspector { "Hide panel (Ctrl+Shift+B)" } else { "Show panel (Ctrl+Shift+B)" },
            ));
        }
        let body: Element<'_, Msg> = match &self.term {
            Some(t) => container(crate::term::view(t, theme::mono_font(), CODE_SIZE as f32, Msg::TermAction))
                .padding(Padding { left: 8.0, ..Padding::ZERO })
                .style(theme::panel(p().bg))
                .into(),
            None => container(
                column![
                    text(if herdr { "herdr is detached. Your agents keep running." } else { "The shell exited." })
                        .size(UI)
                        .color(p().muted),
                    button(
                        row![ico(Icon::Refresh, 14), text(if herdr { "Reattach herdr" } else { "New shell" }).size(UI)]
                            .spacing(6)
                            .align_y(Alignment::Center),
                    )
                    .style(theme::primary)
                    .padding([5, 12])
                    .on_press(Msg::RestartTerminal),
                ]
                .spacing(10)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .style(theme::panel(p().bg))
            .into(),
        };
        column![container(header).padding([4, 12]).width(Length::Fill).style(theme::panel(p().sidebar)), body].into()
    }

    // ------------------------------------------------------------ inspector

    fn view_inspector(&self) -> Element<'_, Msg> {
        let changes = Some(self.git.staged.len() + self.git.unstaged.len()).filter(|n| *n > 0);
        let fresh = self.fresh_changes() > 0;
        let tab = |i: Icon, label: &'static str, which: Inspector, count: Option<usize>, alert: bool| {
            let active = self.inspector == which;
            let color = if active { p().accent } else { p().muted };
            let mut r = row![ico(i, 14).color(color), text(label).size(UI)].spacing(6).align_y(Alignment::Center);
            if let Some(n) = count {
                r = r.push(text(n.to_string()).size(10).color(if alert { p().warn_text } else { p().accent }));
            }
            if alert {
                r = r.push(ico(Icon::DotFilled, 8).color(p().modified));
            }
            button(r).padding([6, 9]).style(theme::tab(active)).on_press(Msg::InspectorTab(which))
        };
        let tabs = row![
            tip(tab(Icon::SourceControl, "Changes", Inspector::Changes, changes, fresh), "Changes (Ctrl+Shift+G)"),
            tip(tab(Icon::Search, "Search", Inspector::Search, None, false), "Search (Ctrl+Shift+F)"),
            tip(tab(Icon::Files, "Files", Inspector::Files, None, false), "Files (Ctrl+Shift+E)"),
        ];
        let body = match self.inspector {
            Inspector::Changes => self.view_git(),
            Inspector::Search => self.view_search(),
            Inspector::Files => self.view_explorer(),
        };
        container(column![container(tabs).width(Length::Fill).style(theme::panel(p().sidebar)), hrule(), body])
            .width(self.inspector_shown_width())
            .height(Length::Fill)
            .style(theme::panel(p().sidebar))
            .into()
    }

    /// One changed file. `origin`: show which agent run last changed it.
    /// One changed file. `hover` (section, index) makes it a Changes row that
    /// reports the mouse; `actions` (given while it is hovered or open, as in
    /// VSCodium) take the place of the agent label.
    fn change_button<'a>(
        &'a self,
        c: &'a Change,
        active: bool,
        msg: Msg,
        hover: Option<(u8, usize)>,
        actions: Vec<Element<'a, Msg>>,
        origin: bool,
    ) -> Element<'a, Msg> {
        let (name, dir) = split_path(&c.path);
        let color = theme::status_color(c.code);
        let new = self.runs.is_new(&c.path);
        // The whole path (clipped in the row), and where a rename came from.
        let full: Cow<'_, str> = match &c.orig {
            Some(orig) => format!("{orig} → {}", c.path).into(),
            None => c.path.as_str().into(),
        };
        let mut content = row![
            if new { ico(Icon::DotFilled, 8).color(p().modified) } else { text("") }.width(10),
            row_tip(
                container(
                    row![
                        text(name).size(UI).color(color).wrapping(text::Wrapping::None),
                        text(dir).size(SMALL).color(p().muted).wrapping(text::Wrapping::None),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center)
                )
                .width(Length::Fill)
                .clip(true),
                full,
            ),
        ]
        .align_y(Alignment::Center)
        .spacing(4);
        // The caller builds `actions` only for the hovered or open row (see `row_actions`).
        if !actions.is_empty() {
            for a in actions {
                content = content.push(a);
            }
        } else if origin && let Some(run) = self.runs.origin(&c.path) {
            let label = format!("{} · {}", run.agent, ago(run.finished.unwrap_or(run.started)));
            let tip_text = match &run.prompt {
                Some(prompt) => text(prompt.as_str()),
                None => text(format!("Changed by {}", run.agent)),
            };
            content = content.push(tooltip(
                text(label)
                    .size(SMALL)
                    .color(if new { p().warn_text } else { p().muted })
                    .wrapping(text::Wrapping::None),
                container(tip_text.size(12)).padding(6).max_width(320).style(theme::bordered(p().bg)),
                tooltip::Position::Left,
            ));
        }
        content = content.push(text(c.code.to_string()).size(SMALL).color(color).width(12));
        // Same height with or without the row's buttons.
        let row = button(container(content).height(22).align_y(Alignment::Center))
            .width(Length::Fill)
            .padding([1, 6])
            .style(theme::flat(active))
            .on_press(msg);
        match hover {
            Some(key) => {
                mouse_area(row).on_enter(Msg::HoverChange(key, true)).on_exit(Msg::HoverChange(key, false)).into()
            }
            None => row.into(),
        }
    }

    fn hint<'a>(&self, msg: &'a str) -> Element<'a, Msg> {
        container(text(msg).size(SMALL).color(p().muted)).padding([8, 10]).into()
    }

    fn view_history(&self) -> Column<'_, Msg> {
        let mut history = Column::new().spacing(1);
        for c in &self.git.log {
            let selected = self.commit_view.selected.as_ref().filter(|s| s.hash == c.hash);
            let r = row![
                text(if selected.is_some() { "▾" } else { "▸" }).size(SMALL).color(p().accent).width(10),
                text(c.subject.as_str()).size(UI).wrapping(text::Wrapping::None),
            ]
            .spacing(6)
            .align_y(Alignment::Center);
            // Refs get the panel's full width instead of the subject's leftover space.
            let mut entry = column![r].spacing(3).width(Length::Fill);
            if !c.refs.is_empty() {
                entry = entry.push(container(badge(c.refs.as_str(), p().accent)).padding(Padding::ZERO.left(16)));
            }
            history = history.push(tooltip(
                button(container(entry).width(Length::Fill).clip(true))
                    .padding([3, 10])
                    .width(Length::Fill)
                    .style(theme::flat(selected.is_some()))
                    .on_press(Msg::HistorySelect(Arc::clone(&c.hash))),
                container(
                    column![text(c.subject.as_str()).size(12), text(c.detail.as_str()).size(12).color(p().muted),]
                        .spacing(2),
                )
                .padding(6)
                .max_width(420)
                .style(theme::bordered(p().bg)),
                tooltip::Position::Bottom,
            ));
            if let Some(selected) = selected {
                let mut details = column![text(c.detail.as_str()).size(SMALL).color(p().muted)].spacing(4);
                match &selected.result {
                    None => {
                        details = details.push(text("Loading commit…").size(SMALL).color(p().muted));
                    }
                    Some(Err(error)) => {
                        details = details.push(text(error.as_str()).size(SMALL).color(p().error_text));
                    }
                    Some(Ok(commit)) => {
                        details = details.push(text(commit.comparison.as_str()).size(SMALL).color(p().muted));
                        if !commit.body.is_empty() {
                            details = details.push(text(commit.body.as_str()).size(SMALL));
                        }
                        if commit.files.is_empty() {
                            details =
                                details.push(text("No changed files in this folder").size(SMALL).color(p().muted));
                        }
                        for (index, file) in commit.files.iter().enumerate() {
                            let active = self.active_diff().is_some_and(|d| {
                                d.rel == file.path
                                    && matches!(&d.source, DiffSource::Commit { details } if details.oid == commit.oid)
                            });
                            details = details.push(row_tip(
                                button(container(history_file_label(file)).width(Length::Fill).clip(true))
                                    .width(Length::Fill)
                                    .padding([3, 6])
                                    .style(theme::flat(active))
                                    .on_press(Msg::HistoryOpen(selected.id, index)),
                                file.path.as_str(),
                            ));
                        }
                        if commit.truncated {
                            details = details
                                .push(text("File list limited to 2,000 changes").size(SMALL).color(p().warn_text));
                        }
                    }
                }
                history = history.push(container(details).padding(Padding::new(6.0).left(26)).width(Length::Fill));
            }
        }
        if self.git.log.is_empty() {
            history = history.push(self.hint("No commits yet"));
        }
        history
    }

    fn view_git(&self) -> Element<'_, Msg> {
        // Until the folder's first snapshot, `git.root` says nothing yet.
        if !self.git_loaded {
            return self.hint("Loading changes…");
        }
        if self.git.root.is_none() {
            return container(
                column![
                    text("This folder is not a git repository.").size(UI).color(p().muted),
                    button(
                        row![
                            self.action_icon(Icon::SourceControl, &Msg::GitInit, 14),
                            text("Initialize repository").size(UI)
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .style(theme::primary)
                    .padding([5, 12])
                    .on_press_maybe((!self.git_busy).then_some(Msg::GitInit)),
                    text("Search and Files still work.").size(SMALL).color(p().muted),
                ]
                .spacing(10)
                .align_x(Alignment::Center),
            )
            .center_x(Length::Fill)
            .padding(24)
            .into();
        }
        let busy = self.git_busy;
        let staged = &self.git.staged;
        let unstaged = &self.git.unstaged;
        // Unmerged paths get their own section and never ride along in Commit All.
        let (conflicts, changes): (Vec<&Change>, Vec<&Change>) = unstaged.iter().partition(|c| c.code == '!');
        let has_changes = !staged.is_empty() || !unstaged.is_empty();
        let can_commit = !busy && conflicts.is_empty() && has_changes && !self.commit_msg.text().trim().is_empty();
        let generating = self.commit_gen.is_some();
        let can_generate = generating || has_changes;
        let commit_all = staged.is_empty() && !changes.is_empty();
        let commit_label: Cow<'_, str> =
            if commit_all { format!("Commit All ({})", changes.len()).into() } else { "Commit".into() };
        let commit_tip: Cow<'_, str> = if !conflicts.is_empty() {
            "Resolve the merge conflicts first".into()
        } else if commit_all {
            let untracked = changes.iter().filter(|c| c.code == 'U').count();
            let n = changes.len();
            match untracked {
                0 => format!("Nothing staged: stages and commits all {n} changes in this folder").into(),
                u => format!(
                    "Nothing staged: stages and commits all {n} changes in this folder, \
                     including {u} untracked {}",
                    if u == 1 { "file" } else { "files" }
                )
                .into(),
            }
        } else {
            "Commit the staged changes (Ctrl+Enter)".into()
        };
        let open = match self.active_tab.and_then(|i| self.tabs.get(i)) {
            Some(Tab::Diff(DiffTab { rel, source: DiffSource::Index { staged }, .. })) => Some((rel.as_str(), *staged)),
            _ => None,
        };

        let mut list = Column::new().spacing(1);
        // Row buttons, VSCodium's order: open the file, discard, stage / unstage.
        let open_file = |c: &Change| -> Element<'_, Msg> {
            if c.code == 'D' {
                tip(icon_button_maybe(Icon::GoToFile, None), "Deleted: nothing to open")
            } else {
                tip(icon_button(Icon::GoToFile, Msg::OpenChangedFile(c.path.clone())), "Open file")
            }
        };
        if !staged.is_empty() {
            let all: Vec<String> = staged.iter().map(|c| c.path.clone()).collect();
            list = list.push(section(
                format!("STAGED  {}", staged.len()),
                vec![
                    tip(icon_button(Icon::Diff, Msg::OpenChanges(true)), "Review staged changes file by file"),
                    tip(self.git_button(Icon::Remove, Msg::Unstage(all)), "Unstage all"),
                ],
            ));
            for (i, c) in staged.iter().enumerate() {
                let working =
                    matches!(&self.busy_op, Some(BusyOp::Unstage(paths)) if paths.as_slice() == [c.path.as_str()]);
                list = list.push(self.change_button(
                    c,
                    open == Some((c.path.as_str(), true)),
                    Msg::OpenDiff(c.path.clone(), true),
                    Some((0, i)),
                    row_actions(
                        self.hovered_change == Some((0, i)) || open == Some((c.path.as_str(), true)) || working,
                        || {
                            vec![
                                open_file(c),
                                tip(
                                    self.git_button(Icon::Remove, Msg::Unstage(vec![c.path.clone()])),
                                    "Unstage changes",
                                ),
                            ]
                        },
                    ),
                    true,
                ));
            }
        }
        if !conflicts.is_empty() {
            list = list.push(section(format!("MERGE CHANGES  {}", conflicts.len()), vec![]));
            for (i, c) in conflicts.iter().enumerate() {
                let working =
                    matches!(&self.busy_op, Some(BusyOp::Stage(paths)) if paths.as_slice() == [c.path.as_str()]);
                list = list.push(self.change_button(
                    c,
                    open == Some((c.path.as_str(), false)),
                    Msg::OpenDiff(c.path.clone(), false),
                    Some((1, i)),
                    row_actions(
                        self.hovered_change == Some((1, i)) || open == Some((c.path.as_str(), false)) || working,
                        || {
                            vec![
                                open_file(c),
                                tip(
                                    self.git_button(Icon::Add, Msg::Stage(vec![c.path.clone()])),
                                    "Mark as resolved (stage it once the conflict markers are gone)",
                                ),
                            ]
                        },
                    ),
                    true,
                ));
            }
        }
        let all: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
        list = list.push(section(
            format!("CHANGES  {}", changes.len()),
            if all.is_empty() {
                vec![]
            } else {
                let discard_all = tip(self.git_button(Icon::Discard, Msg::DiscardAll), "Discard all changes");
                vec![
                    tip(icon_button(Icon::Diff, Msg::OpenChanges(false)), "Review changes file by file"),
                    discard_all,
                    tip(self.git_button(Icon::Add, Msg::Stage(all)), "Stage all changes"),
                ]
            },
        ));
        for (i, c) in changes.iter().enumerate() {
            let working = matches!(&self.busy_op, Some(BusyOp::Stage(paths) | BusyOp::Discard(paths)) if paths.as_slice() == [c.path.as_str()]);
            let discard_tip = if c.code == 'U' {
                "Discard: delete this new file (Undo available)"
            } else {
                "Discard changes (Undo available)"
            };
            list = list.push(self.change_button(
                c,
                open == Some((c.path.as_str(), false)),
                Msg::OpenDiff(c.path.clone(), false),
                Some((2, i)),
                row_actions(
                    self.hovered_change == Some((2, i)) || open == Some((c.path.as_str(), false)) || working,
                    || {
                        vec![
                            open_file(c),
                            tip(self.git_button(Icon::Discard, Msg::Discard(vec![c.path.clone()])), discard_tip),
                            tip(self.git_button(Icon::Add, Msg::Stage(vec![c.path.clone()])), "Stage changes"),
                        ]
                    },
                ),
                true,
            ));
        }
        if !has_changes {
            list = list.push(self.hint("No changes — agent edits show up here"));
        }
        if self.git.truncated {
            list = list.push(
                container(
                    text(format!("Showing first {} changes", crate::git::MAX_CHANGES)).size(SMALL).color(p().warn_text),
                )
                .padding([4, 10]),
            );
        }

        let history = self.view_history();

        let mut branch = row![text(self.git.branch.as_str()).size(UI)].spacing(8);
        if self.git.ahead > 0 {
            branch = branch.push(text(format!("↑{}", self.git.ahead)).size(UI));
        }
        if self.git.behind > 0 {
            branch = branch.push(text(format!("↓{}", self.git.behind)).size(UI));
        }
        column![
            container(
                column![
                    row![
                        tip(
                            button(
                                container(
                                    row![
                                        if matches!(self.busy_op, Some(BusyOp::Branches | BusyOp::SwitchBranch(_))) {
                                            self.spinner(14)
                                        } else {
                                            ico(Icon::Branch, 14).color(p().accent)
                                        },
                                        container(branch).width(Length::Fill).clip(true),
                                        ico(Icon::ChevronDown, 12),
                                    ]
                                    .spacing(6)
                                    .align_y(Alignment::Center)
                                )
                                .width(Length::Fill)
                                .clip(true)
                            )
                            .width(Length::Fill)
                            .padding([3, 4])
                            .style(theme::flat(false))
                            .on_press_maybe((!busy).then_some(Msg::BranchesOpen)),
                            "Switch branch…"
                        ),
                        self.chatgpt_account.as_ref().map(|email| tip(
                            icon_button(Icon::Account, Msg::ChatGptSignOut),
                            if email.is_empty() {
                                "Signed in to ChatGPT — click to sign out".to_string()
                            } else {
                                format!("ChatGPT: {email} — click to sign out")
                            },
                        )),
                        tip(self.git_button(Icon::ArrowDown, Msg::Pull), "Pull (fast-forward)"),
                        tip(
                            self.git_button(Icon::ArrowUp, Msg::Push),
                            if self.git.upstream.is_some() { "Push to configured remote" } else { "Publish branch…" }
                        ),
                        tip(
                            button(if self.manual_refresh { self.spinner(15) } else { ico(Icon::Refresh, 15) })
                                .padding([3, 5])
                                .style(theme::flat(false))
                                .on_press_maybe((!busy && !self.manual_refresh).then_some(Msg::RefreshChanges)),
                            if self.manual_refresh { "Refreshing changes…" } else { "Refresh" },
                        ),
                    ]
                    .align_y(Alignment::Center),
                    row![
                        // Grows with the message (wrapped to the panel width), then scrolls.
                        iced::widget::text_editor(&self.commit_msg)
                            .id(COMMIT_INPUT)
                            .placeholder("Commit message (Ctrl+Enter to commit)")
                            .on_action(Msg::CommitEdit)
                            .size(UI)
                            .padding(6)
                            .height(Length::Shrink)
                            .max_height(320.0)
                            .wrapping(text::Wrapping::WordOrGlyph)
                            .style(|_, status| {
                                let c = p();
                                text_editor::Style {
                                    background: c.bg.into(),
                                    border: iced::Border {
                                        color: if matches!(status, text_editor::Status::Focused { .. }) {
                                            c.accent
                                        } else {
                                            c.border
                                        },
                                        width: 1.0,
                                        radius: 4.0.into(),
                                    },
                                    placeholder: c.muted,
                                    value: c.fg,
                                    selection: c.selection,
                                }
                            })
                            .key_binding(|kp| match kp.key.as_ref() {
                                // Unfocused, iced still asks (see the file editor's bindings).
                                _ if !matches!(kp.status, text_editor::Status::Focused { .. }) => None,
                                Key::Named(Named::Enter) if kp.modifiers.command() =>
                                    Some(Binding::Custom(Msg::Commit)),
                                Key::Character(c) if kp.modifiers.command() && c.eq_ignore_ascii_case("z") =>
                                    Some(Binding::Custom(if kp.modifiers.shift() {
                                        Msg::CommitRedo
                                    } else {
                                        Msg::CommitUndo
                                    })),
                                Key::Character(c) if kp.modifiers.command() && c.eq_ignore_ascii_case("y") =>
                                    Some(Binding::Custom(Msg::CommitRedo)),
                                _ => Binding::from_key_press(kp),
                            }),
                        tooltip(
                            button(
                                container(if generating {
                                    self.spinner(16)
                                } else {
                                    ico(Icon::Sparkle, 16).color(if can_generate { p().accent } else { p().muted })
                                })
                                .center(Length::Fill),
                            )
                            .width(30)
                            .height(30)
                            .padding(0)
                            .style(theme::outline(generating))
                            .on_press_maybe(can_generate.then_some(Msg::GenerateCommit)),
                            container(
                                text(if generating {
                                    "Generating… click to stop"
                                } else {
                                    "Generate commit message"
                                })
                                .size(12)
                            )
                            .padding(6)
                            .style(theme::bordered(p().bg)),
                            tooltip::Position::Left,
                        ),
                    ]
                    .spacing(4)
                    .align_y(Alignment::Start),
                    tip(
                        button(
                            container(
                                row![
                                    if self.busy_op == Some(BusyOp::Commit) {
                                        self.spinner(14)
                                    } else {
                                        ico(Icon::Check, 14)
                                    },
                                    text(commit_label).size(UI)
                                ]
                                .spacing(6)
                                .align_y(Alignment::Center)
                            )
                            .center_x(Length::Fill),
                        )
                        .width(Length::Fill)
                        .padding(5)
                        .style(theme::primary)
                        .on_press_maybe(can_commit.then_some(Msg::Commit)),
                        commit_tip,
                    ),
                    (!conflicts.is_empty()).then(|| {
                        text("Merge conflicts: fix each file, then stage it (+) before committing.")
                            .size(SMALL)
                            .color(p().warn_text)
                    }),
                ]
                .spacing(6)
            )
            .padding(10),
            self.view_changes_toolbar(),
            hrule(),
            // Reserve a scrollbar gutter so run metadata and row actions stay visible.
            scrollable(if self.group_by_run { self.view_runs() } else { list }).spacing(2).height(Length::Fill),
            hdivider(
                Divider::History,
                self.dragging == Some(Divider::History) || self.hovered_divider == Some(Divider::History),
            ),
            section(
                "HISTORY".into(),
                vec![tip(
                    button(
                        row![
                            self.action_icon(Icon::Discard, &Msg::UndoLastCommit, 14),
                            text("Undo Last Commit").size(SMALL)
                        ]
                        .spacing(5)
                        .align_y(Alignment::Center)
                    )
                    .padding([2, 6])
                    .style(theme::flat(false))
                    .on_press_maybe((!busy && !self.git.log.is_empty()).then_some(Msg::UndoLastCommit)),
                    "Remove the last local commit; keep its changes staged"
                )]
            ),
            scrollable(history).spacing(2).height(self.history_height),
        ]
        .into()
    }

    /// `Files | By agent run` switch and "mark all seen".
    fn view_changes_toolbar(&self) -> Element<'_, Msg> {
        let seg = |label: &'static str, on: bool, msg: Msg| {
            button(text(label).size(12)).padding([2, 10]).style(theme::flat(on)).on_press(msg)
        };
        let mut bar = row![
            seg("Files", !self.group_by_run, Msg::GroupByRun(false)),
            seg("By agent run", self.group_by_run, Msg::GroupByRun(true)),
            space::horizontal(),
        ]
        .spacing(2)
        .align_y(Alignment::Center);
        let fresh = self.fresh_changes();
        if fresh > 0 {
            bar = bar
                .push(text(format!("{fresh} new")).size(SMALL).color(p().warn_text))
                .push(tip(icon_button(Icon::Check, Msg::MarkAllSeen), "Mark all as seen"));
        }
        container(bar.spacing(6)).padding([4, 10]).width(Length::Fill).into()
    }

    /// Agent runs that changed files, newest first, each with its files.
    fn view_runs(&self) -> Column<'_, Msg> {
        let mut list = Column::new().spacing(1);
        let open = match self.active_tab.and_then(|i| self.tabs.get(i)) {
            Some(Tab::Diff(DiffTab { rel, source: DiffSource::Trees { run, .. }, .. })) => Some((*run, rel.as_str())),
            _ => None,
        };
        let mut any = false;
        for r in self.runs.runs().iter().rev() {
            let working = r.running();
            if !working && !r.has_changes() {
                continue;
            }
            any = true;
            let selected = self.selected_run == Some(r.id);
            let n = r.files.as_ref().map_or(0, Vec::len);
            let meta = if working {
                format!("{} · working…", r.agent)
            } else {
                format!(
                    "{} · {} · {n} {}",
                    r.agent,
                    ago(r.finished.unwrap_or(r.started)),
                    if n == 1 { "file" } else { "files" }
                )
            };
            let title = match &r.prompt {
                Some(prompt) => text(prompt.as_str()),
                None => text(format!("{} run", r.agent)),
            };
            let mut header = row![
                column![
                    title.size(UI).wrapping(text::Wrapping::WordOrGlyph),
                    text(meta).size(SMALL).color(if working { p().accent } else { p().muted }),
                ]
                .spacing(1)
                .width(Length::Fill),
            ]
            .spacing(4)
            .align_y(Alignment::Center);
            if selected && self.agents.iter().any(|a| a.pane_id == r.pane) {
                header =
                    header.push(tip(icon_button(Icon::Terminal, Msg::FocusAgent(r.pane.clone())), "Show this agent"));
            }
            if selected && !working {
                let idle = !self.git_busy;
                header = header
                    .push(tip(self.git_button(Icon::Add, Msg::StageRun(r.id)), "Stage this run's files"))
                    .push(tip(
                        button(self.action_icon(Icon::Discard, &Msg::RevertRun(r.id), 14))
                            .padding([3, 5])
                            .style(theme::flat_fn)
                            .on_press_maybe(idle.then_some(Msg::RevertRun(r.id))),
                        "Revert this run",
                    ));
            }
            list = list.push(
                button(header)
                    .width(Length::Fill)
                    .padding([5, 10])
                    .style(theme::flat(selected))
                    .on_press(Msg::SelectRun(r.id)),
            );
            for c in r.files.iter().flatten() {
                list = list.push(
                    container(self.change_button(
                        c,
                        open == Some((r.id, c.path.as_str())),
                        Msg::OpenRunDiff(r.id, c.path.clone()),
                        None,
                        Vec::new(),
                        false,
                    ))
                    .padding(Padding { left: 12.0, ..Padding::ZERO }),
                );
            }
        }
        if !any {
            list = list.push(self.hint(match (self.term_kind, self.has_repo()) {
                (TermKind::Shell, _) => "Grouping by agent run needs herdr in the terminal.",
                (_, false) => "Agent runs need a git repository.",
                _ => "When an agent working in this folder changes files, its run shows up here.",
            }));
        }
        list
    }

    fn view_search(&self) -> Element<'_, Msg> {
        let s = &self.search;
        let toggle = |label, on, which, name| option_toggle(label, on, Msg::SearchToggle(which), name);
        let capped = if s.total >= search::MAX_MATCHES { "+" } else { "" };
        // The modal confirms before any file is rewritten.
        let replace: Element<'_, Msg> = if self.replacing {
            button(row![self.spinner(13), text("Replacing…").size(12)].spacing(5).align_y(Alignment::Center))
                .padding([2, 8])
                .style(theme::flat(false))
                .into()
        } else {
            small_button("Replace all", (!s.results.is_empty() && !s.running).then_some(Msg::ReplaceAll))
        };
        let inputs = column![
            row![
                text_input("Search", &s.query)
                    .id(SEARCH_INPUT)
                    .on_input(Msg::SearchQuery)
                    .on_submit(Msg::SearchRun)
                    .size(UI)
                    .padding(6)
                    .style(theme::input),
                toggle("Aa", s.opts.case_sensitive, 0, "Match Case"),
                toggle("ab", s.opts.whole_word, 1, "Match Whole Word"),
                toggle(".*", s.opts.regex, 2, "Use Regular Expression"),
            ]
            .spacing(2)
            .align_y(Alignment::Center),
            row![
                // No on_submit: Enter must never rewrite files.
                text_input("Replace", &s.replace).on_input(Msg::SearchReplace).size(UI).padding(6).style(theme::input),
                replace,
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        ]
        .spacing(6);

        let summary: Element<'_, Msg> = if let Some(e) = &s.error {
            text(e.as_str()).size(12).color(p().error_text).into()
        } else if s.query.is_empty() {
            text("Type to search · respects .gitignore").size(12).color(p().muted).into()
        } else if s.running && s.results.is_empty() {
            text("Searching…").size(12).color(p().muted).into()
        } else if s.results.is_empty() {
            text("No results").size(12).color(p().muted).into()
        } else {
            // Results of the previous query stay while the new one runs.
            row![
                text(format!("{}{capped} results in {} files", s.total, s.results.len())).size(12).color(p().muted),
                s.running.then(|| text("· Searching…").size(12).color(p().muted)),
            ]
            .spacing(6)
            .into()
        };

        let mut list = Column::new();
        let mut shown = 0;
        'files: for (fi, f) in s.results.iter().enumerate() {
            let (name, dir) = split_path(&f.rel);
            list = list.push(row_tip(
                button(
                    row![
                        container(
                            row![
                                text(name).size(UI).wrapping(text::Wrapping::None),
                                text(dir).size(SMALL).color(p().muted).wrapping(text::Wrapping::None),
                            ]
                            .spacing(6)
                            .align_y(Alignment::Center),
                        )
                        .width(Length::Fill)
                        .clip(true),
                        text(f.count).size(SMALL).color(p().accent),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([3, 10])
                .style(theme::flat(false))
                .on_press(Msg::SearchOpen(s.shown, fi, None)),
                f.rel.as_str(),
            ));
            for (li, m) in f.lines.iter().enumerate() {
                if shown >= MAX_RESULT_LINES {
                    list = list.push(
                        container(text("Too many results to display").size(SMALL).color(p().muted)).padding([4, 10]),
                    );
                    break 'files;
                }
                shown += 1;
                // Preview and ranges come from search.rs; spans only borrow them.
                let preview = m.preview.as_str();
                let mut spans: Vec<Span<'_, (), Font>> = Vec::with_capacity(m.ranges.len() * 2 + 1);
                let mut at = 0;
                for &(a, b) in &m.ranges {
                    if a > at {
                        spans.push(span(&preview[at..a]));
                    }
                    spans.push(span(&preview[a..b]).background(p().selection).color(p().fg));
                    at = b;
                }
                if at < preview.len() {
                    spans.push(span(&preview[at..]));
                }
                list = list.push(
                    row![
                        button(rich_text(spans).size(12).wrapping(text::Wrapping::None))
                            .width(Length::Fill)
                            .padding(Padding { top: 1.0, right: 4.0, bottom: 1.0, left: 26.0 })
                            .style(theme::flat(false))
                            .on_press(Msg::SearchOpen(s.shown, fi, Some(li))),
                        tip(icon_button(Icon::Send, Msg::SearchSend(s.shown, fi, li)), "Send reference to agent"),
                    ]
                    .align_y(Alignment::Center),
                );
            }
        }

        column![container(column![inputs, summary].spacing(8)).padding(10), scrollable(list).height(Length::Fill),]
            .into()
    }

    fn view_explorer(&self) -> Element<'_, Msg> {
        let open = self.active_tab.and_then(|i| self.tabs.get(i)).and_then(Tab::file_path);
        let mut list = Column::new();
        let edit = self.name_edit.as_ref();
        // A new entry in the folder itself is typed at the top.
        if let Some(e) = edit.filter(|e| e.kind != EditKind::Rename && e.target == self.root) {
            list = list.push(self.name_input(e, 0));
        }
        // Status and ignored flags are computed in update (see `Explorer::set_marks`).
        for e in &self.explorer.entries {
            let color = match (e.ignored, e.status) {
                (true, _) => p().muted,
                (_, Some(c)) => theme::status_color(c),
                _ => p().fg,
            };
            let chevron: Element<'_, Msg> = if e.is_dir {
                ico(if e.expanded { Icon::ChevronDown } else { Icon::ChevronRight }, 13)
                    .color(p().muted)
                    .width(14)
                    .into()
            } else {
                space().width(14).into()
            };
            let kind = entry_icon(e.is_dir, e.expanded, &e.name, e.ignored);
            let renaming = edit.filter(|ed| ed.kind == EditKind::Rename && ed.target == e.path);
            let name: Element<'_, Msg> = match renaming {
                Some(ed) => self.name_field(ed),
                None => container(text(e.name.as_str()).size(UI).color(color).wrapping(text::Wrapping::None))
                    .width(Length::Fill)
                    .clip(true)
                    .into(),
            };
            let mut line = row![space::horizontal().width(e.depth as f32 * 12.0), chevron, kind, name]
                .align_y(Alignment::Center)
                .spacing(4);
            if let Some(code) = e.status.filter(|_| !e.is_dir) {
                line = line.push(text(code).size(SMALL).color(theme::status_color(code)));
            }
            let menu_open = self.explorer_menu.as_ref().is_some_and(|m| m.path == e.path);
            let row_button = button(line)
                .width(Length::Fill)
                .padding([2, 8])
                .style(theme::flat(open == Some(e.path.as_path()) || menu_open));
            // While renaming, clicks go to the input, not to the row.
            let row_button = if renaming.is_some() {
                row_button
            } else {
                row_button.on_press(Msg::ExplorerClick(e.path.clone(), e.is_dir))
            };
            list = list.push(row_tip(
                mouse_area(row_button).on_right_press(Msg::ExplorerTarget(e.path.clone(), e.is_dir)),
                e.name.as_str(),
            ));
            if let Some(ed) = edit.filter(|ed| ed.kind != EditKind::Rename && e.is_dir && ed.target == e.path) {
                list = list.push(self.name_input(ed, e.depth + 1));
            }
        }
        if self.explorer.entries.is_empty() && edit.is_none() {
            list = list.push(self.hint("Empty folder · right-click for New File"));
        }
        // A right click anywhere in the panel opens the menu there (on a row: for it).
        crate::rightclick::right_click(
            column![
                container(text(file_name(&self.root).to_uppercase()).size(SMALL).color(p().muted)).padding([6, 10]),
                scrollable(list).height(Length::Fill),
            ],
            Msg::ExplorerMenuAt,
        )
    }

    /// The name input of `edit`, without its row.
    fn name_field<'a>(&'a self, edit: &'a NameEdit) -> Element<'a, Msg> {
        let input = text_input(if edit.kind == EditKind::NewFolder { "Folder name" } else { "File name" }, &edit.name)
            .id(NAME_INPUT)
            .on_input(Msg::ExplorerName)
            .on_submit(Msg::ExplorerNameSubmit)
            .size(UI)
            .padding([1, 4])
            .style(theme::input);
        match &edit.error {
            None => input.into(),
            Some(err) => column![input, text(err.as_str()).size(SMALL).color(p().error_text)].spacing(2).into(),
        }
    }

    /// A row to type a new entry's name in, at `depth`.
    fn name_input<'a>(&'a self, edit: &'a NameEdit, depth: usize) -> Element<'a, Msg> {
        let folder = edit.kind == EditKind::NewFolder;
        container(
            row![
                space::horizontal().width(depth as f32 * 12.0 + 14.0),
                entry_icon(folder, false, &edit.name, false),
                self.name_field(edit),
            ]
            .align_y(Alignment::Center)
            .spacing(4),
        )
        .padding([2, 8])
        .into()
    }

    /// The Files context menu, near the pointer and kept inside the window.
    fn view_explorer_menu(&self, menu: &ExplorerMenu) -> Element<'_, Msg> {
        let item = |label: &'static str, action: MenuAction, danger: bool| -> Element<'_, Msg> {
            button(text(label).size(UI).color(if danger { p().error_text } else { p().fg }))
                .width(Length::Fill)
                .padding([4, 12])
                .style(theme::flat(false))
                .on_press(Msg::ExplorerMenu(action))
                .into()
        };
        let sep = || container(hrule()).padding([3, 6]);
        let mut items =
            column![item("New File…", MenuAction::NewFile, false), item("New Folder…", MenuAction::NewFolder, false)];
        let mut rows = 2;
        let mut seps = 0;
        if !menu.is_root {
            items = items.push(sep()).push(item("Rename…", MenuAction::Rename, false)).push(item(
                "Delete (to the trash)",
                MenuAction::Delete,
                true,
            ));
            rows += 2;
            seps += 1;
        }
        items = items.push(sep()).push(item("Copy Path", MenuAction::CopyPath, false));
        rows += 1;
        seps += 1;
        if !menu.is_root {
            items = items.push(item("Copy Relative Path", MenuAction::CopyRelativePath, false));
            rows += 1;
        }
        if self.term.is_some() {
            items = items.push(item("Send Path to Agent", MenuAction::SendPath, false));
            rows += 1;
        }
        const WIDTH: f32 = 220.0;
        let height = rows as f32 * 27.0 + seps as f32 * 7.0 + 10.0;
        let x = menu.at.x.min(self.window_width - WIDTH - 4.0).max(0.0);
        let y = menu.at.y.min(self.window_height - height - 4.0).max(0.0);
        let panel = container(items.spacing(1)).width(WIDTH).padding(4).style(|_| container::Style {
            background: Some(p().sidebar.into()),
            text_color: Some(p().fg),
            border: iced::Border { color: p().border, width: 1.0, radius: 6.0.into() },
            shadow: iced::Shadow {
                color: Color::BLACK.scale_alpha(0.25),
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..Default::default()
        });
        iced::widget::pin(iced::widget::opaque(panel)).x(x).y(y).into()
    }

    // ------------------------------------------------------------- documents

    fn view_document(&self) -> Element<'_, Msg> {
        let mut tabs = row![].spacing(1);
        for (i, t) in self.tabs.iter().enumerate() {
            let active = self.active_tab == Some(i);
            let title = match t {
                Tab::File(f) => {
                    let name = file_name(&f.path);
                    if f.dirty { format!("● {name}") } else { name }
                }
                Tab::Image(f) => match f.image {
                    ImageData::Raster(_) => file_name(&f.path),
                    ImageData::Svg(_) => format!("{} (Preview)", file_name(&f.path)),
                },
                Tab::Diff(d) => {
                    let (name, _) = split_path(&d.rel);
                    let side = match &d.source {
                        DiffSource::Index { staged: true } => "Index".to_string(),
                        DiffSource::Index { staged: false } => "Working Tree".to_string(),
                        DiffSource::Trees { label, .. } => label.clone(),
                        DiffSource::Commit { details } => details.label.clone(),
                    };
                    format!("⇄ {name} ({side})")
                }
            };
            let close = tip(
                button(text("×").size(14)).padding([4, 6]).style(theme::tab(active)).on_press(Msg::CloseTab(i)),
                "Close (Ctrl+W)",
            );
            let tab = container(
                row![
                    button(text(title).size(UI).wrapping(text::Wrapping::None))
                        .padding(Padding { top: 6.0, right: 4.0, bottom: 6.0, left: 12.0 })
                        .style(theme::tab(active))
                        .on_press(Msg::SelectTab(i)),
                    close,
                ]
                .align_y(Alignment::Center),
            )
            .style(move |_| container::Style {
                background: Some(if active { p().bg } else { p().sidebar }.into()),
                border: iced::Border {
                    color: if active { p().accent } else { Color::TRANSPARENT },
                    width: if active { 1.0 } else { 0.0 },
                    radius: 0.0.into(),
                },
                ..Default::default()
            });
            // Middle click closes, as in VSCodium.
            tabs = tabs.push(mouse_area(tab).on_middle_press(Msg::CloseTab(i)));
        }
        let tab_bar = container(
            scrollable(tabs)
                .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(3).scroller_width(3))),
        )
        .width(Length::Fill)
        .style(theme::panel(p().sidebar));

        let body: Element<'_, Msg> = match self.active_tab.and_then(|i| self.tabs.get(i)) {
            None => space().into(),
            Some(Tab::File(t)) => self.view_editor(t),
            Some(Tab::Diff(d)) => self.view_diff(d),
            Some(Tab::Image(t)) => self.view_image(t),
        };
        container(column![tab_bar, body]).width(Length::Fill).height(Length::Fill).style(theme::panel(p().bg)).into()
    }

    /// An image: zoom with the wheel and drag to pan (raster), or fit to the
    /// tab (SVG). Small images keep their size.
    fn view_image<'a>(&'a self, t: &'a ImageTab) -> Element<'a, Msg> {
        let mut info = vec![crate::imagefile::human_bytes(t.bytes)];
        if let Some((w, h)) = t.size {
            info.insert(0, format!("{w} × {h}"));
        }
        let hint = match t.image {
            ImageData::Raster(_) => "Scroll to zoom · drag to pan",
            ImageData::Svg(_) => "Scaled to fit",
        };
        let header = row![
            text(t.crumbs.as_str()).size(12).color(p().muted),
            text(info.join(" · ")).size(12).color(p().muted),
            t.gone.then(|| text("Deleted on disk").size(12).color(p().warn_text)),
            space::horizontal(),
            text(hint).size(12).color(p().muted),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let body: Element<'a, Msg> = match &t.image {
            ImageData::Raster(h) => iced::widget::image::viewer(h.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(16)
                .content_fit(iced::ContentFit::ScaleDown)
                .max_scale(32.0)
                .into(),
            ImageData::Svg(h) => container(
                iced::widget::svg(h.clone())
                    .content_fit(iced::ContentFit::Contain)
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .padding(24)
            .into(),
        };
        column![
            container(header).padding([4, 12]),
            container(body).width(Length::Fill).height(Length::Fill).style(theme::panel(p().sidebar)),
        ]
        .into()
    }

    fn view_editor<'a>(&'a self, t: &'a FileTab) -> Element<'a, Msg> {
        let editor = iced::widget::text_editor(&t.content)
            .id(EDITOR)
            .on_action(Msg::Edit)
            .font(theme::mono_font())
            .size(CODE_SIZE)
            .height(Length::Fill)
            // No wrapping: one row per line, so the line numbers can follow (gutter.rs).
            .wrapping(text::Wrapping::None)
            .padding(Padding { top: EDITOR_PADDING_TOP, right: 12.0, bottom: EDITOR_PADDING_TOP, left: 8.0 })
            .style(|_, _| text_editor::Style {
                background: p().bg.into(),
                border: iced::Border::default(),
                placeholder: p().muted,
                value: p().fg,
                selection: p().selection,
            })
            .key_binding(|kp| {
                // iced asks for bindings even while the editor is unfocused, and
                // publishes (and captures) a custom one regardless: none may take a
                // key typed in the terminal or the commit box (`keys.rs` covers Ctrl+S
                // and Ctrl+Z when no widget has the keyboard).
                if !matches!(kp.status, text_editor::Status::Focused { .. }) {
                    return None;
                }
                let ctrl = kp.modifiers.command();
                let plain_ctrl = ctrl && !kp.modifiers.shift() && !kp.modifiers.alt();
                match kp.key.as_ref() {
                    Key::Character(c) if ctrl && c.eq_ignore_ascii_case("s") => Some(Binding::Custom(Msg::Save)),
                    Key::Character(c) if plain_ctrl && c.eq_ignore_ascii_case("f") => {
                        Some(Binding::Custom(Msg::FindOpen))
                    }
                    Key::Character(c) if plain_ctrl && c.eq_ignore_ascii_case("g") => {
                        Some(Binding::Custom(Msg::PaletteOpen(true)))
                    }
                    Key::Character("\\" | "|") if ctrl && kp.modifiers.shift() => {
                        Some(Binding::Custom(Msg::MatchBracket))
                    }
                    Key::Character(c) if ctrl && c.eq_ignore_ascii_case("z") => {
                        Some(Binding::Custom(if kp.modifiers.shift() { Msg::Redo } else { Msg::Undo }))
                    }
                    Key::Character(c) if ctrl && c.eq_ignore_ascii_case("y") => Some(Binding::Custom(Msg::Redo)),
                    Key::Named(Named::Tab) if !ctrl => Some(Binding::Sequence(vec![Binding::Insert(' '); 4])),
                    _ => Binding::from_key_press(kp),
                }
            })
            .highlight_with::<crate::code::Highlighter>(
                crate::code::Settings {
                    token: t.syntax.clone(),
                    theme: p().syntax,
                    colors: p().brackets(),
                    error: p().error_text,
                },
                |format, _| *format,
            );
        let editor = crate::codeview::editor(editor, t.structure.index.as_deref(), t.structure.indent.unwrap_or(4));
        let editor = iced::widget::sensor(editor).on_show(Msg::EditorViewport).on_resize(Msg::EditorViewport);
        column![
            container(
                row![
                    text(t.crumbs.as_str()).size(12).color(p().muted),
                    space::horizontal(),
                    tip(
                        button(text("{ }").size(14)).on_press(Msg::MatchBracket).style(theme::flat_fn),
                        "Jump to the matching bracket or the end of this block (Ctrl+Shift+\\)",
                    ),
                    (crate::imagefile::kind(&t.path) == Some(crate::imagefile::Kind::Svg)).then(|| tip(
                        icon_label(Icon::Preview, "Preview", Some(Msg::OpenImage(t.path.clone()))),
                        "Show the image this SVG draws"
                    )),
                    icon_label(Icon::Send, "Send line to agent", Some(Msg::SendLine)),
                ]
                .align_y(Alignment::Center)
            )
            .padding([2, 12]),
            t.find.as_deref().map(|f| self.view_find_bar(f)),
            row![self.view_gutter(t), editor].height(Length::Fill),
        ]
        .into()
    }

    /// Find in the file: query, Match Case, `3/17`, previous / next, close.
    fn view_find_bar<'a>(&'a self, f: &'a FindBar) -> Element<'a, Msg> {
        let any = !f.matches.is_empty();
        let count: Element<'a, Msg> = if f.query.is_empty() {
            space().width(0).into()
        } else if !any {
            text("No results").size(SMALL).color(p().error_text).into()
        } else {
            let plus = if f.capped { "+" } else { "" };
            let at = f.current.map_or_else(|| "?".to_string(), |i| (i + 1).to_string());
            text(format!("{at}/{}{plus}", f.matches.len())).size(SMALL).color(p().muted).into()
        };
        let step = |forward: bool| any.then_some(Msg::FindStep(forward));
        let bar = row![
            text_input("Find", &f.query)
                .id(FIND_INPUT)
                .on_input(Msg::FindQuery)
                .on_submit(Msg::FindSubmit)
                .size(UI)
                .padding([3, 6])
                .width(280)
                .style(theme::input),
            option_toggle("Aa", f.case_sensitive, Msg::FindToggleCase, "Match Case"),
            container(count).width(Length::Fixed(72.0)).padding([0, 4]),
            tip(icon_button_maybe(Icon::ArrowUp, step(false)), "Previous match (Shift+Enter · Shift+F3)"),
            tip(icon_button_maybe(Icon::ArrowDown, step(true)), "Next match (Enter · F3)"),
            tip(icon_button(Icon::Close, Msg::FindClose), "Close (Escape)"),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        container(bar).padding([3, 12]).width(Length::Fill).style(theme::panel(p().sidebar)).into()
    }

    /// Line numbers beside the editor, lined up with its scroll (see
    /// `gutter.rs`); the cursor's line is highlighted, as in VSCodium. Only
    /// the visible numbers are built, and only when they change.
    fn view_gutter<'a>(&'a self, t: &'a FileTab) -> Element<'a, Msg> {
        let row_h = FileTab::row();
        let count = t.content.line_count();
        let width = crate::gutter::width(count, CODE_SIZE as f32 * 0.6);
        let numbers: Element<'a, Msg> = match self.editor_view {
            Some(view) => {
                let offset = t.scroll.offset(row_h, count, view);
                let (first, shown, cut) = crate::gutter::visible(offset, row_h, count, view);
                let cursor = t.content.cursor().position.line;
                // Lines with find matches on screen (the current one marked): the editor
                // only draws its selection while it has the keyboard, so these show the
                // way while typing in the find bar.
                let marks: Vec<(usize, bool)> = match &t.find {
                    Some(f) => {
                        let current = f.current.and_then(|i| f.matches.get(i)).map(|m| m.line);
                        let from = f.matches.partition_point(|m| m.line < first);
                        let mut marks: Vec<(usize, bool)> = Vec::new();
                        for m in f.matches[from..].iter().take_while(|m| m.line < first + shown) {
                            if marks.last().is_none_or(|(l, _)| *l != m.line) {
                                marks.push((m.line, current == Some(m.line)));
                            }
                        }
                        marks
                    }
                    None => Vec::new(),
                };
                lazy((first, shown, cut.to_bits(), cursor, theme::current(), marks), move |dep| {
                    let marks = &dep.5;
                    let mut rows = Column::new();
                    let mut next_mark = marks.iter().peekable();
                    for line in first..first + shown {
                        let color = if line == cursor { p().fg } else { p().line_no };
                        let mark = next_mark.next_if(|(l, _)| *l == line).map(|(_, current)| *current);
                        let (color, bg) = match mark {
                            Some(true) => (p().fg, Some(p().selection)),
                            Some(false) => (p().fg, Some(p().selection.scale_alpha(0.35))),
                            None => (color, None),
                        };
                        rows = rows.push(
                            container(
                                text((line + 1).to_string()).size(CODE_SIZE).font(theme::mono_font()).color(color),
                            )
                            .height(row_h)
                            .width(Length::Fill)
                            .align_x(Alignment::End)
                            .style(move |_| container::Style { background: bg.map(Into::into), ..Default::default() }),
                        );
                    }
                    Element::<'static, Msg>::from(iced::widget::pin(rows).y(-cut).width(Length::Fill))
                })
                .into()
            }
            None => space().into(),
        };
        container(numbers)
            .width(width)
            .height(Length::Fill)
            .padding(Padding { top: EDITOR_PADDING_TOP, right: 4.0, bottom: EDITOR_PADDING_TOP, left: 8.0 })
            .clip(true)
            .style(theme::bg(p().bg))
            .into()
    }

    fn view_diff<'a>(&'a self, d: &'a DiffTab) -> Element<'a, Msg> {
        let side_label: Cow<'_, str> = match &d.source {
            DiffSource::Index { staged: true } => "HEAD ⇄ Index".into(),
            DiffSource::Index { staged: false } => "Index ⇄ Working Tree".into(),
            DiffSource::Trees { label, .. } => format!("{label} · before ⇄ after").into(),
            DiffSource::Commit { details } => details.comparison.as_str().into(),
        };
        // The path and counts take what the buttons leave, clipped (full path in the tooltip).
        let mut info = row![
            text(d.rel.as_str()).size(UI).wrapping(text::Wrapping::None),
            text(side_label).size(SMALL).color(p().muted).wrapping(text::Wrapping::None),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if let Some(diff) = &d.diff {
            info = info
                .push(text(format!("+{}", diff.added)).size(12).color(p().added_text))
                .push(text(format!("−{}", diff.removed)).size(12).color(p().error_text));
        }
        let mut header = row![row_tip(container(info).width(Length::Fill).clip(true), d.rel.as_str())]
            .spacing(10)
            .align_y(Alignment::Center);
        // ‹ › step through the run's files, or through the diff's section of Changes.
        let files = match &d.source {
            DiffSource::Trees { run, .. } => self.runs.get(*run).and_then(|r| r.files.as_ref()),
            DiffSource::Commit { details } => Some(&details.files),
            DiffSource::Index { staged: true } => Some(&self.git.staged),
            DiffSource::Index { staged: false } => Some(&self.git.unstaged),
        };
        if let Some(files) = files
            && let Some(i) = files.iter().position(|c| c.path == d.rel)
            && (files.len() > 1 || matches!(d.source, DiffSource::Trees { .. }))
        {
            let n = files.len();
            header = header
                .push(tip(icon_button(Icon::ChevronLeft, Msg::ReviewStep(-1)), "Previous file"))
                .push(text(format!("{}/{n}", i + 1)).size(SMALL).color(p().muted))
                .push(tip(icon_button(Icon::ChevronRight, Msg::ReviewStep(1)), "Next file"));
        }
        header = header
            .push(icon_label(
                if d.full { Icon::Fold } else { Icon::Unfold },
                if d.full { "Hunks" } else { "Full file" },
                Some(Msg::DiffFull(!d.full)),
            ))
            .push(if d.on_disk {
                let label = if matches!(d.source, DiffSource::Commit { .. }) { "Working file" } else { "Open file" };
                icon_label(Icon::GoToFile, label, Some(Msg::DiffOpenFile))
            } else {
                tip(icon_label(Icon::GoToFile, "Open file", None), "File no longer exists")
            });
        if let DiffSource::Index { staged } = d.source {
            let msg = if staged { Msg::Unstage(vec![d.rel.clone()]) } else { Msg::Stage(vec![d.rel.clone()]) };
            let discardable = !staged && self.git.unstaged.iter().any(|c| c.path == d.rel && c.code != '!');
            if discardable {
                header = header.push(tip(
                    self.git_label(Icon::Discard, "Discard", Msg::Discard(vec![d.rel.clone()])),
                    "Throw away these changes (Undo available)",
                ));
            }
            header = header.push(
                button(
                    row![
                        self.action_icon(if staged { Icon::Remove } else { Icon::Add }, &msg, 13),
                        text(if staged { "Unstage" } else { "Stage" }).size(12)
                    ]
                    .spacing(5)
                    .align_y(Alignment::Center),
                )
                .padding([2, 8])
                .style(theme::primary)
                .on_press_maybe((!self.git_busy).then_some(msg)),
            );
        }

        let body: Element<'a, Msg> = match &d.diff {
            None => container(text("Loading…").color(p().muted)).padding(16).into(),
            Some(diff) if diff.error.is_some() => {
                container(text(diff.error.as_deref().unwrap_or_default()).color(p().error_text)).padding(16).into()
            }
            Some(diff) if diff.binary => container(text("Binary file").color(p().muted)).padding(16).into(),
            Some(diff) if diff.too_large => {
                container(text("File too large to diff (over 8 MB)").color(p().muted)).padding(16).into()
            }
            Some(diff) if diff.items.is_empty() => container(text("No changes").color(p().muted)).padding(16).into(),
            Some(diff) => {
                // Built once per diff, anchor and palette, not on every frame:
                // the terminal streams many messages per second while agents work.
                let (rows, anchor) = (Arc::clone(diff), d.anchor);
                let busy = self.git_busy;
                let hunk = match self.busy_op {
                    Some(BusyOp::Hunk(kind, id, i)) => Some((kind, id, i)),
                    _ => None,
                };
                let cached = lazy((diff.id, anchor, theme::current(), busy, hunk), move |_| {
                    diff_rows(&rows, anchor, busy, hunk)
                });
                // `lazy` hints Shrink: the widths are set here, or the rows get none.
                scrollable(container(cached).width(Length::Fill).padding(Padding { right: 12.0, ..Padding::ZERO }))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
        };

        let place = match d.anchor {
            Some(l) => format!("Comment on line {l}…"),
            None => "Comment on this file… (click a line to anchor)".to_string(),
        };
        let comment = row![
            text_input(&place, &self.comment)
                .on_input_maybe((!self.sending_comment).then_some(Msg::Comment))
                .on_submit(Msg::SendComment)
                .size(UI)
                .padding(6)
                .style(theme::input),
            button(
                row![
                    if self.sending_comment { self.spinner(13) } else { ico(Icon::Send, 13) },
                    text(if self.sending_comment { "Sending…" } else { "Send to agent" }).size(12)
                ]
                .spacing(5)
                .align_y(Alignment::Center)
            )
            .padding([5, 10])
            .style(theme::primary)
            .on_press_maybe((!self.sending_comment && !self.comment.trim().is_empty()).then_some(Msg::SendComment)),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        column![
            container(header).padding([6, 12]).style(theme::panel(p().bg)),
            hrule(),
            body,
            hrule(),
            container(comment).padding([6, 12]).style(theme::panel(p().sidebar)),
        ]
        .into()
    }

    fn view_status_bar(&self) -> Element<'_, Msg> {
        let mut left = row![].spacing(14).align_y(Alignment::Center);
        if !self.git.branch.is_empty() {
            left = left.push(tip(
                button(
                    row![
                        if matches!(self.busy_op, Some(BusyOp::Branches | BusyOp::SwitchBranch(_))) {
                            self.spinner(12)
                        } else {
                            ico(Icon::Branch, 12)
                        },
                        text(self.git.branch.as_str()).size(12)
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center),
                )
                .padding([1, 4])
                .style(theme::flat(false))
                .on_press_maybe((!self.git_busy).then_some(Msg::BranchesOpen)),
                "Switch branch…",
            ));
        }
        let working = self.agents.iter().filter(|a| a.agent_status == AgentStatus::Working).count();
        let blocked = self.agents.iter().filter(|a| a.agent_status == AgentStatus::Blocked).count();
        if working > 0 {
            left = left.push(
                row![
                    ico(Icon::DotFilled, 10).color(p().accent),
                    text(format!("{working} working")).size(12).color(p().accent)
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            );
        }
        if let Some(first) = self.agents.iter().find(|a| a.agent_status == AgentStatus::Blocked) {
            left = left.push(
                button(
                    row![
                        ico(Icon::Bell, 12).color(p().modified),
                        text(format!("{blocked} need you")).size(12).color(p().warn_text)
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center),
                )
                .padding([1, 4])
                .style(theme::flat(false))
                .on_press(Msg::FocusAgent(first.pane_id.clone())),
            );
        }
        let fresh = self.fresh_changes();
        if fresh > 0 {
            left = left.push(
                button(
                    row![
                        ico(Icon::DotFilled, 10).color(p().modified),
                        text(format!("{fresh} new {}", if fresh == 1 { "change" } else { "changes" })).size(12)
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center),
                )
                .padding([1, 4])
                .style(theme::flat(false))
                .on_press(Msg::InspectorTab(Inspector::Changes)),
            );
        }
        if let Some((msg, err)) = &self.status {
            // Click (or Escape) dismisses; a shortened error shows git's whole text on hover.
            let message = button(
                text(msg.as_str())
                    .size(12)
                    .color(if *err { p().error_text } else { p().muted })
                    .wrapping(text::Wrapping::None),
            )
            .padding([1, 4])
            .style(theme::flat(false))
            .on_press(Msg::DismissStatus);
            left = left.push(match &self.status_detail {
                Some(detail) => tooltip(
                    message,
                    container(text(detail.as_str()).size(12).font(theme::mono_font()))
                        .padding(6)
                        .max_width(640)
                        .style(theme::bordered(p().bg)),
                    tooltip::Position::Top,
                )
                .into(),
                None => tip(message, "Click to dismiss"),
            });
            if let Some((label, action)) = &self.status_action {
                left = left.push(
                    button(text(*label).size(12))
                        .padding([1, 8])
                        .style(theme::primary)
                        .on_press_maybe((!self.git_busy).then(|| action.clone())),
                );
            }
        }
        let mut right = row![].spacing(16).align_y(Alignment::Center);
        if let Some(Tab::File(t)) = self.active_tab.and_then(|i| self.tabs.get(i)) {
            let p = t.content.cursor().position;
            // iced's column is a byte offset: shown in characters, as editors do.
            let col = t.content.line(p.line).map_or(p.column, |l| {
                let mut end = p.column.min(l.text.len());
                while !l.text.is_char_boundary(end) {
                    end -= 1;
                }
                l.text[..end].chars().count()
            });
            right = right
                .push(text(format!("Ln {}, Col {}", p.line + 1, col + 1)).size(12))
                .push(text(t.syntax.as_str()).size(12));
        }
        if self.git_busy || self.manual_refresh || self.replacing {
            let label = self.busy_op.as_ref().map_or(
                if self.replacing {
                    "Replacing…"
                } else if self.manual_refresh {
                    "Refreshing changes…"
                } else {
                    "Working…"
                },
                BusyOp::label,
            );
            right = right.push(row![self.spinner(12), text(label).size(12).color(p().muted)].spacing(4));
        } else if self.search.running {
            right = right.push(text("Searching…").size(12).color(p().muted));
        }
        let theme_label: Cow<'_, str> = match self.theme {
            theme::Choice::Auto => format!("{} (auto)", p().name).into(),
            theme::Choice::Fixed(_) => p().name.into(),
        };
        right = right.push(tooltip(
            button(row![ico(Icon::ColorMode, 12), text(theme_label).size(12)].spacing(4).align_y(Alignment::Center))
                .padding([1, 4])
                .style(theme::flat(false))
                .on_press(Msg::CycleTheme),
            container(text("Switch theme (Ctrl+Shift+T)").size(12)).padding(6).style(theme::bordered(p().bg)),
            tooltip::Position::Top,
        ));
        right = right.push(tooltip(
            button(ico(Icon::Gear, 13))
                .padding([1, 4])
                .style(theme::flat(self.gear_menu))
                .on_press(Msg::GearMenu(!self.gear_menu)),
            container(text("Settings, shortcuts, about").size(12)).padding(6).style(theme::bordered(p().bg)),
            tooltip::Position::Top,
        ));
        // First run (nothing open, no agents): the shortcuts; later they live in tooltips.
        if self.tabs.is_empty() && self.agents.is_empty() && self.status.is_none() {
            right = right.push(
                text("Ctrl+Shift+ P open file · G changes · F search · E files · B panel · T theme · M focus")
                    .size(SMALL)
                    .color(p().muted)
                    .wrapping(text::Wrapping::None),
            );
        }
        // The left side gives way (clipped) so the right side always fits.
        container(row![container(left).width(Length::Fill).clip(true), right].spacing(16).align_y(Alignment::Center))
            .padding([2, 10])
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(p().bg.into()),
                text_color: Some(p().fg),
                border: iced::Border { color: p().border, width: 1.0, radius: 0.0.into() },
                ..Default::default()
            })
            .into()
    }
}

// ------------------------------------------------------------------ diff rows

/// The rows of a diff, owning their text (for `lazy`), at most [`diff::MAX_FULL_ROWS`].
fn diff_rows(
    diff: &diff::FileDiff,
    anchor: Option<usize>,
    busy: bool,
    hunk: Option<(HunkOp, u64, usize)>,
) -> Element<'static, Msg> {
    let note = |s: String| container(text(s).size(12).color(p().warn_text)).padding([4, 12]);
    let mut rows = Column::new().width(Length::Fill);
    if diff.full_capped {
        rows = rows.push(note(format!(
            "Over {} rows: showing the changes only. Open the file to see all of it.",
            diff::MAX_FULL_ROWS
        )));
    }
    let capped = diff.full_capped;
    // Each git hunk's toolbar goes above the first changed row inside it.
    let shown = &diff.items[..diff.items.len().min(diff::MAX_FULL_ROWS)];
    let bars = diff.patch.as_ref().map(|patch| {
        let changed = shown.iter().map(|item| match item {
            Item::Row { left, right } => {
                (left.number.filter(|_| left.kind == Kind::Delete), right.number.filter(|_| right.kind == Kind::Insert))
            }
            Item::Gap(_) => (None, None),
        });
        crate::hunks::place(changed, &patch.hunks)
    });
    for (r, item) in shown.iter().enumerate() {
        if let (Some(patch), Some(i)) = (&diff.patch, bars.as_ref().and_then(|b| b[r])) {
            rows = rows.push(hunk_bar(&patch.hunks[i].header, patch.staged, diff.id, i, busy, hunk));
        }
        rows = rows.push(match item {
            Item::Gap(n) => Element::from(
                button(
                    container(text(format!("⋯ {n} unchanged lines")).size(12).color(p().muted)).center_x(Length::Fill),
                )
                .width(Length::Fill)
                .padding(2)
                .style(|_, s| button::Style {
                    background: Some(
                        if matches!(s, button::Status::Hovered) { p().section } else { p().sidebar }.into(),
                    ),
                    text_color: p().muted,
                    ..Default::default()
                })
                // Already capped: asking for the full file again gives the same hunks.
                .on_press_maybe((!capped).then_some(Msg::DiffFull(true))),
            ),
            Item::Row { left, right } => diff_row(left, right, anchor),
        });
    }
    if let Some(more) = diff.items.len().checked_sub(diff::MAX_FULL_ROWS).filter(|n| *n > 0) {
        rows = rows.push(note(format!("{more} more rows not shown. Open the file to see the rest.")));
    }
    rows.into()
}

/// The bar above one git hunk: its `@@` line and what can be done with it.
fn hunk_bar(
    header: &str,
    staged: bool,
    diff: u64,
    i: usize,
    busy: bool,
    hunk: Option<(HunkOp, u64, usize)>,
) -> Element<'static, Msg> {
    let action = |icon, label: &'static str, kind| {
        let msg = Msg::Hunk(kind, diff, i);
        let working = hunk == Some((kind, diff, i));
        let icon: Element<'static, Msg> = if working { crate::spinner::view(13) } else { ico(icon, 13).into() };
        button(row![icon, text(label).size(12)].spacing(5).align_y(Alignment::Center))
            .padding([2, 8])
            .style(theme::flat(false))
            .on_press_maybe((!busy).then_some(msg))
    };
    let mut bar = row![
        container(
            text(header.to_string()).size(11).font(theme::mono_font()).color(p().muted).wrapping(text::Wrapping::None)
        )
        .width(Length::Fill)
        .clip(true),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    if staged {
        bar = bar.push(tip(action(Icon::Remove, "Unstage", HunkOp::Unstage), "Take this change out of the index"));
    } else {
        bar = bar
            .push(tip(action(Icon::Discard, "Discard", HunkOp::Discard), "Throw away this change (Undo available)"))
            .push(tip(action(Icon::Add, "Stage", HunkOp::Stage), "Stage only this change"));
    }
    container(bar)
        .padding(Padding { top: 1.0, right: 8.0, bottom: 1.0, left: 12.0 })
        .style(theme::panel(p().section))
        .into()
}

fn diff_bg(kind: Kind, anchored: bool) -> Element<'static, Msg> {
    let bg = match (kind, anchored) {
        (_, true) => Some(p().selection.scale_alpha(0.5)),
        (Kind::Insert, _) => Some(p().insert_line),
        (Kind::Delete, _) => Some(p().delete_line),
        (Kind::Empty, _) => Some(p().filler),
        (Kind::Equal, _) => None,
    };
    container(space())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style { background: bg.map(Into::into), ..Default::default() })
        .into()
}

/// One aligned row. Line text decides the height (wrapped lines grow the row);
/// backgrounds are a layer underneath that stretches to it.
fn diff_row(left: &diff::Side, right: &diff::Side, anchor: Option<usize>) -> Element<'static, Msg> {
    let line = right.number.or(left.number);
    let anchored = anchor.is_some() && line == anchor;
    let backgrounds = row![diff_bg(left.kind, anchored), vrule(), diff_bg(right.kind, anchored)];
    let content = row![diff_side(left), space().width(1), diff_side(right)];
    // The pointer's row is tinted by a button on top (it also takes the click):
    // its hover is drawn by the widget itself, so the cached rows need no messages.
    let hover = button(space().width(Length::Fill).height(Length::Fill))
        .padding(0)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_, s| button::Style {
            background: matches!(s, button::Status::Hovered | button::Status::Pressed)
                .then(|| p().fg.scale_alpha(0.06).into()),
            ..Default::default()
        })
        .on_press_maybe(line.map(Msg::DiffAnchor));
    Stack::new().width(Length::Fill).push(content).push_under(backgrounds).push(hover).into()
}

fn diff_side(side: &diff::Side) -> Element<'static, Msg> {
    let text_bg = match side.kind {
        Kind::Insert => p().insert_text,
        Kind::Delete => p().delete_text,
        _ => Color::TRANSPARENT,
    };
    let spans: Vec<Span<'static, (), Font>> = side
        .segments
        .iter()
        .map(|(emph, s)| {
            let sp = span(s.clone());
            if *emph && !s.trim().is_empty() { sp.background(text_bg) } else { sp }
        })
        .collect();
    let marker = match side.kind {
        Kind::Insert => "+",
        Kind::Delete => "−",
        _ => " ",
    };
    container(
        row![
            text(side.label.clone())
                .size(12)
                .font(theme::mono_font())
                .color(p().line_no)
                .width(44)
                .align_x(iced::alignment::Horizontal::Right),
            text(marker)
                .size(12)
                .font(theme::mono_font())
                .color(match side.kind {
                    Kind::Insert => p().added_text,
                    Kind::Delete => p().error_text,
                    _ => p().line_no,
                })
                .width(14)
                .align_x(iced::alignment::Horizontal::Center),
            rich_text(spans).size(13).font(theme::mono_font()).color(p().fg).width(Length::Fill),
        ]
        .spacing(4),
    )
    .width(Length::Fill)
    .padding([0, 4])
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_filenames_never_paint_over_the_status_column() {
        use iced::advanced::{Layout, Renderer as _, layout, renderer, renderer::Headless, widget::Tree};
        use iced::{Pixels, Size, mouse};
        use std::future::Future as _;

        let mut new =
            std::pin::pin!(<iced::Renderer as Headless>::new(theme::ui_font(), Pixels(14.0), Some("tiny-skia")));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(Some(mut renderer)) = new.as_mut().poll(&mut cx) else {
            panic!("headless renderer")
        };
        let mut render = |file: &Change, width: u32| {
            let size = Size::new(width as f32, 40.0);
            let viewport = iced::Rectangle::with_size(size);
            renderer.reset(viewport);
            let mut element: Element<'_, Msg> = container(history_file_label(file)).width(Length::Fill).into();
            let mut tree = Tree::new(&element);
            let node = element.as_widget_mut().layout(&mut tree, &renderer, &layout::Limits::new(Size::ZERO, size));
            element.as_widget().draw(
                &tree,
                &mut renderer,
                &theme::theme(),
                &renderer::Style { text_color: p().fg },
                Layout::new(&node),
                mouse::Cursor::Unavailable,
                &viewport,
            );
            renderer.screenshot(Size::new(width, 40), 1.0, p().sidebar)
        };
        for status in ['M', 'A', 'D'] {
            for width in [140_u32, 240, 400] {
                let short = render(&Change::new("a.txt", status), width);
                let long = render(&Change::new(format!("{}.txt", "very-long-file-name-".repeat(20)), status), width);
                // The final 24 pixels contain the gap and status. A long path
                // must render exactly the same pixels there as a short path.
                for y in 0..40 {
                    let start = ((y * width + width - 24) * 4) as usize;
                    let end = ((y + 1) * width * 4) as usize;
                    assert_eq!(
                        &long[start..end],
                        &short[start..end],
                        "path painted under {status} at width {width}, row {y}"
                    );
                }
            }
        }
    }

    #[test]
    fn history_clicks_expand_files_and_open_immutable_diffs() {
        use iced::advanced::Renderer as _;
        use iced::advanced::{Layout, Shell, clipboard, layout, renderer, renderer::Headless, widget::Tree};
        use iced::{Event, Pixels, Point, Size, mouse};
        use std::future::Future as _;

        // App::new creates a terminal and writes state. Run this UI test in a
        // separate process with isolated XDG roots and a shell that exits.
        let Ok(root) = std::env::var("MIRA_HISTORY_TEST_ROOT") else {
            let base = crate::files::test_dir("history-ui");
            let root = base.join("work");
            std::fs::create_dir_all(&root).unwrap();
            std::fs::create_dir_all(base.join("config/mira")).unwrap();
            std::fs::write(
                base.join("config/mira/config"),
                "terminal = shell\nautosave = off\ntheme = LAGC Tech Light\n",
            )
            .unwrap();
            crate::git::git(&root, &["init", "-q"]).unwrap();
            std::fs::write(root.join("sample.txt"), "before\n").unwrap();
            for message in ["Initial", "Edit sample"] {
                if message == "Edit sample" {
                    std::fs::write(root.join("sample.txt"), "after\n").unwrap();
                    std::fs::write(root.join("second.txt"), "second\n").unwrap();
                }
                crate::git::git(&root, &["add", "-A"]).unwrap();
                crate::git::git(
                    &root,
                    &[
                        "-c",
                        "user.name=Test",
                        "-c",
                        "user.email=test@example.invalid",
                        "-c",
                        "commit.gpgsign=false",
                        "commit",
                        "-qm",
                        message,
                    ],
                )
                .unwrap();
            }
            std::fs::write(root.join("sample.txt"), "local edits\n").unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "view::tests::history_clicks_expand_files_and_open_immutable_diffs", "--nocapture"])
                .env("MIRA_HISTORY_TEST_ROOT", &root)
                .env("XDG_CONFIG_HOME", base.join("config"))
                .env("XDG_STATE_HOME", base.join("state"))
                .env("SHELL", "/bin/true")
                .output()
                .unwrap();
            std::fs::remove_dir_all(base).unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        };
        let root = std::path::PathBuf::from(root);
        let (mut app, task) = App::new(crate::app::Start::Folder(root.clone()), true);
        drop(task);
        app.git = crate::git::snapshot(&root, &mut Default::default(), false).unwrap();
        let mut new =
            std::pin::pin!(<iced::Renderer as Headless>::new(theme::ui_font(), Pixels(14.0), Some("tiny-skia")));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(Some(mut renderer)) = new.as_mut().poll(&mut cx) else {
            panic!("headless renderer")
        };
        let click = |mut element: Element<'_, Msg>, renderer: &mut iced::Renderer, file: bool| {
            let mut tree = Tree::new(&element);
            let node = element.as_widget_mut().layout(
                &mut tree,
                renderer,
                &layout::Limits::new(Size::ZERO, Size::new(400.0, 600.0)),
            );
            let layout = Layout::new(&node);
            let target = if file {
                layout
                    .children()
                    .nth(1)
                    .unwrap()
                    .children()
                    .next()
                    .unwrap()
                    .children()
                    .nth(2)
                    .unwrap()
                    .bounds()
                    .center()
            } else {
                Point::new(100.0, 10.0)
            };
            let cursor = mouse::Cursor::Available(target);
            let mut messages = Vec::new();
            for event in [
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ] {
                let mut shell = Shell::new(&mut messages);
                element.as_widget_mut().update(
                    &mut tree,
                    &event,
                    layout,
                    cursor,
                    renderer,
                    &mut clipboard::Null,
                    &mut shell,
                    &layout.bounds(),
                );
            }
            messages
        };
        let mut messages = click(app.view_history().into(), &mut renderer, false);
        assert!(matches!(messages.first(), Some(Msg::HistorySelect(_))));
        drop(app.update(messages.remove(0)));
        let selected = app.commit_view.selected.as_ref().unwrap();
        let (id, hash) = (selected.id, Arc::clone(&selected.hash));
        let details = Arc::new(crate::git::commits::inspect(&root, &hash, "").unwrap());
        drop(app.update(Msg::HistoryLoaded(id, Ok(Arc::clone(&details)))));
        let mut messages = click(app.view_history().into(), &mut renderer, true);
        assert!(matches!(messages.first(), Some(Msg::HistoryOpen(_, 0))));
        drop(app.update(messages.remove(0)));
        let tab = app.active_diff().unwrap();
        assert!(matches!(&tab.source, DiffSource::Commit { details: source } if source.oid == details.oid));
        let (old, new) = crate::git::commits::contents(&root, &details, &tab.rel, 1024).unwrap().unwrap();
        assert_eq!((old.as_str(), new.as_str()), ("before\n", "after\n"));
        let load = tab.load;
        drop(app.update(Msg::DiffLoaded(load, true, Arc::new(diff::compute(&old, &new, Some(3))))));
        assert!(app.active_diff().unwrap().diff.as_ref().unwrap().patch.is_none());

        if let Ok(dir) = std::env::var("MIRA_HISTORY_PREVIEW_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            for (name, mut element, size) in [
                ("history.png", app.view_history().into(), Size::new(400, 400)),
                ("diff.png", app.view_diff(app.active_diff().unwrap()), Size::new(1100, 500)),
            ] {
                renderer.reset(iced::Rectangle::with_size(Size::new(size.width as f32, size.height as f32)));
                let mut tree = Tree::new(&element);
                let node = element.as_widget_mut().layout(
                    &mut tree,
                    &renderer,
                    &layout::Limits::new(Size::ZERO, Size::new(size.width as f32, size.height as f32)),
                );
                let layout = Layout::new(&node);
                let mut messages = Vec::new();
                element.as_widget_mut().update(
                    &mut tree,
                    &Event::Window(iced::window::Event::RedrawRequested(std::time::Instant::now())),
                    layout,
                    mouse::Cursor::Unavailable,
                    &renderer,
                    &mut clipboard::Null,
                    &mut Shell::new(&mut messages),
                    &layout.bounds(),
                );
                element.as_widget().draw(
                    &tree,
                    &mut renderer,
                    &theme::theme(),
                    &renderer::Style { text_color: p().fg },
                    layout,
                    mouse::Cursor::Unavailable,
                    &layout.bounds(),
                );
                let bytes = renderer.screenshot(size, 1.0, p().sidebar);
                image::save_buffer(dir.join(name), &bytes, size.width, size.height, image::ColorType::Rgba8).unwrap();
            }
        }
        drop(app.update(Msg::ReviewStep(1)));
        assert_eq!(app.active_diff().unwrap().rel, "second.txt");
        drop(app.update(Msg::HistorySelect(hash)));
        assert!(app.commit_view.selected.is_none());
        assert_eq!(std::fs::read_to_string(root.join("sample.txt")).unwrap(), "local edits\n");
    }

    #[test]
    fn long_branch_badges_wrap_inside_narrow_panels() {
        use iced::advanced::{layout, renderer::Headless, widget::Tree};
        use iced::{Pixels, Size};
        use std::future::Future as _;

        let mut new =
            std::pin::pin!(<iced::Renderer as Headless>::new(theme::ui_font(), Pixels(14.0), Some("tiny-skia")));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(Some(renderer)) = new.as_mut().poll(&mut cx) else { panic!("headless renderer") };
        for width in [140.0, 220.0, 320.0] {
            let mut element =
                badge("origin/feature/FRED-296-FRED-298-locations-frontend, feature/Freddys-Demo", p().accent);
            let mut tree = Tree::new(&element);
            let node = element.as_widget_mut().layout(
                &mut tree,
                &renderer,
                &layout::Limits::new(Size::ZERO, Size::new(width, 500.0)),
            );
            assert!(node.size().width <= width, "badge stays inside a {width}px panel");
            assert!(node.size().height > 28.0, "full branch names wrap instead of clipping at {width}px");
        }
    }

    #[test]
    fn activity_frames_keep_toolbar_buttons_in_place() {
        use iced::advanced::{layout, renderer::Headless, widget::Tree};
        use iced::{Pixels, Size};
        use std::future::Future as _;

        let mut new =
            std::pin::pin!(<iced::Renderer as Headless>::new(Font::MONOSPACE, Pixels(14.0), Some("tiny-skia")));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(Some(renderer)) = new.as_mut().poll(&mut cx) else { panic!("headless renderer") };
        let bounds = |active: Option<(usize, usize)>| {
            let mut toolbar = row![button(text("branch")).width(Length::Fill)];
            for (index, icon) in [Icon::Account, Icon::ArrowDown, Icon::ArrowUp, Icon::Refresh].into_iter().enumerate()
            {
                let glyph = match active {
                    Some((button, frame)) if button == index => spinner_frame(frame, 15),
                    _ => ico(icon, 15),
                };
                toolbar = toolbar.push(button(glyph).padding([3, 5]));
            }
            let mut element: Element<'_, Msg> = toolbar.spacing(4).into();
            let mut tree = Tree::new(&element);
            let node = element.as_widget_mut().layout(
                &mut tree,
                &renderer,
                &layout::Limits::new(Size::ZERO, Size::new(400.0, 40.0)),
            );
            node.children().iter().map(layout::Node::bounds).collect::<Vec<_>>()
        };
        let idle = bounds(None);
        for button in 0..4 {
            for frame in 0..SPINNER.len() {
                assert_eq!(bounds(Some((button, frame))), idle, "button {button}, spinner frame {frame}");
            }
        }
    }
}
