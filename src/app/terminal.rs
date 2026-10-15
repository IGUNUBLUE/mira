//! The center terminal (herdr or a shell). Every command goes through `termio::Gate`.

use super::{App, CODE_SIZE, Msg, blocking};
use crate::herdr;
use crate::termio;
use crate::theme;

use alacritty_terminal::index::{Column, Line, Point as GridPoint};
use alacritty_terminal::vte::ansi::Rgb;
use iced::{Task, keyboard};
use iced_term::{AlacrittyEvent, BackendCommand};
use std::collections::HashMap;
use std::time::Instant;

/// What runs in the center terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKind {
    Herdr,
    /// herdr is not installed (or `terminal = "shell"` in the config).
    Shell,
}

fn swallow(
    c: &str,
    m: keyboard::Modifiers,
) -> (iced_term::bindings::Binding<iced_term::bindings::InputKind>, iced_term::bindings::BindingAction) {
    (
        iced_term::bindings::Binding {
            target: iced_term::bindings::InputKind::Char(c.to_string()),
            modifiers: m,
            terminal_mode_include: iced_term::TermMode::empty(),
            terminal_mode_exclude: iced_term::TermMode::empty(),
        },
        iced_term::bindings::BindingAction::Esc(String::new()),
    )
}

/// mira's `Ctrl+Shift+<letter>` shortcuts; swallowed by the terminal so they
/// work while it has focus. Never `c`/`v` (copy/paste).
const SHORTCUT_LETTERS: [&str; 6] = ["g", "f", "e", "m", "b", "t"];

impl App {
    pub(super) fn update_terminal(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Term(iced_term::Event::BackendCall(_, cmd)) => return self.on_term_call(cmd),
            Msg::TermInput(input) => return self.on_term_input(input),
            Msg::TermFlush => {
                self.flush_scheduled = false;
                if self.gate.flush_due(Instant::now()) {
                    let cmds = self.gate.flush();
                    self.term_apply(cmds);
                }
                return self.schedule_term_flush();
            }
            Msg::RestartTerminal => return self.spawn_terminal(),
            _ => {}
        }
        Task::none()
    }

    pub(super) fn spawn_terminal(&mut self) -> Task<Msg> {
        let (program, args) = match self.term_kind {
            TermKind::Herdr => herdr::launch_command(self.herdr_session.as_deref()),
            TermKind::Shell => (std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()), Vec::new()),
        };
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("TERM_PROGRAM".to_string(), "mira".to_string());
        let settings = iced_term::settings::Settings {
            backend: iced_term::settings::BackendSettings {
                program,
                args,
                env,
                working_directory: Some(self.root.clone()),
            },
            theme: iced_term::settings::ThemeSettings::new(Box::new(theme::terminal_palette())),
            font: iced_term::settings::FontSettings {
                size: CODE_SIZE as f32,
                font_type: theme::mono_font(),
                ..Default::default()
            },
        };
        let id = self.next_term_id;
        self.next_term_id += 1;
        let mut term = match iced_term::Terminal::new(id, settings) {
            Ok(t) => t,
            Err(e) => {
                self.set_status(format!("terminal: {e}"), true);
                return Task::none();
            }
        };
        let cs = keyboard::Modifiers::CTRL | keyboard::Modifiers::SHIFT;
        let mut bindings: Vec<_> = SHORTCUT_LETTERS.iter().map(|c| swallow(c, cs)).collect();
        bindings.push(swallow("`", keyboard::Modifiers::CTRL));
        term.handle(iced_term::Command::AddBindings(bindings));
        self.term = Some(term);
        self.gate = termio::Gate::default();
        self.term_cell = None;
        self.workspace = None;
        self.focus_terminal()
    }

    pub(super) fn focus_terminal(&mut self) -> Task<Msg> {
        self.focus_epoch += 1;
        self.term_focus = true;
        self.term.as_ref().map_or_else(Task::none, |t| iced_term::TerminalView::focus(t.widget_id().clone()))
    }

    pub(super) fn term_write(&mut self, bytes: Vec<u8>) {
        self.term_send(iced_term::Command::ProxyToBackend(BackendCommand::Write(bytes)));
    }

    /// Every command for the terminal goes through the gate (see `termio::Gate`).
    pub(super) fn term_send(&mut self, cmd: iced_term::Command) {
        if let termio::Gated::Forward(cmds) = self.gate.command(cmd) {
            self.term_apply(cmds);
        }
    }

    fn term_apply(&mut self, cmds: Vec<iced_term::Command>) {
        let Some(t) = &mut self.term else { return };
        let mut shutdown = false;
        for cmd in cmds {
            shutdown |= t.handle(cmd) == iced_term::actions::Action::Shutdown;
        }
        if shutdown {
            self.term = None;
            self.gate = termio::Gate::default();
            let what = if self.term_kind == TermKind::Herdr { "herdr detached" } else { "The shell exited" };
            self.set_status(format!("{what} — agents keep running in herdr"), false);
        }
    }

    /// Make sure a queued batch gets flushed even if it never sees `Wakeup`.
    fn schedule_term_flush(&mut self) -> Task<Msg> {
        if !self.gate.busy() || self.flush_scheduled {
            return Task::none();
        }
        self.flush_scheduled = true;
        Task::perform(blocking(|| std::thread::sleep(termio::GATE_QUIET)), |()| Msg::TermFlush)
    }

    fn on_term_call(&mut self, cmd: BackendCommand) -> Task<Msg> {
        match cmd {
            BackendCommand::ProcessAlacrittyEvent(event) => {
                // OSC 52 store from apps in the terminal. Loads are not answered:
                // letting programs read the clipboard is a data leak.
                if let AlacrittyEvent::ClipboardStore(_, text) = &event {
                    return iced::clipboard::write(text.clone());
                }
                let color = |i| {
                    let (r, g, b) = termio::color_reply_rgb(i);
                    Rgb { r, g, b }
                };
                match self.gate.event(event, Instant::now(), color) {
                    termio::Gated::Forward(cmds) => self.term_apply(cmds),
                    termio::Gated::Queued => return self.schedule_term_flush(),
                }
            }
            cmd => {
                // The backend truncates the cell size to whole pixels; match it.
                if let BackendCommand::Resize(_, Some(cell)) = &cmd {
                    self.term_cell = Some(iced::Size::new(cell.width.floor().max(1.0), cell.height.floor().max(1.0)));
                }
                self.term_send(iced_term::Command::ProxyToBackend(cmd));
                return self.schedule_term_flush();
            }
        }
        Task::none()
    }

    fn on_term_input(&mut self, input: termio::Input) -> Task<Msg> {
        match input {
            termio::Input::Mouse { button, pressed, line, col, mods } => {
                let button = match button {
                    termio::Button::Middle => iced_term::MouseButton::MiddleButton,
                    termio::Button::Right => iced_term::MouseButton::RightButton,
                    termio::Button::WheelUp => iced_term::MouseButton::ScrollUp,
                    termio::Button::WheelDown => iced_term::MouseButton::ScrollDown,
                };
                let point = GridPoint::new(Line(i32::try_from(line).unwrap_or(i32::MAX)), Column(col));
                self.term_send(iced_term::Command::ProxyToBackend(BackendCommand::MouseReport(
                    button, mods, point, pressed,
                )));
            }
            termio::Input::Bytes(b) => self.term_write(b),
            termio::Input::Paste(text) => self.term_write(termio::bracketed(&text)),
        }
        self.schedule_term_flush()
    }
}
