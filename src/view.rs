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
use crate::termio;
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

/// An icon glyph as text.
fn ico<'a>(i: Icon, size: u32) -> text::Text<'a> {
    let t = text(i.glyph()).size(size).line_height(1.0);
    match icons::font() {
        Some(f) => t.font(f),
        None => t.font(theme::mono_font()),
    }
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

fn badge<'a>(label: impl text::IntoFragment<'a>, color: Color) -> Element<'a, Msg> {
    container(text(label).size(10).color(p().bg).wrapping(text::Wrapping::None))
        .padding([0, 5])
        .style(move |_| container::Style {
            background: Some(color.into()),
            border: iced::Border::default().rounded(8),
            ..Default::default()
        })
        .into()
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
        AgentStatus::Idle => (p().line_no, "idle"),
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
        column![body.height(Length::Fill), self.view_status_bar()].into()
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
            Some(t) => container(termio::TermInput::new(
                iced_term::TerminalView::show(t).map(Msg::Term),
                herdr,
                self.term_cell,
                (self.focus_epoch, self.term_focus),
                Msg::TermInput,
            ))
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
        let fresh = self.runs.new_files() > 0;
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
            .width(self.inspector_width)
            .height(Length::Fill)
            .style(theme::panel(p().sidebar))
            .into()
    }

    /// One changed file. `origin`: show which agent run last changed it.
    fn change_button<'a>(
        &'a self,
        c: &'a Change,
        active: bool,
        msg: Msg,
        action: Option<Element<'a, Msg>>,
        origin: bool,
    ) -> Element<'a, Msg> {
        let (name, dir) = split_path(&c.path);
        let color = theme::status_color(c.code);
        let new = self.runs.is_new(&c.path);
        let mut content = row![
            if new { ico(Icon::DotFilled, 8).color(p().modified) } else { text("") }.width(10),
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
        ]
        .align_y(Alignment::Center)
        .spacing(4);
        if origin && let Some(run) = self.runs.origin(&c.path) {
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
        if let Some(a) = action {
            content = content.push(a);
        }
        content = content.push(text(c.code.to_string()).size(SMALL).color(color).width(12));
        button(content).width(Length::Fill).padding([2, 6]).style(theme::flat(active)).on_press(msg).into()
    }

    fn hint<'a>(&self, msg: &'a str) -> Element<'a, Msg> {
        container(text(msg).size(SMALL).color(p().muted)).padding([8, 10]).into()
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
                        row![ico(Icon::SourceControl, 14), text("Initialize repository").size(UI)]
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
        let when_idle = |m: Msg| (!busy).then_some(m);

        let mut list = Column::new().spacing(1);
        if !staged.is_empty() {
            let all: Vec<String> = staged.iter().map(|c| c.path.clone()).collect();
            list = list.push(section(
                format!("STAGED  {}", staged.len()),
                vec![tip(icon_button_maybe(Icon::Remove, when_idle(Msg::Unstage(all))), "Unstage all")],
            ));
            for c in staged {
                list = list.push(self.change_button(
                    c,
                    open == Some((c.path.as_str(), true)),
                    Msg::OpenDiff(c.path.clone(), true),
                    Some(tip(
                        icon_button_maybe(Icon::Remove, when_idle(Msg::Unstage(vec![c.path.clone()]))),
                        "Unstage changes",
                    )),
                    true,
                ));
            }
        }
        if !conflicts.is_empty() {
            list = list.push(section(format!("MERGE CHANGES  {}", conflicts.len()), vec![]));
            for c in &conflicts {
                list = list.push(self.change_button(
                    c,
                    open == Some((c.path.as_str(), false)),
                    Msg::OpenDiff(c.path.clone(), false),
                    Some(tip(
                        icon_button_maybe(Icon::Add, when_idle(Msg::Stage(vec![c.path.clone()]))),
                        "Mark as resolved (stage it once the conflict markers are gone)",
                    )),
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
                vec![tip(icon_button_maybe(Icon::Add, when_idle(Msg::Stage(all))), "Stage all changes")]
            },
        ));
        for c in &changes {
            list = list.push(self.change_button(
                c,
                open == Some((c.path.as_str(), false)),
                Msg::OpenDiff(c.path.clone(), false),
                Some(tip(icon_button_maybe(Icon::Add, when_idle(Msg::Stage(vec![c.path.clone()]))), "Stage changes")),
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

        let mut history = Column::new().spacing(1);
        for (i, c) in self.git.log.iter().enumerate() {
            let mut r = row![
                text(if i == 0 { "○" } else { "●" }).size(SMALL).color(p().accent),
                text(c.subject.as_str()).size(UI).wrapping(text::Wrapping::None),
            ]
            .spacing(6)
            .align_y(Alignment::Center);
            if !c.refs.is_empty() {
                r = r.push(badge(c.refs.as_str(), p().accent));
            }
            history = history.push(tooltip(
                container(r).padding([2, 10]).width(Length::Fill).clip(true),
                container(
                    column![text(c.subject.as_str()).size(12), text(c.detail.as_str()).size(12).color(p().muted),]
                        .spacing(2),
                )
                .padding(6)
                .max_width(420)
                .style(theme::bordered(p().bg)),
                tooltip::Position::Bottom,
            ));
        }
        if self.git.log.is_empty() {
            history = history.push(self.hint("No commits yet"));
        }

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
                        ico(Icon::Branch, 14).color(p().accent),
                        branch,
                        space::horizontal(),
                        self.chatgpt_account.as_ref().map(|email| if self.confirm_signout {
                            tip(
                                button(text("Sign out?").size(12))
                                    .padding([3, 6])
                                    .style(theme::danger)
                                    .on_press(Msg::ChatGptSignOut),
                                "Click again to sign out of ChatGPT · Esc cancels",
                            )
                        } else {
                            tip(
                                icon_button(Icon::Account, Msg::ChatGptSignOut),
                                if email.is_empty() {
                                    "Signed in to ChatGPT — click to sign out".to_string()
                                } else {
                                    format!("ChatGPT: {email} — click to sign out")
                                },
                            )
                        }),
                        tip(icon_button_maybe(Icon::ArrowDown, when_idle(Msg::Pull)), "Pull (fast-forward)"),
                        tip(icon_button_maybe(Icon::ArrowUp, when_idle(Msg::Push)), "Push"),
                        tip(icon_button_maybe(Icon::Refresh, when_idle(Msg::Tick)), "Refresh"),
                    ]
                    .align_y(Alignment::Center),
                    row![
                        // Grows with the message (wrapped to the panel width), then scrolls.
                        iced::widget::text_editor(&self.commit_msg)
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
                                    placeholder: c.line_no,
                                    value: c.fg,
                                    selection: c.selection,
                                }
                            })
                            .key_binding(|kp| match kp.key.as_ref() {
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
                                    text(SPINNER[self.spin])
                                        .size(16)
                                        .line_height(1.0)
                                        .font(icons::font().unwrap_or_else(theme::mono_font))
                                        .color(p().accent)
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
                                row![ico(Icon::Check, 14), text(commit_label).size(UI)]
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
            scrollable(if self.group_by_run { self.view_runs() } else { list }).height(Length::Fill),
            hdivider(
                Divider::History,
                self.dragging == Some(Divider::History) || self.hovered_divider == Some(Divider::History),
            ),
            section("HISTORY".into(), vec![]),
            scrollable(history).height(self.history_height),
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
        let fresh = self.runs.new_files();
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
                let confirming = self.confirm_revert == Some(r.id);
                let idle = !self.git_busy;
                header = header
                    .push(tip(
                        icon_button_maybe(Icon::Add, idle.then_some(Msg::StageRun(r.id))),
                        "Stage this run's files",
                    ))
                    .push(tip(
                        button(if confirming {
                            text(format!("Revert {n} {}?", if n == 1 { "file" } else { "files" })).size(12)
                        } else {
                            ico(Icon::Discard, 14)
                        })
                        .padding([3, 5])
                        .style(if confirming { theme::danger } else { theme::flat_fn })
                        .on_press_maybe(idle.then_some(Msg::RevertRun(r.id))),
                        if confirming {
                            "Click again to restore these files · Esc cancels"
                        } else {
                            "Revert this run"
                        },
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
        let toggle = |label: &'static str, on: bool, which: u8, name: &'static str| {
            tip(
                button(text(label).size(12).font(theme::mono_font()))
                    .padding([2, 5])
                    .style(theme::flat(on))
                    .on_press(Msg::SearchToggle(which)),
                name,
            )
        };
        let capped = if s.total >= search::MAX_MATCHES { "+" } else { "" };
        // Two steps, like Revert: files not open in a tab can't be undone.
        let replace: Element<'_, Msg> = if self.confirm_replace {
            tip(
                button(text(format!("Replace {}{capped} in {} files?", s.total, s.results.len())).size(12))
                    .padding([2, 8])
                    .style(theme::danger)
                    .on_press(Msg::ReplaceAll),
                "Click again to rewrite these files on disk · Esc cancels",
            )
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
                        text(f.lines.len()).size(SMALL).color(p().accent),
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
        let open = match self.active_tab.and_then(|i| self.tabs.get(i)) {
            Some(Tab::File(t)) => Some(t.path.as_path()),
            _ => None,
        };
        let mut list = Column::new();
        // Status and ignored flags are computed in update (see `Explorer::set_marks`).
        for e in &self.explorer.entries {
            let color = match (e.ignored, e.status) {
                (true, _) => p().line_no.scale_alpha(0.75),
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
            let kind: Element<'_, Msg> = match (e.is_dir, icons::font()) {
                (true, _) => {
                    ico(if e.expanded { Icon::FolderOpen } else { Icon::Folder }, 15).color(p().accent).width(18).into()
                }
                (false, Some(f)) => {
                    let (glyph, c) = icons::file_icon(&e.name);
                    let c = if e.ignored { c.scale_alpha(0.55) } else { c };
                    text(glyph).font(f).size(15).line_height(1.0).color(c).width(18).into()
                }
                (false, None) => space().width(0).into(),
            };
            let mut line = row![
                space::horizontal().width(e.depth as f32 * 12.0),
                chevron,
                kind,
                // Clipped, so a deep or long name never pushes the status letter out.
                container(text(e.name.as_str()).size(UI).color(color).wrapping(text::Wrapping::None))
                    .width(Length::Fill)
                    .clip(true),
            ]
            .align_y(Alignment::Center)
            .spacing(4);
            if let Some(code) = e.status.filter(|_| !e.is_dir) {
                line = line.push(text(code).size(SMALL).color(theme::status_color(code)));
            }
            list = list.push(row_tip(
                button(line)
                    .width(Length::Fill)
                    .padding([2, 8])
                    .style(theme::flat(open == Some(e.path.as_path())))
                    .on_press(Msg::ExplorerClick(e.path.clone(), e.is_dir)),
                e.name.as_str(),
            ));
        }
        if self.explorer.entries.is_empty() {
            list = list.push(self.hint("Empty folder"));
        }
        column![
            container(text(file_name(&self.root).to_uppercase()).size(SMALL).color(p().muted)).padding([6, 10]),
            scrollable(list).height(Length::Fill),
        ]
        .into()
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
                Tab::Diff(d) => {
                    let (name, _) = split_path(&d.rel);
                    let side = match &d.source {
                        DiffSource::Index { staged: true } => "Index".to_string(),
                        DiffSource::Index { staged: false } => "Working Tree".to_string(),
                        DiffSource::Trees { label, .. } => label.clone(),
                    };
                    format!("⇄ {name} ({side})")
                }
            };
            // A first Ctrl+W / × on unsaved edits arms this; the next one discards.
            let discarding = matches!(t, Tab::File(f) if self.pending_close.as_ref() == Some(&f.path));
            let close = if discarding {
                tip(
                    button(text("Discard?").size(12)).padding([3, 6]).style(theme::danger).on_press(Msg::CloseTab(i)),
                    "Click again to close without saving · Ctrl+S saves · Esc cancels",
                )
            } else {
                tip(
                    button(text("×").size(14)).padding([4, 6]).style(theme::tab(active)).on_press(Msg::CloseTab(i)),
                    "Close (Ctrl+W)",
                )
            };
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
        };
        container(column![tab_bar, body]).width(Length::Fill).height(Length::Fill).style(theme::panel(p().bg)).into()
    }

    fn view_editor<'a>(&'a self, t: &'a FileTab) -> Element<'a, Msg> {
        let editor = iced::widget::text_editor(&t.content)
            .on_action(Msg::Edit)
            .font(theme::mono_font())
            .size(CODE_SIZE)
            .height(Length::Fill)
            .padding(Padding { top: 6.0, right: 12.0, bottom: 6.0, left: 16.0 })
            .style(|_, _| text_editor::Style {
                background: p().bg.into(),
                border: iced::Border::default(),
                placeholder: p().line_no,
                value: p().fg,
                selection: p().selection,
            })
            .key_binding(|kp| {
                let ctrl = kp.modifiers.command();
                match kp.key.as_ref() {
                    Key::Character(c) if ctrl && c.eq_ignore_ascii_case("s") => Some(Binding::Custom(Msg::Save)),
                    Key::Character(c) if ctrl && c.eq_ignore_ascii_case("z") => {
                        Some(Binding::Custom(if kp.modifiers.shift() { Msg::Redo } else { Msg::Undo }))
                    }
                    Key::Character(c) if ctrl && c.eq_ignore_ascii_case("y") => Some(Binding::Custom(Msg::Redo)),
                    Key::Named(Named::Tab) if !ctrl && matches!(kp.status, text_editor::Status::Focused { .. }) => {
                        Some(Binding::Sequence(vec![Binding::Insert(' '); 4]))
                    }
                    _ => Binding::from_key_press(kp),
                }
            })
            .highlight(&t.syntax, p().syntax);
        column![
            container(
                row![
                    text(t.crumbs.as_str()).size(12).color(p().muted),
                    space::horizontal(),
                    icon_label(Icon::Send, "Send line to agent", Some(Msg::SendLine)),
                ]
                .align_y(Alignment::Center)
            )
            .padding([2, 12]),
            editor,
        ]
        .into()
    }

    fn view_diff<'a>(&'a self, d: &'a DiffTab) -> Element<'a, Msg> {
        let side_label: Cow<'_, str> = match &d.source {
            DiffSource::Index { staged: true } => "HEAD ⇄ Index".into(),
            DiffSource::Index { staged: false } => "Index ⇄ Working Tree".into(),
            DiffSource::Trees { label, .. } => format!("{label} · before ⇄ after").into(),
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
        if let DiffSource::Trees { run, .. } = d.source
            && let Some(files) = self.runs.get(run).and_then(|r| r.files.as_ref())
            && let Some(i) = files.iter().position(|c| c.path == d.rel)
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
                icon_label(Icon::GoToFile, "Open file", Some(Msg::DiffOpenFile))
            } else {
                tip(icon_label(Icon::GoToFile, "Open file", None), "File no longer exists")
            });
        if let DiffSource::Index { staged } = d.source {
            let msg = if staged { Msg::Unstage(vec![d.rel.clone()]) } else { Msg::Stage(vec![d.rel.clone()]) };
            header = header.push(
                button(text(if staged { "Unstage" } else { "Stage" }).size(12))
                    .padding([2, 8])
                    .style(theme::primary)
                    .on_press_maybe((!self.git_busy).then_some(msg)),
            );
        }

        let body: Element<'a, Msg> = match &d.diff {
            None => container(text("Loading…").color(p().muted)).padding(16).into(),
            Some(diff) if diff.binary => container(text("Binary file").color(p().muted)).padding(16).into(),
            Some(diff) if diff.too_large => {
                container(text("File too large to diff (over 8 MB)").color(p().muted)).padding(16).into()
            }
            Some(diff) if diff.items.is_empty() => container(text("No changes").color(p().muted)).padding(16).into(),
            Some(diff) => {
                // Built once per diff, anchor and palette, not on every frame:
                // the terminal streams many messages per second while agents work.
                let (rows, anchor) = (Arc::clone(diff), d.anchor);
                let cached = lazy((diff.id, anchor, theme::current()), move |_| diff_rows(&rows, anchor));
                scrollable(container(cached).padding(Padding { right: 12.0, ..Padding::ZERO }))
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
                .on_input(Msg::Comment)
                .on_submit(Msg::SendComment)
                .size(UI)
                .padding(6)
                .style(theme::input),
            button(row![ico(Icon::Send, 13), text("Send to agent").size(12)].spacing(5).align_y(Alignment::Center))
                .padding([5, 10])
                .style(theme::primary)
                .on_press_maybe((!self.comment.trim().is_empty()).then_some(Msg::SendComment)),
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
            left = left.push(
                button(
                    row![ico(Icon::Branch, 12), text(self.git.branch.as_str()).size(12)]
                        .spacing(4)
                        .align_y(Alignment::Center),
                )
                .padding([1, 4])
                .style(theme::flat(false))
                .on_press(Msg::InspectorTab(Inspector::Changes)),
            );
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
        let fresh = self.runs.new_files();
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
        }
        let mut right = row![].spacing(16).align_y(Alignment::Center);
        if let Some(Tab::File(t)) = self.active_tab.and_then(|i| self.tabs.get(i)) {
            let p = t.content.cursor().position;
            right = right
                .push(text(format!("Ln {}, Col {}", p.line + 1, p.column + 1)).size(12))
                .push(text(t.syntax.as_str()).size(12));
        }
        if self.git_busy || self.search.running {
            right = right.push(text("working…").size(12).color(p().muted));
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
        // First run (nothing open, no agents): the shortcuts; later they live in tooltips.
        if self.tabs.is_empty() && self.agents.is_empty() && self.status.is_none() {
            right = right.push(
                text("Ctrl+Shift+ G changes · F search · E files · B panel · T theme · M focus")
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
fn diff_rows(diff: &diff::FileDiff, anchor: Option<usize>) -> Element<'static, Msg> {
    let note = |s: String| container(text(s).size(12).color(p().warn_text)).padding([4, 12]);
    let mut rows = Column::new();
    if diff.full_capped {
        rows = rows.push(note(format!(
            "Over {} rows: showing the changes only. Open the file to see all of it.",
            diff::MAX_FULL_ROWS
        )));
    }
    let capped = diff.full_capped;
    for item in diff.items.iter().take(diff::MAX_FULL_ROWS) {
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
    let stacked = Stack::new().width(Length::Fill).push(content).push_under(backgrounds);
    match line {
        Some(l) => {
            mouse_area(stacked).on_press(Msg::DiffAnchor(l)).interaction(iced::mouse::Interaction::Pointer).into()
        }
        None => stacked.into(),
    }
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
