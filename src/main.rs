//! mira — a lightweight, terminal-first workspace for reviewing and
//! committing changes made by coding agents.

mod agentlog;
mod app;
mod code;
mod codeview;
mod commitgen;
mod diff;
mod explorer;
mod fileops;
mod files;
mod git;
mod gutter;
mod herdr;
mod history;
mod hunks;
mod icons;
mod imagefile;
mod openai;
mod persist;
mod proc;
mod quickopen;
mod rightclick;
mod runs;
mod search;
mod spinner;
mod term;
mod termio;
mod textfind;
mod theme;
mod view;

use std::path::PathBuf;

/// Default to iced's GPU renderer on the OpenGL backend.
///
/// - On some Vulkan drivers colors were gamma-encoded twice (`#3B6ADF` showed
///   as `#84ACF0`): washed out, low contrast. GL is exact.
/// - The CPU renderer (tiny-skia) uses less memory, but typing and resizing
///   panels lag visibly in a large window, so it is opt-in:
///   `ICED_BACKEND=tiny-skia`.
///
/// An explicit `ICED_BACKEND` or `WGPU_BACKEND` from the user wins.
fn pick_renderer() {
    if std::env::var_os("WGPU_BACKEND").is_none() && std::env::var_os("ICED_BACKEND").is_none() {
        // SAFETY: first statement of `main`, before any thread exists, so no
        // other code can be reading the environment concurrently.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("WGPU_BACKEND", "gl");
        }
    }
}

/// What the command line asked for.
#[derive(Debug, PartialEq)]
enum Cli {
    Help,
    Version,
    Run { fresh: bool, dir: Option<std::ffi::OsString> },
}

const USAGE: &str = "Usage: mira [--fresh] [DIRECTORY]\n\nOpens DIRECTORY in mira and focuses its herdr workspace. Without one,\n\
                     mira opens the current directory, or (from the home directory, e.g. the app launcher) the\n\
                     folder of herdr's focused workspace. mira then follows herdr: switching workspace switches\n\
                     the folder. --fresh forgets the recorded agent turns.";

/// Arguments as the OS gives them: a folder name needn't be UTF-8.
fn parse_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Cli, String> {
    let (mut fresh, mut dir, mut only_paths) = (false, None, false);
    for arg in args {
        match arg.to_str() {
            Some("--") if !only_paths => only_paths = true,
            Some("-h" | "--help") if !only_paths => return Ok(Cli::Help),
            Some("-V" | "--version") if !only_paths => return Ok(Cli::Version),
            Some("--fresh") if !only_paths => fresh = true,
            // `--pick` used to ask for a folder; herdr's workspaces decide now. Accepted for old launchers.
            Some("--pick") if !only_paths => {}
            Some(flag) if flag.starts_with('-') && !only_paths => return Err(format!("unknown option {flag}")),
            _ if dir.is_some() => return Err("only one directory can be opened".into()),
            _ => dir = Some(arg),
        }
    }
    Ok(Cli::Run { fresh, dir })
}

fn main() -> iced::Result {
    pick_renderer();
    let (fresh, arg) = match parse_args(std::env::args_os().skip(1)) {
        Ok(Cli::Help) => {
            println!("{USAGE}");
            return Ok(());
        }
        Ok(Cli::Version) => {
            println!("mira {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Ok(Cli::Run { fresh, dir }) => (fresh, dir),
        Err(e) => {
            eprintln!("mira: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    let start = match arg {
        Some(dir) => match PathBuf::from(&dir).canonicalize() {
            Ok(root) if root.is_dir() => app::Start::Folder(root),
            Ok(_) => {
                eprintln!("mira: {}: not a directory", dir.to_string_lossy());
                std::process::exit(2);
            }
            Err(e) => {
                eprintln!("mira: {}: {e}", dir.to_string_lossy());
                std::process::exit(2);
            }
        },
        None => {
            let home = std::env::var_os("HOME").map(PathBuf::from).and_then(|h| h.canonicalize().ok());
            match std::env::current_dir().ok().and_then(|d| d.canonicalize().ok()) {
                // Launched from a project directory: open it. From the launcher (cwd = home,
                // or `/` for some): whatever herdr is focused on.
                Some(cwd) if Some(&cwd) != home.as_ref() && cwd.parent().is_some() => app::Start::Folder(cwd),
                _ => app::Start::Follow,
            }
        }
    };

    let window = iced::window::Settings {
        size: iced::Size::new(1400.0, 900.0),
        // mira saves edited files before closing (see `Msg::CloseRequested`).
        exit_on_close_request: false,
        // Raw 64×64 RGBA (assets/mira-64.rgba): no image decoder needed at runtime.
        icon: iced::window::icon::from_rgba(include_bytes!("../assets/mira-64.rgba").to_vec(), 64, 64).ok(),
        // `window_controls = "on"`: mira draws the title bar itself.
        decorations: app::WindowControls::from_config().decorated(),
        // Matches mira.desktop, so launchers and docks show its icon (Wayland app id, X11 class).
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: "mira".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    iced::application(move || app::App::new(start.clone(), fresh), app::App::update, app::App::view)
        .title(app::App::title)
        .subscription(app::App::subscription)
        .theme(|_: &app::App| theme::theme())
        .default_font(theme::ui_font())
        .window(window)
        .antialiasing(true)
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    fn parse(args: &[&str]) -> Result<Cli, String> {
        parse_args(args.iter().map(OsString::from))
    }

    #[test]
    fn arguments_are_read_as_given() {
        assert_eq!(parse(&[]), Ok(Cli::Run { fresh: false, dir: None }));
        assert_eq!(parse(&["--fresh", "a"]), Ok(Cli::Run { fresh: true, dir: Some("a".into()) }));
        assert_eq!(parse(&["a", "-h"]), Ok(Cli::Help));
        assert_eq!(parse(&["--pick"]), Ok(Cli::Run { fresh: false, dir: None }));
        assert_eq!(parse(&["--", "-dir"]), Ok(Cli::Run { fresh: false, dir: Some("-dir".into()) }));
        assert!(parse(&["--foo"]).unwrap_err().contains("--foo"));
        assert!(parse(&["a", "b"]).is_err());
        // A Latin-1 folder name: no panic, passed on as is.
        let latin1 = OsString::from_vec(b"caf\xe9".to_vec());
        assert_eq!(parse_args([latin1.clone()]), Ok(Cli::Run { fresh: false, dir: Some(latin1) }));
    }
}
