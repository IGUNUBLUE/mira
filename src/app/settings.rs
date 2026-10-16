//! Settings, the keyboard shortcuts list and About, opened from the gear in
//! the status bar (`Ctrl+,` for Settings), and mira's own window controls
//! (`window_controls = "on"`, for desktops that draw no title bar).
//!
//! Settings write `~/.config/mira/config` (`persist::config_set`, which keeps
//! the user's other lines and comments); choices apply at once, text fields
//! when Enter is pressed or the dialog closes.

use super::{App, AutoSave, Msg, blocking};
use crate::persist;
use crate::theme;

use iced::Task;

/// URL of mira's repository (About, Check for Updates).
pub const REPO_URL: &str = "https://github.com/IGUNUBLUE/mira";

/// A dialog over the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Settings,
    Shortcuts,
    About,
}

/// What the gear menu offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Settings,
    Shortcuts,
    Updates,
    About,
}

/// `window_controls`: who draws close / minimize / maximize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowControls {
    /// The desktop does (its title bar, or none on tiling compositors).
    #[default]
    Auto,
    /// mira draws a title bar with its own buttons.
    On,
    /// Neither: no title bar at all.
    Off,
}

impl WindowControls {
    pub fn parse(s: Option<&str>) -> Self {
        match s.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("on") => Self::On,
            Some("off") => Self::Off,
            _ => Self::Auto,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::On => "on",
            Self::Off => "off",
        }
    }

    /// Whether the desktop should decorate the window.
    pub fn decorated(self) -> bool {
        self == Self::Auto
    }

    pub fn from_config() -> Self {
        Self::parse(persist::config_get("window_controls").as_deref())
    }
}

/// A setting typed as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    AutosaveDelay,
    HerdrSession,
    CommitModel,
    CommitCommand,
}

impl Field {
    pub const ALL: [Field; 4] = [Field::AutosaveDelay, Field::HerdrSession, Field::CommitModel, Field::CommitCommand];

    pub fn key(self) -> &'static str {
        match self {
            Field::AutosaveDelay => "autosave_delay",
            Field::HerdrSession => "herdr_session",
            Field::CommitModel => "commit_model",
            Field::CommitCommand => "commit_command",
        }
    }

    /// Whether `value` can be saved (empty means the default).
    fn check(self, value: &str) -> Result<(), String> {
        let v = value.trim();
        match self {
            _ if v.is_empty() => Ok(()),
            Field::AutosaveDelay => match v.parse::<u64>() {
                Ok(ms) if ms >= 100 => Ok(()),
                _ => Err("Autosave delay: a number of milliseconds, 100 or more".into()),
            },
            Field::HerdrSession if !crate::herdr::valid_session(v) => {
                Err("herdr session: letters, digits, '-' and '_' only".into())
            }
            _ => Ok(()),
        }
    }
}

