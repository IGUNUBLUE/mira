//! Layout: inspector, focus mode, the center split, dividers, window and theme.

use super::{App, AutoSave, Msg};
use crate::persist;
use crate::theme;

use iced::Task;
use iced::widget::pane_grid::Axis;

/// Draggable divider between the center and the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    /// Vertical: between the center and the inspector.
    Inspector,
    /// Horizontal: between Changes and History in the Changes panel.
    History,
}

pub(super) const MIN_HISTORY: f32 = 60.0;
/// Room the Changes list keeps above History.
const MIN_CHANGES_LIST: f32 = 120.0;

const MIN_SIDE: f32 = 220.0;
const MIN_CENTER: f32 = 360.0;
const DIVIDER: f32 = 5.0;

/// Widget id of the Search query input (focused by Ctrl+Shift+F).
pub const SEARCH_INPUT: &str = "search-query";
/// Widget id of the commit message box (focused by Ctrl+Shift+G).
pub const COMMIT_INPUT: &str = "commit-message";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inspector {
    /// Git changes, annotated with the agent runs that made them.
    Changes,
    Search,
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CenterPane {
    Document,
    Terminal,
}

impl App {
    /// The side panel's width as drawn: the saved width, narrowed when the
    /// window is too small for it and the center's minimum (it comes back when
    /// the window grows again).
    pub fn inspector_shown_width(&self) -> f32 {
        self.inspector_width.min((self.window_width - MIN_CENTER).max(MIN_SIDE))
    }

    pub(super) fn update_layout(&mut self, msg: Msg) -> Task<Msg> {
        let files_before = self.files_visible();
        let task = self.layout_msg(msg);
        // A name being typed goes with the Files tree it's typed in.
        if !self.files_visible() {
            self.name_edit = None;
        }
        if !files_before && self.files_visible() {
            // The tree and ignored files are only kept fresh while shown:
            // catch up now instead of at the next tick. A refresh in flight
            // without ignored files is superseded (its reply is dropped).
            self.explorer.refresh();
            self.refreshing = false;
            return Task::batch([task, self.refresh_git()]);
        }
        task
    }

    fn layout_msg(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Inspector(i) => {
                // Like VSCodium: the shortcut always lands in the panel (Ctrl+Shift+B hides it).
                self.inspector = i;
                self.show_inspector = true;
                self.focus_mode = false;
                self.sync_center();
                if i == Inspector::Search {
                    return iced::widget::operation::focus(SEARCH_INPUT)
                        .chain(iced::widget::operation::select_all(SEARCH_INPUT));
                }
                // Like VSCodium's Ctrl+Shift+G: straight to the commit message.
                if i == Inspector::Changes && self.git.root.is_some() {
                    return iced::widget::operation::focus(COMMIT_INPUT);
                }
            }
            Msg::DismissStatus => self.clear_status(),
            Msg::InspectorTab(i) => self.inspector = i,
            Msg::ToggleInspector => {
                self.show_inspector = !self.show_inspector;
                self.focus_mode = false;
                self.sync_center();
            }
            Msg::FocusMode => {
                self.focus_mode = !self.focus_mode;
                self.sync_center();
                return self.focus_terminal();
            }
            Msg::CenterResized(e) => {
                self.center.resize(e.split, e.ratio);
                self.doc_ratio = e.ratio;
            }
            Msg::WindowFocus(f) => {
                self.window_focused = f;
                if !f
                    && self.confirmation.is_none()
                    && self.remote_dialog.is_none()
                    && self.branch_picker.is_none()
                    && self.autosave != AutoSave::Off
                {
                    self.save_dirty_tabs();
                }
                // herdr enables focus reporting (mode 1004) and redraws on focus.
                if let Some(t) = &self.term {
                    t.focus_changed(f);
                }
                // Cheap and idempotent: also covers the herdr client just started.
                if f {
                    self.report_color_scheme();
                }
            }
            Msg::DividerHover(d) => self.hovered_divider = d,
            Msg::DividerPress(d) => {
                self.dragging = Some(d);
                self.drag_anchor = None;
            }
            Msg::DividerDrag(pos) => match self.dragging {
                Some(Divider::Inspector) => {
                    let max = (self.window_width - MIN_CENTER).max(MIN_SIDE);
                    self.inspector_width = (self.window_width - pos.x - DIVIDER / 2.0).clamp(MIN_SIDE, max);
                }
                // Relative to where the drag started: the panel's top edge isn't known here.
                Some(Divider::History) => {
                    let (start_y, start_h) = *self.drag_anchor.get_or_insert((pos.y, self.history_height));
                    let max = (self.window_height - 280.0 - MIN_CHANGES_LIST).max(MIN_HISTORY);
                    self.history_height = (start_h - (pos.y - start_y)).clamp(MIN_HISTORY, max);
                }
                None => {}
            },
            Msg::DividerRelease => {
                let saved = match self.dragging.take() {
                    Some(Divider::Inspector) => {
                        persist::config_set("inspector_width", &format!("{:.0}", self.inspector_width))
                    }
                    Some(Divider::History) => {
                        persist::config_set("history_height", &format!("{:.0}", self.history_height))
                    }
                    None => Ok(()),
                };
                if let Err(e) = saved {
                    self.set_status(format!("Could not save layout: {e}"), true);
                }
            }
            Msg::WindowSize(size) => {
                self.window_width = size.width;
                self.window_height = size.height;
            }
            Msg::CycleTheme => {
                self.theme = self.theme.next();
                self.apply_theme();
                let name = theme::p().name;
                let label = match self.theme {
                    theme::Choice::Auto => format!("Theme: auto (following HyDE · {name})"),
                    theme::Choice::Fixed(_) => format!("Theme: {name}"),
                };
                self.set_status(label, false);
                if let Err(e) = persist::config_set("theme", &self.theme.label()) {
                    self.set_status(format!("Could not save theme: {e}"), true);
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// Activate the palette of the current choice; recolor the terminal if it changed.
    pub(super) fn apply_theme(&mut self) {
        if theme::set_current(self.theme.resolve()) {
            if let Some(t) = &self.term {
                t.redraw();
            }
            self.report_color_scheme();
        }
    }

    /// Add or remove the document pane. The terminal pane is split or
    /// restored in place, never rebuilt, so its widget state (focus) survives.
    pub(super) fn sync_center(&mut self) {
        let want_doc = !self.tabs.is_empty() && !self.focus_mode;
        let find = |kind| self.center.iter().find(|(_, k)| **k == kind).map(|(p, _)| *p);
        match (want_doc, find(CenterPane::Document), find(CenterPane::Terminal)) {
            (true, None, Some(term)) => {
                if let Some((doc, split)) = self.center.split(Axis::Horizontal, term, CenterPane::Document) {
                    // The new pane lands below; documents go on top.
                    self.center.swap(doc, term);
                    self.center.resize(split, self.doc_ratio);
                }
            }
            (false, Some(doc), _) => {
                let _ = self.center.close(doc);
            }
            _ => {}
        }
    }
}
