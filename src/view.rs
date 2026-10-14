//! Views. Layout:
//!
//! ```text
//! ┌ document (optional) ──────────┬ inspector ─────────────┐
//! ├ herdr (agents) ───────────────┤ Review·Git·Search·Files │
//! └───────────────────────────────┴─────────────────────────┘
//! ```

use crate::app::*;
use crate::diff::{self, Item, Kind};
use crate::explorer;
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
    Column, Stack, button, column, container, mouse_area, pane_grid as grid, rich_text, row, scrollable, space, span,
    text, text_input, tooltip,
};
use iced::{Alignment, Color, Element, Font, Length, Padding};

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
    button(ico(i, 15)).padding([3, 5]).style(theme::flat(false)).on_press(msg).into()
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

fn small_button<'a>(label: impl text::IntoFragment<'a>, msg: Option<Msg>) -> Element<'a, Msg> {
    button(text(label).size(12)).padding([2, 8]).style(theme::flat(false)).on_press_maybe(msg).into()
}

fn tip<'a>(content: impl Into<Element<'a, Msg>>, label: &'a str) -> Element<'a, Msg> {
    tooltip(
        content,
        container(text(label).size(12)).padding(6).style(theme::bordered(p().bg)),
        tooltip::Position::Bottom,
    )
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

fn agent_style(status: AgentStatus) -> (Icon, Color, &'static str) {
    match status {
        AgentStatus::Working => (Icon::DotFilled, p().accent, "working"),
        AgentStatus::Blocked => (Icon::Attention, p().modified, "needs you"),
        AgentStatus::Done => (Icon::DotOutline, p().added, "done"),
        AgentStatus::Idle => (Icon::DotOutline, p().muted, "idle"),
        AgentStatus::Unknown => (Icon::DotOutline, p().line_no, "…"),
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

fn duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
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
        if herdr {
            header = header.push(text("Ctrl+B prefix · Ctrl+B ? help").size(SMALL).color(p().line_no));
        }
        header = header.push(space::horizontal()).push(tip(
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
                self.focus_epoch,
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
        let review_badge = Some(self.runs.unreviewed()).filter(|n| *n > 0);
        let git_badge = Some(self.git.staged.len() + self.git.unstaged.len()).filter(|n| *n > 0);
        let tab = |i: Icon, label: &'static str, which: Inspector, count: Option<usize>| {
            let active = self.inspector == which;
            let color = if active { p().accent } else { p().muted };
            let mut r = row![ico(i, 14).color(color), text(label).size(UI)].spacing(6).align_y(Alignment::Center);
            if let Some(n) = count {
                r = r.push(text(n.to_string()).size(10).color(p().accent));
            }
            button(r).padding([6, 9]).style(theme::tab(active)).on_press(Msg::InspectorTab(which))
        };
        let tabs = row![
            tab(Icon::Diff, "Review", Inspector::Review, review_badge),
            tab(Icon::SourceControl, "Git", Inspector::Git, git_badge),
            tab(Icon::Search, "Search", Inspector::Search, None),
            tab(Icon::Files, "Files", Inspector::Files, None),
        ];
        let body = match self.inspector {
            Inspector::Review => self.view_review(),
            Inspector::Git => self.view_git(),
            Inspector::Search => self.view_search(),
            Inspector::Files => self.view_explorer(),
        };
        container(column![container(tabs).width(Length::Fill).style(theme::panel(p().sidebar)), hrule(), body])
            .width(self.inspector_width)
            .height(Length::Fill)
            .style(theme::panel(p().sidebar))
            .into()
    }

    fn change_button<'a>(
        &self,
        c: &'a Change,
        active: bool,
        msg: Msg,
        action: Option<Element<'a, Msg>>,
    ) -> Element<'a, Msg> {
        let (name, dir) = split_path(&c.path);
        let color = theme::status_color(c.code);
        let mut content = row![
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
        if let Some(a) = action {
            content = content.push(a);
        }
        content = content.push(text(c.code.to_string()).size(SMALL).color(color).width(12));
        button(content).width(Length::Fill).padding([2, 10]).style(theme::flat(active)).on_press(msg).into()
    }

    fn hint<'a>(&self, msg: &'a str) -> Element<'a, Msg> {
        container(text(msg).size(SMALL).color(p().muted)).padding([8, 10]).into()
    }

    fn view_review(&self) -> Element<'_, Msg> {
        let mut top = Column::new().spacing(1);

        // Agents working in this folder right now.
        let unreviewed = self.runs.unreviewed();
        let mut actions: Vec<Element<'_, Msg>> = Vec::new();
        if unreviewed > 0 {
            actions.push(tip(icon_button(Icon::Check, Msg::MarkAllReviewed), "Mark all reviewed"));
        }
        top = top.push(section(format!("AGENTS IN {}", file_name(&self.root).to_uppercase()), vec![]));
        if self.herdr_error.is_some() || self.term_kind == TermKind::Shell {
            let msg = match &self.herdr_error {
                Some(e) => e.as_str(),
                None => "Agent tracking needs herdr. mira is running a plain shell.",
            };
            top = top.push(self.hint(msg));
        } else if self.agents.is_empty() {
            top =
                top.push(self.hint("No agent here yet. Start one in herdr (codex, kiro-cli, claude…) in this folder."));
        }
        for a in &self.agents {
            let (dot, color, label) = agent_style(a.agent_status);
            top = top.push(
                container(
                    row![
                        ico(dot, 13).color(color).width(16),
                        text(a.label()).size(UI),
                        text(label).size(SMALL).color(color),
                        space::horizontal(),
                        text(a.pane_id.to_string()).size(SMALL).color(p().line_no),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .padding([3, 10]),
            );
        }

        // Runs, newest first.
        top = top.push(space::vertical().height(4)).push(section(
            if unreviewed > 0 { format!("RUNS · {unreviewed} TO REVIEW") } else { "RUNS".into() },
            actions,
        ));
        if !self.has_repo() {
            top = top.push(self.hint("Runs need a git repository to show what changed."));
        } else if self.runs.runs.is_empty() {
            top = top.push(self.hint("Each time an agent here finishes working, its changes show up here."));
        }
        let mut list = Column::new().spacing(1);
        for r in self.runs.runs.iter().rev() {
            let selected = self.selected_run == Some(r.id);
            let (state, state_color) = match (&r.files, r.running()) {
                (_, true) => ("working…".to_string(), p().accent),
                (None, false) => ("…".to_string(), p().muted),
                (Some(f), false) if f.is_empty() => ("no changes".to_string(), p().muted),
                (Some(f), false) => (format!("{} files", f.len()), if r.reviewed { p().muted } else { p().modified }),
            };
            let pending = !r.running() && !r.reviewed && r.files.as_ref().is_some_and(|f| !f.is_empty());
            let dot = if r.running() {
                ico(Icon::DotFilled, 10).color(p().accent)
            } else if pending {
                ico(Icon::DotFilled, 10).color(p().modified)
            } else {
                ico(Icon::DotOutline, 10).color(p().line_no)
            };
            let title = r.prompt.clone().unwrap_or_else(|| format!("{} run", r.agent));
            let title_color = if r.reviewed { p().muted } else { p().fg };
            list = list.push(
                button(
                    row![
                        dot.width(14),
                        column![
                            text(title).size(UI).color(title_color).wrapping(text::Wrapping::WordOrGlyph),
                            row![
                                text(r.agent.clone()).size(SMALL).color(p().accent),
                                text(state).size(SMALL).color(state_color),
                                space::horizontal(),
                                text(ago(r.finished.unwrap_or(r.started))).size(SMALL).color(p().line_no),
                            ]
                            .spacing(8),
                        ]
                        .spacing(2),
                    ]
                    .spacing(4),
                )
                .width(Length::Fill)
                .padding([5, 10])
                .style(theme::flat(selected))
                .on_press(Msg::SelectRun(r.id)),
            );
        }

        column![
            scrollable(top).height(Length::Shrink),
            scrollable(list).height(Length::FillPortion(2)),
            hrule(),
            container(self.view_run_detail()).height(Length::FillPortion(3)),
        ]
        .into()
    }

    fn view_run_detail(&self) -> Element<'_, Msg> {
        let Some(run) = self.selected_run.and_then(|id| self.runs.get(id)) else {
            return self.hint("Select a run to see what it changed. Click a line in the diff to comment; the note goes to that agent.");
        };
        let took = run.finished.map(|f| duration(f.saturating_sub(run.started)));
        let subtitle = match took {
            Some(t) => format!("{} · {} ago · took {t}", run.agent, ago(run.started)),
            None => format!("{} · started {} ago · working", run.agent, ago(run.started)),
        };
        let files = self.review_files();
        let has_files = files.is_some_and(|f| !f.is_empty()) && !run.running();
        let confirming = self.confirm_revert == Some(run.id);
        let actions = row![
            button(
                row![ico(Icon::Discard, 13), text(if confirming { "Confirm revert" } else { "Revert" }).size(12)]
                    .spacing(5)
                    .align_y(Alignment::Center)
            )
            .padding([3, 10])
            .style(if confirming { theme::danger } else { theme::flat_fn })
            .on_press_maybe(has_files.then_some(Msg::RevertRun)),
            button(row![ico(Icon::Add, 13), text("Stage").size(12)].spacing(5).align_y(Alignment::Center))
                .padding([3, 10])
                .style(theme::flat(false))
                .on_press_maybe(has_files.then_some(Msg::StageRun)),
            space::horizontal(),
            button(
                row![ico(Icon::Check, 13), text(if run.reviewed { "Reviewed" } else { "Mark reviewed" }).size(12)]
                    .spacing(5)
                    .align_y(Alignment::Center)
            )
            .padding([3, 10])
            .style(if run.reviewed { theme::flat_fn } else { theme::primary })
            .on_press_maybe((!run.running()).then_some(Msg::MarkReviewed(run.id))),
        ]
        .spacing(6);

        let mut list = Column::new().spacing(1);
        match files {
            _ if run.running() => list = list.push(self.hint("The agent is still working…")),
            None if run.trees().is_none() && !self.has_repo() => {}
            None => list = list.push(self.hint("Computing changes…")),
            Some(f) if f.is_empty() => list = list.push(self.hint("No file changes.")),
            Some(f) => {
                let open_rel = match self.active_tab.and_then(|i| self.tabs.get(i)) {
                    Some(Tab::Diff(d)) if matches!(d.source, DiffSource::Trees { .. }) => Some(d.rel.as_str()),
                    _ => None,
                };
                for c in f {
                    list = list.push(self.change_button(
                        c,
                        open_rel == Some(c.path.as_str()),
                        Msg::OpenRunDiff(c.path.clone()),
                        None,
                    ));
                }
            }
        }
        column![
            container(
                column![
                    text(run.prompt.clone().unwrap_or_else(|| format!("{} run", run.agent)))
                        .size(14)
                        .wrapping(text::Wrapping::WordOrGlyph),
                    text(subtitle).size(SMALL).color(p().muted),
                    actions,
                ]
                .spacing(6)
            )
            .padding(10),
            scrollable(list).height(Length::Fill),
        ]
        .into()
    }

    fn view_git(&self) -> Element<'_, Msg> {
        if self.git.root.is_none() {
            return container(text("This folder is not a git repository.").size(UI).color(p().muted))
                .padding(12)
                .into();
        }
        let staged = &self.git.staged;
        let unstaged = &self.git.unstaged;
        let has_changes = !staged.is_empty() || !unstaged.is_empty();
        let can_commit = !self.git_busy && !self.commit_msg.text().trim().is_empty() && has_changes;
        let generating = self.commit_gen.is_some();
        let can_generate = generating || has_changes;
        let commit_label = if staged.is_empty() && !unstaged.is_empty() { "Commit All" } else { "Commit" };
        let open = match self.active_tab.and_then(|i| self.tabs.get(i)) {
            Some(Tab::Diff(DiffTab { rel, source: DiffSource::Index { staged }, .. })) => Some((rel.as_str(), *staged)),
            _ => None,
        };

        let mut list = Column::new().spacing(1);
        if !staged.is_empty() {
            let all: Vec<String> = staged.iter().map(|c| c.path.clone()).collect();
            list = list
                .push(section(format!("STAGED  {}", staged.len()), vec![icon_button(Icon::Remove, Msg::Unstage(all))]));
            for c in staged {
                list = list.push(self.change_button(
                    c,
                    open == Some((c.path.as_str(), true)),
                    Msg::OpenDiff(c.path.clone(), true),
                    Some(icon_button(Icon::Remove, Msg::Unstage(vec![c.path.clone()]))),
                ));
            }
        }
        let all: Vec<String> = unstaged.iter().map(|c| c.path.clone()).collect();
        list = list.push(section(
            format!("CHANGES  {}", unstaged.len()),
            if all.is_empty() { vec![] } else { vec![icon_button(Icon::Add, Msg::Stage(all))] },
        ));
        for c in unstaged {
            list = list.push(self.change_button(
                c,
                open == Some((c.path.as_str(), false)),
                Msg::OpenDiff(c.path.clone(), false),
                Some(icon_button(Icon::Add, Msg::Stage(vec![c.path.clone()]))),
            ));
        }
        if self.git.truncated {
            list = list.push(
                container(
                    text(format!("Showing first {} changes", crate::git::MAX_CHANGES)).size(SMALL).color(p().modified),
                )
                .padding([4, 10]),
            );
        }

        list = list.push(space::vertical().height(8)).push(section("HISTORY".into(), vec![]));
        for (i, c) in self.git.log.iter().enumerate() {
            let mut r = row![
                text(if i == 0 { "○" } else { "●" }).size(SMALL).color(p().accent),
                text(c.subject.clone()).size(UI).wrapping(text::Wrapping::None),
            ]
            .spacing(6)
            .align_y(Alignment::Center);
            if !c.refs.is_empty() {
                r = r.push(badge(c.refs.replace("HEAD -> ", ""), p().accent));
            }
            list = list.push(tooltip(
                container(r).padding([2, 10]).width(Length::Fill).clip(true),
                container(text(format!("{}  {} · {}", c.hash, c.author, c.when)).size(12))
                    .padding(6)
                    .style(theme::bordered(p().bg)),
                tooltip::Position::Bottom,
            ));
        }

        let mut branch = self.git.branch.clone();
        if self.git.ahead > 0 {
            branch.push_str(&format!("  ↑{}", self.git.ahead));
        }
        if self.git.behind > 0 {
            branch.push_str(&format!("  ↓{}", self.git.behind));
        }
        column![
            container(
                column![
                    row![
                        ico(Icon::Branch, 14).color(p().accent),
                        text(branch).size(UI),
                        space::horizontal(),
                        tip(icon_button(Icon::ArrowDown, Msg::Pull), "Pull (fast-forward)"),
                        tip(icon_button(Icon::ArrowUp, Msg::Push), "Push"),
                        tip(icon_button(Icon::Refresh, Msg::Tick), "Refresh"),
                    ]
                    .align_y(Alignment::Center),
                    iced::widget::text_editor(&self.commit_msg)
                        .placeholder("Commit message (Ctrl+Enter to commit)")
                        .on_action(Msg::CommitEdit)
                        .size(UI)
                        .padding(6)
                        .height(Length::Shrink)
                        .max_height(180.0)
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
                            Key::Named(Named::Enter) if kp.modifiers.command() => Some(Binding::Custom(Msg::Commit)),
                            _ => Binding::from_key_press(kp),
                        }),
                    row![
                        button(
                            container(
                                row![
                                    ico(if generating { Icon::Close } else { Icon::Sparkle }, 13).color(p().accent),
                                    text(if generating { "Generating… (cancel)" } else { "Generate message" }).size(12),
                                ]
                                .spacing(6)
                                .align_y(Alignment::Center),
                            )
                            .center_x(Length::Fill)
                        )
                        .width(Length::Fill)
                        .padding(4)
                        .style(theme::flat(generating))
                        .on_press_maybe(can_generate.then_some(Msg::GenerateCommit)),
                    ],
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
                ]
                .spacing(6)
            )
            .padding(10),
            scrollable(list).height(Length::Fill),
        ]
        .into()
    }

    fn view_search(&self) -> Element<'_, Msg> {
        let s = &self.search;
        let toggle = |label: &'static str, on: bool, which: u8| {
            button(text(label).size(12).font(theme::mono_font()))
                .padding([2, 5])
                .style(theme::flat(on))
                .on_press(Msg::SearchToggle(which))
        };
        let inputs = column![
            row![
                text_input("Search", &s.query)
                    .on_input(Msg::SearchQuery)
                    .on_submit(Msg::SearchRun)
                    .size(UI)
                    .padding(6)
                    .style(theme::input),
                toggle("Aa", s.opts.case_sensitive, 0),
                toggle("ab", s.opts.whole_word, 1),
                toggle(".*", s.opts.regex, 2),
            ]
            .spacing(2)
            .align_y(Alignment::Center),
            row![
                text_input("Replace", &s.replace)
                    .on_input(Msg::SearchReplace)
                    .on_submit(Msg::ReplaceAll)
                    .size(UI)
                    .padding(6)
                    .style(theme::input),
                small_button("Replace all", (!s.results.is_empty()).then_some(Msg::ReplaceAll)),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        ]
        .spacing(6);

        let summary: Element<'_, Msg> = if let Some(e) = &s.error {
            text(e.clone()).size(12).color(p().deleted).into()
        } else if s.query.is_empty() {
            space().into()
        } else if s.running && s.results.is_empty() {
            text("Searching…").size(12).color(p().muted).into()
        } else {
            let capped = if s.total >= search::MAX_MATCHES { "+" } else { "" };
            text(format!("{}{capped} results in {} files", s.total, s.results.len())).size(12).color(p().muted).into()
        };

        let mut list = Column::new();
        let mut shown = 0;
        'files: for f in &s.results {
            let (name, dir) = split_path(&f.rel);
            list = list.push(
                button(
                    row![
                        text(name).size(UI),
                        text(dir).size(SMALL).color(p().muted).wrapping(text::Wrapping::None),
                        space::horizontal(),
                        text(f.lines.len().to_string()).size(SMALL).color(p().accent),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([3, 10])
                .style(theme::flat(false))
                .on_press(Msg::OpenFile(f.path.clone(), None)),
            );
            for m in &f.lines {
                if shown >= MAX_RESULT_LINES {
                    list = list.push(
                        container(text("Too many results to display").size(SMALL).color(p().muted)).padding([4, 10]),
                    );
                    break 'files;
                }
                shown += 1;
                let (preview, ranges) = search::preview(m);
                let mut spans: Vec<Span<'static, (), Font>> = Vec::new();
                let mut at = 0;
                for (a, b) in ranges {
                    if a > at {
                        spans.push(span(preview[at..a].to_string()));
                    }
                    spans.push(span(preview[a..b].to_string()).background(p().selection).color(p().fg));
                    at = b;
                }
                if at < preview.len() {
                    spans.push(span(preview[at..].to_string()));
                }
                let reference = format!("{}:{}", f.rel, m.line + 1);
                list = list.push(
                    row![
                        button(rich_text(spans).size(12).wrapping(text::Wrapping::None))
                            .width(Length::Fill)
                            .padding(Padding { top: 1.0, right: 4.0, bottom: 1.0, left: 26.0 })
                            .style(theme::flat(false))
                            .on_press(Msg::OpenFile(f.path.clone(), Some(m.line))),
                        tip(
                            icon_button(Icon::Send, Msg::SendToAgent(format!("{reference} "))),
                            "Send reference to agent"
                        ),
                    ]
                    .align_y(Alignment::Center),
                );
            }
        }

        column![container(column![inputs, summary].spacing(8)).padding(10), scrollable(list).height(Length::Fill),]
            .into()
    }

    fn view_explorer(&self) -> Element<'_, Msg> {
        let decorations = self.git.decorations();
        let mut list = Column::new();
        for e in &self.explorer.entries {
            let repo_rel = format!("{}{}", self.repo_prefix, e.rel);
            let ignored = explorer::is_ignored(&repo_rel, &self.git.ignored);
            let status = if e.is_dir {
                explorer::dir_status(&repo_rel, &decorations)
            } else {
                decorations.get(&repo_rel).copied()
            };
            let color = match (ignored, status) {
                (true, _) => p().line_no.scale_alpha(0.75),
                (_, Some(c)) => theme::status_color(c),
                _ => p().fg,
            };
            let expanded = self.explorer.expanded.contains(&e.path);
            let chevron: Element<'_, Msg> = if e.is_dir {
                ico(if expanded { Icon::ChevronDown } else { Icon::ChevronRight }, 13).color(p().muted).width(14).into()
            } else {
                space().width(14).into()
            };
            let kind: Element<'_, Msg> = match (e.is_dir, icons::font()) {
                (true, _) => {
                    ico(if expanded { Icon::FolderOpen } else { Icon::Folder }, 15).color(p().accent).width(18).into()
                }
                (false, Some(f)) => {
                    let (glyph, c) = icons::file_icon(&e.name);
                    let c = if ignored { c.scale_alpha(0.55) } else { c };
                    text(glyph.to_string()).font(f).size(15).line_height(1.0).color(c).width(18).into()
                }
                (false, None) => space().width(0).into(),
            };
            let open = matches!(self.active_tab.and_then(|i| self.tabs.get(i)), Some(Tab::File(t)) if t.path == e.path);
            let mut line = row![
                space::horizontal().width(e.depth as f32 * 12.0),
                chevron,
                kind,
                text(e.name.clone()).size(UI).color(color).wrapping(text::Wrapping::None),
                space::horizontal(),
            ]
            .align_y(Alignment::Center)
            .spacing(4);
            if let Some(code) = status.filter(|_| !e.is_dir) {
                line = line.push(text(code.to_string()).size(SMALL).color(theme::status_color(code)));
            }
            list = list.push(
                button(line)
                    .width(Length::Fill)
                    .padding([2, 8])
                    .style(theme::flat(open))
                    .on_press(Msg::ExplorerClick(e.path.clone(), e.is_dir)),
            );
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
            tabs = tabs.push(
                container(
                    row![
                        button(text(title).size(UI).wrapping(text::Wrapping::None))
                            .padding(Padding { top: 6.0, right: 4.0, bottom: 6.0, left: 12.0 })
                            .style(theme::tab(active))
                            .on_press(Msg::SelectTab(i)),
                        button(text("×").size(14)).padding([4, 6]).style(theme::tab(active)).on_press(Msg::CloseTab(i)),
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
                }),
            );
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
        let rel = t.path.strip_prefix(&self.root).unwrap_or(&t.path).to_string_lossy().replace('/', " › ");
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
                    Key::Named(Named::Tab) if !ctrl && matches!(kp.status, text_editor::Status::Focused { .. }) => {
                        Some(Binding::Sequence(vec![Binding::Insert(' '); 4]))
                    }
                    _ => Binding::from_key_press(kp),
                }
            })
            .highlight(&t.syntax, p().syntax);
        let rel_ref = t.path.strip_prefix(&self.root).unwrap_or(&t.path).to_string_lossy().into_owned();
        let line = t.content.cursor().position.line + 1;
        column![
            container(
                row![
                    text(rel).size(12).color(p().muted),
                    space::horizontal(),
                    icon_label(Icon::Send, "Send line to agent", Some(Msg::SendToAgent(format!("{rel_ref}:{line} ")))),
                ]
                .align_y(Alignment::Center)
            )
            .padding([2, 12]),
            editor,
        ]
        .into()
    }

    fn view_diff<'a>(&'a self, d: &'a DiffTab) -> Element<'a, Msg> {
        let root = self.repo_root();
        let side_label = match &d.source {
            DiffSource::Index { staged: true } => "HEAD ⇄ Index".to_string(),
            DiffSource::Index { staged: false } => "Index ⇄ Working Tree".to_string(),
            DiffSource::Trees { label, .. } => format!("{label} · before ⇄ after"),
        };
        let mut header = row![text(d.rel.clone()).size(UI), text(side_label).size(SMALL).color(p().muted)]
            .spacing(10)
            .align_y(Alignment::Center);
        if let Some(diff) = &d.diff {
            header = header
                .push(text(format!("+{}", diff.added)).size(12).color(p().added))
                .push(text(format!("−{}", diff.removed)).size(12).color(p().deleted));
        }
        let path = root.join(&d.rel);
        header = header.push(space::horizontal());
        if matches!(d.source, DiffSource::Trees { .. })
            && let Some((i, n)) = self.review_position(&d.rel)
        {
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
            .push(icon_label(
                Icon::GoToFile,
                "Open file",
                path.is_file().then(|| Msg::OpenFile(path.clone(), d.anchor.map(|l| l.saturating_sub(1)))),
            ));
        if let DiffSource::Index { staged } = d.source {
            header = header.push(
                button(text(if staged { "Unstage" } else { "Stage" }).size(12))
                    .padding([2, 8])
                    .style(theme::primary)
                    .on_press(if staged { Msg::Unstage(vec![d.rel.clone()]) } else { Msg::Stage(vec![d.rel.clone()]) }),
            );
        }

        let body: Element<'a, Msg> = match &d.diff {
            None => container(text("Loading…").color(p().muted)).padding(16).into(),
            Some(diff) if diff.binary => container(text("Binary file").color(p().muted)).padding(16).into(),
            Some(diff) if diff.items.is_empty() => container(text("No changes").color(p().muted)).padding(16).into(),
            Some(diff) => {
                let mut rows = Column::new();
                for item in &diff.items {
                    rows = rows.push(match item {
                        Item::Gap(n) => Element::from(
                            button(
                                container(text(format!("⋯ {n} unchanged lines")).size(12).color(p().muted))
                                    .center_x(Length::Fill),
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
                            .on_press(Msg::DiffFull(true)),
                        ),
                        Item::Row { left, right } => diff_row(left, right, d.anchor),
                    });
                }
                scrollable(container(rows).padding(Padding { right: 12.0, ..Padding::ZERO }))
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
                    row![ico(Icon::Branch, 12), text(self.git.branch.clone()).size(12)]
                        .spacing(4)
                        .align_y(Alignment::Center),
                )
                .padding([1, 4])
                .style(theme::flat(false))
                .on_press(Msg::Inspector(Inspector::Git)),
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
        if blocked > 0 {
            left = left.push(
                row![
                    ico(Icon::Bell, 12).color(p().modified),
                    text(format!("{blocked} need you")).size(12).color(p().modified)
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            );
        }
        let unreviewed = self.runs.unreviewed();
        if unreviewed > 0 {
            left = left.push(
                button(
                    row![ico(Icon::Diff, 12).color(p().modified), text(format!("{unreviewed} to review")).size(12)]
                        .spacing(4)
                        .align_y(Alignment::Center),
                )
                .padding([1, 4])
                .style(theme::flat(false))
                .on_press(Msg::Inspector(Inspector::Review)),
            );
        }
        if let Some((msg, err)) = &self.status {
            left = left.push(text(msg.clone()).size(12).color(if *err { p().deleted } else { p().muted }));
        }
        let mut right = row![].spacing(16).align_y(Alignment::Center);
        if let Some(Tab::File(t)) = self.active_tab.and_then(|i| self.tabs.get(i)) {
            let p = t.content.cursor().position;
            right = right
                .push(text(format!("Ln {}, Col {}", p.line + 1, p.column + 1)).size(12))
                .push(text(t.syntax.clone()).size(12));
        }
        if self.git_busy || self.search.running {
            right = right.push(text("working…").size(12).color(p().muted));
        }
        let theme_label = match self.theme {
            theme::Choice::Auto => format!("{} (auto)", p().name),
            theme::Choice::Fixed(_) => p().name.to_string(),
        };
        right = right.push(tooltip(
            button(row![ico(Icon::ColorMode, 12), text(theme_label).size(12)].spacing(4).align_y(Alignment::Center))
                .padding([1, 4])
                .style(theme::flat(false))
                .on_press(Msg::CycleTheme),
            container(text("Switch theme (Ctrl+Shift+T)").size(12)).padding(6).style(theme::bordered(p().bg)),
            tooltip::Position::Top,
        ));
        right = right.push(
            text("Ctrl+Shift+ R review · G git · F search · E files · B panel · T theme · M focus")
                .size(SMALL)
                .color(p().line_no),
        );
        container(row![left, space::horizontal(), right].align_y(Alignment::Center))
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
fn diff_row<'a>(left: &'a diff::Side, right: &'a diff::Side, anchor: Option<usize>) -> Element<'a, Msg> {
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

fn diff_side(side: &diff::Side) -> Element<'_, Msg> {
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
            text(side.number.map(|n| n.to_string()).unwrap_or_default())
                .size(12)
                .font(theme::mono_font())
                .color(p().line_no)
                .width(44)
                .align_x(iced::alignment::Horizontal::Right),
            text(marker)
                .size(12)
                .font(theme::mono_font())
                .color(p().line_no)
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
