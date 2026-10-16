//! The center terminal (herdr or a shell), drawn by mira's own widget (`term`).

use super::{App, Msg, blocking};
use crate::herdr;
use crate::term;
use crate::termio;
use crate::theme;

use iced::Task;
use std::collections::HashMap;
use std::time::Duration;

/// What runs in the center terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKind {
    Herdr,
    /// herdr is not installed (or `terminal = "shell"` in the config).
    Shell,
}

/// Open `url` (a link Ctrl+clicked in the terminal) in the desktop's browser.
/// `file://` links never go to xdg-open: terminal output is untrusted, and a
/// `.desktop` file or a script there would be launched (see [`file_link`]).
pub(super) fn open_url(url: &str) -> Result<(), String> {
    // Only links, never a path or an option for xdg-open.
    if !["https://", "http://", "ftp://", "ssh://"].iter().any(|s| url.starts_with(s)) {
        return Err(format!("not a link: {url}"));
    }
    let mut child = std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not open the link (xdg-open: {e})"))?;
    // Reaped off the UI thread; its exit status says nothing useful.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// The local path of a `file://` link (`file:///p` or `file://localhost/p`,
/// percent-decoded); `None` for other links or another host's file.
pub(super) fn file_link(url: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let rest = url.strip_prefix("file://")?;
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    // A query or fragment isn't part of the path.
    let path = path.split(['?', '#']).next()?;
    if !path.starts_with('/') {
        return None;
    }
    let (mut bytes, mut it) = (Vec::new(), path.bytes());
    while let Some(b) = it.next() {
        if b == b'%' {
            let hex = [it.next()?, it.next()?];
            bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(b);
        }
    }
    Some(std::path::PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

impl App {
    pub(super) fn update_terminal(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Term(events) => {
                let mut tasks = Vec::new();
                for event in events {
                    match event {
                        // What it copied just before exiting still goes to the clipboard.
                        term::Event::Exited => {
                            self.term = None;
                            let what =
                                if self.term_kind == TermKind::Herdr { "herdr detached" } else { "The shell exited" };
                            self.set_status(format!("{what} — agents keep running in herdr"), false);
                            break;
                        }
                        // OSC 52 from programs in the terminal (herdr copies on select).
                        term::Event::Clipboard { primary: true, text } => {
                            tasks.push(iced::clipboard::write_primary(text));
                        }
                        term::Event::Clipboard { primary: false, text } => tasks.push(iced::clipboard::write(text)),
                        e @ term::Event::Wakeup => {
                            if let Some(t) = &mut self.term {
                                t.on_event(&e);
                            }
                        }
                    }
                }
                return Task::batch(tasks);
            }
            Msg::TermAction(action) => {
                let Some(t) = &mut self.term else { return Task::none() };
                return match t.apply(action) {
                    term::Effect::None => Task::none(),
                    term::Effect::Copy { primary: true, text } => iced::clipboard::write_primary(text),
                    term::Effect::Copy { primary: false, text } => iced::clipboard::write(text),
                    // A file shown by an agent opens in mira, as a tab.
                    term::Effect::Open(url) if url.starts_with("file://") => match file_link(&url) {
                        Some(path) if path.is_file() => self.update(Msg::OpenFile(path, None)),
                        _ => {
                            self.set_status(format!("Only files open from file:// links: {url}"), true);
                            Task::none()
                        }
                    },
                    term::Effect::Open(url) => {
                        if let Err(e) = open_url(&url) {
                            self.set_status(e, true);
                        }
                        Task::none()
                    }
                };
            }
            Msg::ReportColorScheme => self.report_color_scheme(),
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
        let id = self.next_term_id;
        self.next_term_id += 1;
        // Drop the old one first: its PTY hangs up on its program.
        self.term = None;
        match term::Terminal::spawn(id, term::Options { program, args, env, cwd: self.root.clone() }) {
            Ok(t) => self.term = Some(t),
            Err(e) => {
                self.set_status(format!("terminal: {e}"), true);
                return Task::none();
            }
        }
        self.workspace = None;
        // The client needs a moment to start reading its input.
        let report = if self.term_kind == TermKind::Herdr {
            Task::perform(blocking(|| std::thread::sleep(Duration::from_secs(2))), |()| Msg::ReportColorScheme)
        } else {
            Task::none()
        };
        Task::batch([self.focus_terminal(), report])
    }

    pub(super) fn focus_terminal(&mut self) -> Task<Msg> {
        self.term.as_ref().map_or_else(Task::none, |t| iced::widget::operation::focus(t.widget_id().clone()))
    }

    pub(super) fn term_write(&mut self, bytes: Vec<u8>) {
        if let Some(t) = &self.term {
            t.write(bytes);
        }
    }

    /// Tell herdr whether mira's palette is light or dark, as a terminal does
    /// (see `termio::color_scheme_report`), so its theme can follow.
    pub(super) fn report_color_scheme(&mut self) {
        if self.term_kind == TermKind::Herdr && self.term.is_some() {
            self.term_write(termio::color_scheme_report(termio::is_dark(theme::p())).to_vec());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn file_links_are_local_paths_and_never_reach_xdg_open() {
        assert_eq!(file_link("file:///tmp/a%20b.rs"), Some(PathBuf::from("/tmp/a b.rs")));
        assert_eq!(file_link("file://localhost/x/y.txt#L3"), Some(PathBuf::from("/x/y.txt")));
        assert_eq!(file_link("file:///q%3F.md?x"), Some(PathBuf::from("/q?.md")));
        assert_eq!(file_link("file://otherhost/x"), None);
        assert_eq!(file_link("file:///bad%zz"), None);
        assert_eq!(file_link("https://x"), None);
        assert!(open_url("file:///home/u/evil.desktop").is_err());
    }
}