/// A choice made with buttons, applied at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setting {
    /// `theme`: "auto" or a palette's name.
    Theme(String),
    WindowControls(WindowControls),
    Autosave(&'static str),
    Terminal(&'static str),
    CommitProvider(&'static str),
}

/// What Settings shows, read when it opens (`view` never reads files): the
/// text fields as typed (saved on Enter or when it closes) and the choices.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drafts {
    pub values: [String; 4],
    pub autosave: String,
    pub terminal: String,
    pub provider: String,
}

impl Drafts {
    fn load() -> Self {
        Self {
            values: Field::ALL.map(|f| persist::config_get(f.key()).unwrap_or_default()),
            autosave: current("autosave", "afterDelay"),
            terminal: current("terminal", "herdr"),
            provider: current("commit_provider", "auto"),
        }
    }

    pub fn get(&self, f: Field) -> &str {
        &self.values[f as usize]
    }
}

/// The value of `key` in the config, or `default`.
pub fn current(key: &str, default: &str) -> String {
    persist::config_get(key).filter(|v| !v.trim().is_empty()).unwrap_or_else(|| default.to_string())
}

impl App {
    pub(super) fn update_settings(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::GearMenu(open) => self.gear_menu = open,
            Msg::Gear(item) => {
                self.gear_menu = false;
                return match item {
                    MenuItem::Settings => self.update(Msg::OpenDialog(Dialog::Settings)),
                    MenuItem::Shortcuts => self.update(Msg::OpenDialog(Dialog::Shortcuts)),
                    MenuItem::About => self.update(Msg::OpenDialog(Dialog::About)),
                    MenuItem::Updates => self.open_link(&format!("{REPO_URL}/releases")),
                };
            }
            Msg::OpenDialog(d) => {
                self.gear_menu = false;
                if d == Dialog::Settings {
                    self.drafts = Drafts::load();
                    self.settings_error = None;
                }
                if d == Dialog::About && self.herdr_version.is_none() {
                    self.dialog = Some(d);
                    return Task::perform(blocking(crate::herdr::version), Msg::HerdrVersion);
                }
                self.dialog = Some(d);
            }
            Msg::CloseDialog => {
                // A typed value that can't be saved keeps the dialog open with its
                // error; closing again (the error seen) drops it.
                if self.dialog == Some(Dialog::Settings) {
                    let seen = self.settings_error.is_some();
                    if !self.save_drafts() && !seen {
                        return Task::none();
                    }
                }
                self.dialog = None;
            }
            Msg::HerdrVersion(v) => self.herdr_version = Some(v.unwrap_or_else(|| "not found".into())),
            Msg::SettingChoice(choice) => return self.apply_choice(choice),
            Msg::SettingText(field, value) => {
                self.drafts.values[field as usize] = value;
                self.settings_error = None;
            }
            Msg::SettingSubmit => {
                self.save_drafts();
            }
            Msg::OpenConfigFile => {
                // What was typed is saved first, as when the dialog closes.
                if !self.save_drafts() {
                    return Task::none();
                }
                let Some(path) = persist::config_dir().map(|d| d.join("config")) else { return Task::none() };
                if !path.exists() {
                    // An empty file to edit; mira works without one.
                    if let Some(dir) = path.parent()
                        && let Err(e) = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&path, ""))
                    {
                        self.set_status(format!("config: {e}"), true);
                        return Task::none();
                    }
                }
                self.dialog = None;
                return self.update(Msg::OpenFile(path, None));
            }
            Msg::OpenLink(url) => return self.open_link(&url),
            Msg::CopyAbout => return iced::clipboard::write(self.about_text()),
            Msg::WindowMinimize => {
                return iced::window::latest().and_then(|id| iced::window::minimize(id, true));
            }
            Msg::WindowMaximize => return iced::window::latest().and_then(iced::window::toggle_maximize),
            Msg::WindowDrag => return iced::window::latest().and_then(iced::window::drag),
            _ => {}
        }
        Task::none()
    }

    fn open_link(&mut self, url: &str) -> Task<Msg> {
        if let Err(e) = super::terminal::open_url(url) {
            self.set_status(e, true);
        }
        Task::none()
    }

    fn save(&mut self, key: &str, value: &str) -> bool {
        match persist::config_set(key, value) {
            Ok(()) => true,
            Err(e) => {
                self.settings_error = Some(format!("Could not save {key}: {e}"));
                false
            }
        }
    }

    fn apply_choice(&mut self, choice: Setting) -> Task<Msg> {
        match choice {
            Setting::Theme(name) => {
                if let Some(c) = theme::Choice::parse(&name) {
                    self.theme = c;
                    self.apply_theme();
                    self.save("theme", &c.label());
                }
            }
            Setting::WindowControls(wc) => {
                let toggle = wc.decorated() != self.window_controls.decorated();
                self.window_controls = wc;
                self.save("window_controls", wc.label());
                if toggle {
                    return iced::window::latest().and_then(iced::window::toggle_decorations);
                }
            }
            Setting::Autosave(mode) => {
                if self.save("autosave", mode) {
                    self.drafts.autosave = mode.into();
                    self.autosave = AutoSave::from_config();
                }
            }
            Setting::Terminal(kind) => {
                if self.save("terminal", kind) {
                    self.drafts.terminal = kind.into();
                }
            }
            Setting::CommitProvider(p) => {
                if self.save("commit_provider", p) {
                    self.drafts.provider = p.into();
                }
            }
        }
        Task::none()
    }

    /// Save the typed fields that changed (a bad value stops there, with
    /// `settings_error` set). Returns whether everything was saved.
    fn save_drafts(&mut self) -> bool {
        for f in Field::ALL {
            let value = self.drafts.get(f).trim().to_string();
            if value == persist::config_get(f.key()).unwrap_or_default().trim() {
                continue;
            }
            if let Err(e) = f.check(&value) {
                self.settings_error = Some(e);
                return false;
            }
            if !self.save(f.key(), &value) {
                return false;
            }
        }
        self.autosave = AutoSave::from_config();
        true
    }

    /// What About shows, as text for a bug report.
    pub fn about_text(&self) -> String {
        format!(
            "mira {}\nherdr: {}\nterminal: {}\nOS: {} {}\nrenderer: {}",
            env!("CARGO_PKG_VERSION"),
            self.herdr_version.as_deref().unwrap_or("?"),
            if self.term_kind == super::TermKind::Herdr { "herdr" } else { "shell" },
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::env::var("ICED_BACKEND").unwrap_or_else(|_| "wgpu".into()),
        )
    }
}

