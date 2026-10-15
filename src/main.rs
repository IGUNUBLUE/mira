//! mira — a lightweight, terminal-first workspace for reviewing and
//! committing changes made by coding agents.

mod agentlog;
mod app;
mod commitgen;
mod diff;
mod explorer;
mod files;
mod git;
mod herdr;
mod history;
mod icons;
mod openai;
mod persist;
mod proc;
mod runs;
mod search;
mod termio;
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

fn main() -> iced::Result {
    pick_renderer();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let fresh = args.iter().any(|a| a == "--fresh");
    // `--pick` used to ask for a folder; herdr's workspaces decide now. Accepted for old launchers.
    args.retain(|a| a != "--fresh" && a != "--pick");
    let arg = args.into_iter().next();
    if matches!(arg.as_deref(), Some("-h" | "--help")) {
        println!(
            "Usage: mira [--fresh] [DIRECTORY]\n\nOpens DIRECTORY in mira and focuses its herdr workspace. Without one,\n\
             mira opens the current directory, or (from the home directory, e.g. the app launcher) the\n\
             folder of herdr's focused workspace. mira then follows herdr: switching workspace switches\n\
             the folder. --fresh forgets the recorded agent turns."
        );
        return Ok(());
    }
    if matches!(arg.as_deref(), Some("-V" | "--version")) {
        println!("mira {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let start = match arg {
        Some(dir) => match PathBuf::from(&dir).canonicalize().ok().filter(|p| p.is_dir()) {
            Some(root) => app::Start::Folder(root),
            None => {
                eprintln!("mira: {dir}: not a directory");
                std::process::exit(2);
            }
        },
        None => {
            let home = std::env::var_os("HOME").map(PathBuf::from).and_then(|h| h.canonicalize().ok());
            match std::env::current_dir().ok().and_then(|d| d.canonicalize().ok()) {
                // Launched from a project directory: open it.
                Some(cwd) if Some(&cwd) != home.as_ref() => app::Start::Folder(cwd),
                // From the launcher (cwd = home): whatever herdr is focused on.
                _ => app::Start::Follow,
            }
        }
    };

    iced::application(move || app::App::new(start.clone(), fresh), app::App::update, app::App::view)
        .title(app::App::title)
        .subscription(app::App::subscription)
        .theme(|_: &app::App| theme::theme())
        .default_font(theme::ui_font())
        .window_size((1400.0, 900.0))
        // mira saves edited files before closing (see `Msg::CloseRequested`).
        .exit_on_close_request(false)
        .antialiasing(true)
        .run()
}
