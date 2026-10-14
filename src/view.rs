//! Views. Layout:
//!
//! ```text
//! ┌ sessions + turns ┬ document (optional) ───────┬ inspector ─────────┐
//! │                  ├ agent terminal ────────────┤ Review·Git·Env·…   │
//! └──────────────────┴────────────────────────────┴────────────────────┘
//! ```

use crate::app::*;
use crate::diff::{self, Item, Kind};
use crate::envfiles::KeyChange;
use crate::explorer;
use crate::git::Change;
use crate::search;
use crate::session::Status;
use crate::theme::{self, *};

use iced::keyboard::{Key, key::Named};
use iced::widget::pane_grid;
use iced::widget::text::Span;
use iced::widget::text_editor::{self, Binding};
use iced::widget::{
    Column, Stack, button, column, container, mouse_area, pane_grid as grid, rich_text, row,
    scrollable, space, span, text, text_input, tooltip,
};
use iced::{Alignment, Color, Element, Font, Length, Padding};

const UI: u32 = 14;
const SMALL: u32 = 12;
const MAX_RESULT_LINES: usize = 1_500;
const LEFT_WIDTH: f32 = 230.0;
const INSPECTOR_WIDTH: f32 = 380.0;

fn hrule<'a>() -> Element<'a, Msg> {
    container(space()).width(Length::Fill).height(1).style(theme::bg(p().border)).into()
}

fn vrule<'a>() -> Element<'a, Msg> {
    container(space()).width(1).height(Length::Fill).style(theme::bg(p().border)).into()
}

fn icon_button<'a>(label: &'a str, msg: Msg) -> Element<'a, Msg> {
    button(text(label).size(14).font(theme::mono_font())).padding([0, 5]).style(theme::flat(false)).on_press(msg).into()
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

fn status_style(s: Status, attention: bool) -> (&'static str, Color, &'static str) {
    match (s, attention) {
        (_, true) => ("◉", p().modified, "needs you"),
        (Status::Working, _) => ("●", p().accent, "working"),
        (Status::Waiting, _) => ("◐", p().modified, "waiting"),
        (Status::Idle, _) => ("○", p().muted, "idle"),
        (Status::Exited, _) => ("×", p().deleted, "exited"),
    }
}

