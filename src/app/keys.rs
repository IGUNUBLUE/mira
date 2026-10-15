//! Global keyboard shortcuts.

use super::{App, Inspector, Msg, Tab};

use iced::{Task, keyboard};

impl App {
    pub(super) fn on_key(&mut self, event: keyboard::Event, captured: bool) -> Task<Msg> {
        let keyboard::Event::KeyPressed { key, modifiers, physical_key, .. } = event else {
            return Task::none();
        };
        // Escape that no widget took: the terminal (iced_term writes ESC and
        // captures it) and a focused text input (it unfocuses and captures)
        // keep theirs. The editors unfocus without capturing, so it lands here.
        if key == keyboard::Key::Named(keyboard::key::Named::Escape) && !captured && modifiers.is_empty() {
            return self.escape();
        }
        if !modifiers.command() {
            return Task::none();
        }
        let shift = modifiers.shift();
        let latin = key.to_latin(physical_key).map(|c| c.to_ascii_lowercase());
        let backquote = matches!(physical_key, keyboard::key::Physical::Code(keyboard::key::Code::Backquote));
        // While a widget (terminal, editor, input) owns the key, only
        // Ctrl+Shift+… and Ctrl+` act globally.
        if captured && !shift && !backquote {
            return Task::none();
        }
        match (latin, shift) {
            (Some('s'), false) => {
                if let Some(i) = self.active_tab {
                    self.save_tab(i, true);
                }
            }
            // Editor not focused (e.g. after clicking a tab): undo/redo the active file.
            (Some('z'), false) => return self.update(Msg::Undo),
            (Some('z'), true) | (Some('y'), false) if !captured => return self.update(Msg::Redo),
            (Some('w'), false) => {
                if let Some(i) = self.active_tab {
                    self.close_tab(i);
                }
            }
            // Plain Ctrl+B is herdr's prefix.
            (Some('b'), true) => return self.update(Msg::ToggleInspector),
            (Some('g'), true) => return self.update(Msg::Inspector(Inspector::Changes)),
            (Some('f'), true) => return self.update(Msg::Inspector(Inspector::Search)),
            (Some('e'), true) => return self.update(Msg::Inspector(Inspector::Files)),
            (Some('t'), true) => return self.update(Msg::CycleTheme),
            (Some('m'), true) => return self.update(Msg::FocusMode),
            _ if backquote => return self.focus_terminal(),
            _ => {}
        }
        Task::none()
    }

    /// Escape cancels the most pressing pending thing, one per press: the
    /// ChatGPT sign-in dialog, a
    /// confirmation (revert, replace, sign out, discard tab), the diff's
    /// anchored line, its typed comment, then the status message. It never
    /// stops a commit-message generation: Escape also reaches mira from the
    /// file editor, and ✦ already stops it.
    fn escape(&mut self) -> Task<Msg> {
        if self.signin.is_some() {
            return self.update(Msg::SignInCancel);
        }
        if self.clear_confirms() {
            self.status = None;
            self.status_action = None;
            self.status_detail = None;
            return Task::none();
        }
        if let Some(Tab::Diff(d)) = self.active_tab.and_then(|i| self.tabs.get_mut(i))
            && d.anchor.take().is_some()
        {
            return Task::none();
        }
        if self.active_diff().is_some() && !self.comment.is_empty() {
            self.comment.clear();
            return Task::none();
        }
        self.clear_status();
        Task::none()
    }
}