/// mira's shortcuts, as listed in the dialog (and the guide).
pub const SHORTCUTS: [(&str, &str); 20] = [
    ("Ctrl+`", "Focus the terminal"),
    ("Ctrl+Shift+G", "Changes (focuses the commit message)"),
    ("Ctrl+Shift+F", "Search"),
    ("Ctrl+Shift+E", "Files"),
    ("Ctrl+Shift+B", "Show or hide the side panel"),
    ("Ctrl+Shift+M", "Focus mode"),
    ("Ctrl+Shift+T", "Next theme"),
    ("Ctrl+Shift+P, Ctrl+P", "Quick open (Ctrl+P outside the terminal)"),
    ("Ctrl+G", "Go to line"),
    ("Ctrl+F", "Find in the open file"),
    ("F3, Shift+F3", "Next, previous match"),
    ("Ctrl+,", "Settings"),
    ("Ctrl+S", "Save"),
    ("Ctrl+W", "Close the tab"),
    ("Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y", "Undo, redo"),
    ("Ctrl+Enter", "Commit (in the commit message)"),
    ("Ctrl+Shift+C, Ctrl+Shift+V", "Copy, paste in the terminal"),
    ("Shift+drag", "Select in the terminal while herdr has the mouse"),
    ("Ctrl+click", "Open a link in the terminal"),
    ("Escape", "Close the dialog, menu or find bar; cancel a confirmation"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_controls_parse_and_who_decorates() {
        assert_eq!(WindowControls::parse(None), WindowControls::Auto);
        assert_eq!(WindowControls::parse(Some(" ON ")), WindowControls::On);
        assert_eq!(WindowControls::parse(Some("off")), WindowControls::Off);
        assert_eq!(WindowControls::parse(Some("weird")), WindowControls::Auto);
        assert!(WindowControls::Auto.decorated());
        assert!(!WindowControls::On.decorated() && !WindowControls::Off.decorated());
        assert_eq!(WindowControls::On.label(), "on");
    }

    #[test]
    fn typed_settings_are_checked() {
        assert!(Field::AutosaveDelay.check("").is_ok());
        assert!(Field::AutosaveDelay.check("250").is_ok());
        assert!(Field::AutosaveDelay.check("50").is_err());
        assert!(Field::AutosaveDelay.check("soon").is_err());
        assert!(Field::HerdrSession.check("work-2").is_ok());
        assert!(Field::HerdrSession.check("a b").is_err());
        assert!(Field::CommitCommand.check("ollama run x").is_ok());
        assert_eq!(Field::ALL.map(|f| f as usize), [0, 1, 2, 3]);
    }
}