fn ago(t: std::time::Instant) -> String {
    let s = t.elapsed().as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        _ => format!("{}h", s / 3600),
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

        let mut body = row![];
        if !self.focus_mode {
            body = body.push(self.view_sessions()).push(vrule());
        }
        body = body.push(container(center).style(theme::bg(p().border)).width(Length::Fill).height(Length::Fill));
        if self.show_inspector && !self.focus_mode {
            body = body.push(vrule()).push(self.view_inspector());
        }
        column![body.height(Length::Fill), self.view_status_bar()].into()
    }

    // ------------------------------------------------------------- sessions

    fn view_sessions(&self) -> Element<'_, Msg> {
        let mut list = Column::new().spacing(1);
        list = list.push(section(
            "SESSIONS".into(),
            vec![tip(icon_button("+", Msg::TogglePresets), "New session (Ctrl+Shift+N)")],
        ));

        if self.show_presets || self.sessions.is_empty() {
            let mut menu = Column::new().spacing(2).padding([6, 10]);
            for (name, bin) in &self.presets {
                menu = menu.push(
                    button(row![text(*name).size(UI), space::horizontal(), text(*bin).size(SMALL).color(p().muted)])
                        .width(Length::Fill)
                        .padding([3, 8])
                        .style(theme::flat(false))
                        .on_press(Msg::NewSession(Some(name))),
                );
            }
            menu = menu.push(
                button(text("shell").size(UI))
                    .width(Length::Fill)
                    .padding([3, 8])
                    .style(theme::flat(false))
                    .on_press(Msg::NewSession(None)),
            );
            if self.presets.is_empty() {
                menu = menu.push(text("No agent CLIs found on PATH").size(SMALL).color(p().muted));
            }
            list = list.push(container(menu).style(theme::panel(p().bg)));
        }

        for (i, s) in self.sessions.iter().enumerate() {
            let (dot, color, label) = status_style(s.status, s.attention);
            let changed = s.changed_files();
            let mut meta = row![text(label).size(SMALL).color(color)].spacing(8);
            if changed > 0 {
                meta = meta.push(text(format!("{changed} files")).size(SMALL).color(p().muted));
            }
            if !s.turns.is_empty() {
                meta = meta.push(text(format!("{} turns", s.turns.len())).size(SMALL).color(p().muted));
            }
            let item = row![
                text(dot).size(14).color(color).width(16),
                column![text(s.name.clone()).size(UI), meta].spacing(1),
                space::horizontal(),
                tip(
                    button(text(if self.confirm_close == Some(s.id) { "kill?" } else { "×" }).size(12))
                        .padding([0, 4])
                        .style(if self.confirm_close == Some(s.id) { theme::danger } else { theme::flat_fn })
                        .on_press(Msg::CloseSession(i)),
                    "Close session",
                ),
            ]
            .spacing(6)
            .align_y(Alignment::Center);
            list = list.push(
                button(item)
                    .width(Length::Fill)
                    .padding([5, 10])
                    .style(theme::flat(i == self.active))
                    .on_press(Msg::SelectSession(i)),
            );
        }

        if let Some(s) = self.session() {
            list = list.push(space::vertical().height(6)).push(section(format!("TURNS · {}", s.name), vec![]));
            if !self.has_repo() {
                list = list.push(
                    container(text("Turns need a git repository.").size(SMALL).color(p().muted)).padding([4, 10]),
                );
            }
            let all_active = self.review == ReviewTarget::All;
            list = list.push(
                button(row![text("All changes this session").size(UI), space::horizontal()])
                    .width(Length::Fill)
                    .padding([4, 10])
                    .style(theme::flat(all_active))
                    .on_press(Msg::Review(ReviewTarget::All)),
            );
            for (i, t) in s.turns.iter().enumerate().rev() {
                let state: Element<'_, Msg> = if t.running() {
                    text("running…").size(SMALL).color(p().accent).into()
                } else {
                    match &t.files {
                        Some(f) if f.is_empty() => text("no changes").size(SMALL).color(p().muted).into(),
                        Some(f) => text(format!("{} files", f.len())).size(SMALL).color(p().modified).into(),
                        None => text("…").size(SMALL).color(p().muted).into(),
                    }
                };
                let label: String = t.label.chars().take(80).collect();
                list = list.push(
                    button(
                        column![
                            text(label).size(UI).wrapping(text::Wrapping::WordOrGlyph),
                            row![text(format!("#{}", i + 1)).size(SMALL).color(p().muted), state, space::horizontal(), text(ago(t.started)).size(SMALL).color(p().muted)].spacing(8),
                        ]
                        .spacing(2),
                    )
                    .width(Length::Fill)
                    .padding([4, 10])
                    .style(theme::flat(self.review == ReviewTarget::Turn(i)))
                    .on_press(Msg::Review(ReviewTarget::Turn(i))),
                );
            }
        }

        container(scrollable(list).height(Length::Fill))
            .width(LEFT_WIDTH)
            .height(Length::Fill)
            .style(theme::panel(p().sidebar))
            .into()
    }

    fn view_terminal(&self) -> Element<'_, Msg> {
        let Some(s) = self.session() else {
            return container(
                button(text("+ New session").size(UI)).style(theme::primary).padding([4, 10]).on_press(Msg::TogglePresets),
            )
            .center(Length::Fill)
            .style(theme::panel(p().bg))
            .into();
        };
        let (dot, color, label) = status_style(s.status, s.attention);
        let header = row![
            text(dot).size(13).color(color),
            text(s.name.clone()).size(UI),
            text(label).size(SMALL).color(color),
            space::horizontal(),
            tip(small_button(if self.focus_mode { "Exit focus" } else { "Focus" }, Some(Msg::FocusMode)), "Ctrl+Shift+M"),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let header = if self.focus_mode {
            header
        } else {
            header.push(tip(
                small_button(if self.show_inspector { "Hide panel" } else { "Show panel" }, Some(Msg::ToggleInspector)),
                "Ctrl+B",
            ))
        };
        column![
            container(header).padding([4, 12]).width(Length::Fill).style(theme::panel(p().sidebar)),
            container(iced_term::TerminalView::show(&s.term).map(Msg::Term))
                .padding(Padding { left: 8.0, ..Padding::ZERO })
                .style(theme::panel(p().bg)),
        ]
        .into()
    }

    // ------------------------------------------------------------ inspector

    fn view_inspector(&self) -> Element<'_, Msg> {
        let review_badge = self.session().map(|s| s.changed_files()).filter(|n| *n > 0);
        let git_badge = Some(self.git.staged.len() + self.git.unstaged.len()).filter(|n| *n > 0);
        let env_badge = Some(
            self.env
                .files
                .iter()
                .filter(|f| f.risky() || !f.missing.is_empty() || !self.env_changes(f).is_empty())
                .count(),
        )
        .filter(|n| *n > 0);
        let tab = |label: &'static str, which: Inspector, count: Option<usize>| {
            let active = self.inspector == which;
            let mut r = row![text(label).size(UI)].spacing(4).align_y(Alignment::Center);
            if let Some(n) = count {
                r = r.push(text(n.to_string()).size(10).color(if which == Inspector::Env { p().modified } else { p().accent }));
            }
            button(r).padding([6, 9]).style(theme::tab(active)).on_press(Msg::InspectorTab(which))
        };
        let tabs = row![
            tab("Review", Inspector::Review, review_badge),
            tab("Git", Inspector::Git, git_badge),
            tab(".env", Inspector::Env, env_badge),
            tab("Search", Inspector::Search, None),
            tab("Files", Inspector::Files, None),
        ];
        let body = match self.inspector {
            Inspector::Review => self.view_review(),
            Inspector::Git => self.view_git(),
            Inspector::Env => self.view_env(),
            Inspector::Search => self.view_search(),
            Inspector::Files => self.view_explorer(),
        };
        container(column![container(tabs).width(Length::Fill).style(theme::panel(p().sidebar)), hrule(), body])
            .width(INSPECTOR_WIDTH)
            .height(Length::Fill)
            .style(theme::panel(p().sidebar))
            .into()
    }

    fn change_button<'a>(&self, c: &'a Change, active: bool, msg: Msg, action: Option<Element<'a, Msg>>) -> Element<'a, Msg> {
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

    fn view_review(&self) -> Element<'_, Msg> {
        let Some(s) = self.session() else {
            return container(text("Start a session to review agent changes.").size(UI).color(p().muted)).padding(12).into();
        };
        let (title, subtitle) = match self.review {
            ReviewTarget::All => ("All changes this session".to_string(), format!("since {} started", s.name)),
            ReviewTarget::Turn(t) => match s.turns.get(t) {
                Some(turn) => (turn.label.clone(), format!("Turn #{} · {} ago", t + 1, ago(turn.started))),
                None => (String::new(), String::new()),
            },
        };
        let running = matches!(self.review, ReviewTarget::Turn(t) if s.turns.get(t).is_some_and(|t| t.running()));

        let mut list = Column::new().spacing(1);
        let files = self.review_files();
        match files {
            _ if running => {
                list = list.push(container(text("The agent is still working on this turn…").size(UI).color(p().accent)).padding(10))
            }
            None => list = list.push(container(text("Computing changes…").size(UI).color(p().muted)).padding(10)),
            Some(f) if f.is_empty() => {
                list = list.push(container(text("No file changes.").size(UI).color(p().muted)).padding(10))
            }
            Some(f) => {
                let open_rel = match self.active_tab.and_then(|i| self.tabs.get(i)) {
                    Some(Tab::Diff(d)) if matches!(d.source, DiffSource::Trees { .. }) => Some(d.rel.as_str()),
                    _ => None,
                };
                for c in f {
                    list = list.push(self.change_button(c, open_rel == Some(c.path.as_str()), Msg::OpenTurnDiff(c.path.clone()), None));
                }
            }
        }

        let has_files = files.is_some_and(|f| !f.is_empty()) && !running;
        let confirming = self.confirm_revert.is_some();
        let actions = row![
            button(text(if confirming { "Confirm revert" } else { "Revert" }).size(12))
                .padding([3, 10])
                .style(if confirming { theme::danger } else { theme::flat_fn })
                .on_press_maybe(has_files.then_some(Msg::RevertReview)),
            button(text("Stage these").size(12))
                .padding([3, 10])
                .style(theme::flat(false))
                .on_press_maybe(has_files.then(|| Msg::Stage(files.map(|f| f.iter().map(|c| c.path.clone()).collect()).unwrap_or_default()))),
            space::horizontal(),
            button(text("Git ›").size(12)).padding([3, 10]).style(theme::primary).on_press(Msg::Inspector(Inspector::Git)),
        ]
        .spacing(6);

        column![
            container(
                column![
                    text(title).size(14),
                    text(subtitle).size(SMALL).color(p().muted),
                    actions,
                ]
                .spacing(6)
            )
            .padding(10),
            hrule(),
            scrollable(list).height(Length::Fill),
            container(
                text("Open a file, click a line in the diff, and comment — the note is typed into the agent's prompt.")
                    .size(SMALL)
                    .color(p().muted)
            )
            .padding(10),
        ]
        .into()
    }

    fn view_git(&self) -> Element<'_, Msg> {
        if self.git.root.is_none() {
            return container(text("This folder is not a git repository.").size(UI).color(p().muted)).padding(12).into();
        }
        let staged = &self.git.staged;
        let unstaged = &self.git.unstaged;
        let can_commit =
            !self.git_busy && !self.commit_msg.trim().is_empty() && (!staged.is_empty() || !unstaged.is_empty());
        let commit_label = if staged.is_empty() && !unstaged.is_empty() { "✓ Commit All" } else { "✓ Commit" };
        let open = match self.active_tab.and_then(|i| self.tabs.get(i)) {
            Some(Tab::Diff(DiffTab { rel, source: DiffSource::Index { staged }, .. })) => Some((rel.as_str(), *staged)),
            _ => None,
        };

        let mut list = Column::new().spacing(1);
        if !staged.is_empty() {
            let all: Vec<String> = staged.iter().map(|c| c.path.clone()).collect();
            list = list.push(section(format!("STAGED  {}", staged.len()), vec![icon_button("−", Msg::Unstage(all))]));
            for c in staged {
                list = list.push(self.change_button(
                    c,
                    open == Some((c.path.as_str(), true)),
                    Msg::OpenDiff(c.path.clone(), true),
                    Some(icon_button("−", Msg::Unstage(vec![c.path.clone()]))),
                ));
            }
        }
        let all: Vec<String> = unstaged.iter().map(|c| c.path.clone()).collect();
        list = list.push(section(
            format!("CHANGES  {}", unstaged.len()),
            if all.is_empty() { vec![] } else { vec![icon_button("+", Msg::Stage(all))] },
        ));
        for c in unstaged {
            list = list.push(self.change_button(
                c,
                open == Some((c.path.as_str(), false)),
                Msg::OpenDiff(c.path.clone(), false),
                Some(icon_button("+", Msg::Stage(vec![c.path.clone()]))),
            ));
        }
        if self.git.truncated {
            list = list.push(
                container(text(format!("Showing first {} changes", crate::git::MAX_CHANGES)).size(SMALL).color(p().modified))
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

        let mut branch = format!("⑂ {}", self.git.branch);
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
                        text(branch).size(UI),
                        space::horizontal(),
                        tip(icon_button("↓", Msg::Pull), "Pull (fast-forward)"),
                        tip(icon_button("↑", Msg::Push), "Push"),
                        tip(icon_button("⟳", Msg::Tick), "Refresh"),
                    ]
                    .align_y(Alignment::Center),
                    text_input("Commit message (Enter to commit)", &self.commit_msg)
                        .on_input(Msg::CommitInput)
                        .on_submit(Msg::Commit)
                        .size(UI)
                        .padding(6)
                        .style(theme::input),
                    button(container(text(commit_label).size(UI)).center_x(Length::Fill))
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

    fn view_env(&self) -> Element<'_, Msg> {
        let mut list = Column::new().spacing(1);
        if self.env.files.is_empty() {
            list = list.push(
                container(
                    text(if self.env.loading { "Looking for .env files…" } else { "No .env files in this folder." })
                        .size(UI)
                        .color(p().muted),
                )
                .padding(12),
            );
        }
        for f in &self.env.files {
            let changes = self.env_changes(f);
            let mut header = row![text(f.rel.clone()).size(UI).wrapping(text::Wrapping::None)]
                .spacing(6)
                .align_y(Alignment::Center);
            if f.example {
                header = header.push(badge("example", p().muted));
            }
            if f.tracked && !f.example {
                header = header.push(badge("tracked by git!", p().deleted));
            } else if f.risky() {
                header = header.push(badge("not ignored", p().modified));
            }
            if !changes.is_empty() {
                header = header.push(badge(format!("{} changed", changes.len()), p().modified));
            }
            header = header.push(space::horizontal());
            header = header.push(tip(icon_button("◌", Msg::EnvHideAll(f.path.clone())), "Hide all values"));
            header = header.push(tip(icon_button("+", Msg::EnvAdd(f.path.clone())), "Add key"));
            header = header.push(tip(icon_button("↗", Msg::OpenFile(f.path.clone(), None)), "Open in editor (shows values)"));
            list = list.push(container(header).padding([5, 10]).width(Length::Fill).style(theme::panel(p().section)));

            if !f.missing.is_empty() {
                list = list.push(
                    container(
                        row![
                            text(format!("Missing vs example: {}", f.missing.join(", ")))
                                .size(SMALL)
                                .color(p().deleted)
                                .width(Length::Fill),
                            small_button("Tell agent", Some(Msg::EnvSendMissing(f.path.clone()))),
                        ]
                        .align_y(Alignment::Center),
                    )
                    .padding([3, 10]),
                );
            }
            if !f.duplicates.is_empty() {
                list = list.push(
                    container(text(format!("Defined twice: {}", f.duplicates.join(", "))).size(SMALL).color(p().modified))
                        .padding([3, 10]),
                );
            }

            for e in &f.entries {
                let editing = matches!(&self.env.editing, Some((p, k, _)) if *p == f.path && *k == e.key);
                let revealed = self.env.revealed.contains(&(f.path.clone(), e.key.clone()));
                let change = changes.get(&e.key).copied();
                let key_color = match change {
                    Some(KeyChange::Added) => p().added,
                    Some(KeyChange::Changed) => p().modified,
                    _ => p().fg,
                };
                let value: Element<'_, Msg> = if editing {
                    let buf = self.env.editing.as_ref().map(|(_, _, b)| b.as_str()).unwrap_or("");
                    row![
                        text_input("value", buf)
                            .on_input(Msg::EnvEditInput)
                            .on_submit(Msg::EnvEditSave)
                            .secure(!revealed)
                            .size(12)
                            .font(theme::mono_font())
                            .padding(3)
                            .style(theme::input),
                        icon_button("✓", Msg::EnvEditSave),
                        icon_button("×", Msg::EnvEditCancel),
                    ]
                    .spacing(2)
                    .align_y(Alignment::Center)
                    .into()
                } else {
                    let shown = if revealed { e.value.clone() } else { crate::envfiles::mask(&e.value) };
                    row![
                        text(shown)
                            .size(12)
                            .font(theme::mono_font())
                            .color(if e.value.is_empty() { p().modified } else { p().muted })
                            .wrapping(text::Wrapping::None)
                            .width(Length::Fill),
                        tip(icon_button(if revealed { "◉" } else { "◎" }, Msg::EnvReveal(f.path.clone(), e.key.clone())), "Reveal"),
                        tip(icon_button("✎", Msg::EnvEdit(f.path.clone(), e.key.clone())), "Edit"),
                    ]
                    .spacing(2)
                    .align_y(Alignment::Center)
                    .into()
                };
                list = list.push(
                    container(
                        column![
                            row![
                                text(e.key.clone()).size(12).font(theme::mono_font()).color(key_color),
                                text(match change {
                                    Some(KeyChange::Added) => "new",
                                    Some(KeyChange::Changed) => "changed",
                                    _ => "",
                                })
                                .size(10)
                                .color(key_color),
                            ]
                            .spacing(6),
                            container(value).clip(true),
                        ]
                        .spacing(1),
                    )
                    .padding([3, 10]),
                );
            }
            for (k, c) in &changes {
                if *c == KeyChange::Removed {
                    list = list.push(
                        container(text(format!("{k}  removed")).size(12).font(theme::mono_font()).color(p().deleted)).padding([3, 10]),
                    );
                }
            }
            if let Some((p, buf)) = &self.env.adding
                && *p == f.path {
                    list = list.push(
                        container(
                            row![
                                text_input("NEW_KEY=value", buf)
                                    .on_input(Msg::EnvAddInput)
                                    .on_submit(Msg::EnvAddSave)
                                    .size(12)
                                    .font(theme::mono_font())
                                    .padding(3)
                                    .style(theme::input),
                                icon_button("✓", Msg::EnvAddSave),
                                icon_button("×", Msg::EnvEditCancel),
                            ]
                            .spacing(2)
                            .align_y(Alignment::Center),
                        )
                        .padding([3, 10]),
                    );
                }
            if !changes.is_empty() {
                list = list.push(
                    container(small_button("Mark as reviewed", Some(Msg::EnvMarkReviewed(f.path.clone())))).padding([2, 6]),
                );
            }
            list = list.push(space::vertical().height(6));
        }

        column![
            container(
                text("Values stay masked and are never sent to agents. Changes are tracked since you opened mira or last marked a file as reviewed.")
                    .size(SMALL)
                    .color(p().muted)
            )
            .padding(10),
            scrollable(list).height(Length::Fill),
        ]
        .into()
    }

    fn view_search(&self) -> Element<'_, Msg> {
        let s = &self.search;
        let toggle = |label: &'static str, on: bool, which: u8| {
            button(text(label).size(12).font(theme::mono_font())).padding([2, 5]).style(theme::flat(on)).on_press(Msg::SearchToggle(which))
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
                    list = list.push(container(text("Too many results to display").size(SMALL).color(p().muted)).padding([4, 10]));
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
                        tip(icon_button("›", Msg::SendToAgent(format!("{reference} "))), "Send reference to agent"),
                    ]
                    .align_y(Alignment::Center),
                );
            }
        }

        column![
            container(column![inputs, summary].spacing(8)).padding(10),
            scrollable(list).height(Length::Fill),
        ]
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
            let chevron = match (e.is_dir, self.explorer.expanded.contains(&e.path)) {
                (true, true) => "▾",
                (true, false) => "▸",
                _ => " ",
            };
            let open = matches!(self.active_tab.and_then(|i| self.tabs.get(i)), Some(Tab::File(t)) if t.path == e.path);
            let mut line = row![
                space::horizontal().width(e.depth as f32 * 12.0),
                text(chevron).size(UI).width(12).color(p().muted),
                text(e.name.clone()).size(UI).color(color),
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
            scrollable(tabs).direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(3).scroller_width(3))),
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
                    small_button("Send line to agent", Some(Msg::SendToAgent(format!("{rel_ref}:{line} ")))),
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
                .push(tip(icon_button("‹", Msg::ReviewStep(-1)), "Previous file"))
                .push(text(format!("{}/{n}", i + 1)).size(SMALL).color(p().muted))
                .push(tip(icon_button("›", Msg::ReviewStep(1)), "Next file"));
        }
        header = header
            .push(small_button(if d.full { "Hunks" } else { "Full file" }, Some(Msg::DiffFull(!d.full))))
            .push(small_button("Open file", path.is_file().then(|| Msg::OpenFile(path.clone(), d.anchor.map(|l| l.saturating_sub(1))))));
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
                            button(container(text(format!("⋯ {n} unchanged lines")).size(12).color(p().muted)).center_x(Length::Fill))
                                .width(Length::Fill)
                                .padding(2)
                                .style(|_, s| button::Style {
                                    background: Some(if matches!(s, button::Status::Hovered) { p().section } else { p().sidebar }.into()),
                                    text_color: p().muted,
                                    ..Default::default()
                                })
                                .on_press(Msg::DiffFull(true)),
                        ),
                        Item::Row { left, right } => diff_row(left, right, d.anchor),
                    });
                }
                scrollable(container(rows).padding(Padding { right: 12.0, ..Padding::ZERO })).height(Length::Fill).into()
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
            button(text("Send to agent").size(12))
                .padding([5, 10])
                .style(theme::primary)
                .on_press_maybe((!self.comment.trim().is_empty() && self.session().is_some()).then_some(Msg::SendComment)),
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
                button(text(format!("⑂ {}", self.git.branch)).size(12))
                    .padding([1, 4])
                    .style(theme::flat(false))
                    .on_press(Msg::Inspector(Inspector::Git)),
            );
        }
        let waiting = self.sessions.iter().filter(|s| s.attention || s.status == Status::Waiting).count();
        if waiting > 0 {
            left = left.push(text(format!("◐ {waiting} waiting")).size(12).color(p().modified));
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
            theme::Choice::Auto => format!("◐ {} (auto)", p().name),
            theme::Choice::Fixed(_) => format!("◐ {}", p().name),
        };
        right = right.push(tooltip(
            button(text(theme_label).size(12)).padding([1, 4]).style(theme::flat(false)).on_press(Msg::CycleTheme),
            container(text("Switch theme (Ctrl+Shift+T)").size(12)).padding(6).style(theme::bordered(p().bg)),
            tooltip::Position::Top,
        ));
        right = right.push(text("Ctrl+Shift+ N new · J/K switch · R review · G git · D env · T theme · M focus").size(SMALL).color(p().line_no));
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
        Some(l) => mouse_area(stacked).on_press(Msg::DiffAnchor(l)).interaction(iced::mouse::Interaction::Pointer).into(),
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
            text(marker).size(12).font(theme::mono_font()).color(p().line_no).width(14).align_x(iced::alignment::Horizontal::Center),
            rich_text(spans).size(13).font(theme::mono_font()).color(p().fg).width(Length::Fill),
        ]
        .spacing(4),
    )
    .width(Length::Fill)
    .padding([0, 4])
    .into()
}
