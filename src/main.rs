//! mira — a lightweight, terminal-first workspace for reviewing and
//! committing changes made by coding agents.

mod agentlog;
mod app;
mod commitgen;
mod diff;
mod explorer;
mod git;
mod herdr;
mod icons;
mod persist;
mod runs;
mod search;
mod termio;
mod theme;
mod view;

use std::path::PathBuf;

/// Default to iced's GPU renderer on the OpenGL backend.
///
/// - wgpu on Vulkan gamma-encoded colors twice on the owner's machine
///   (`#3B6ADF` showed as `#84ACF0`): washed out, low contrast. GL is exact.
/// - The CPU renderer (tiny-skia) uses far less memory (32 MB vs 112 MB idle)
///   but typing and resizing panels lag visibly at this window size, so it is
///   opt-in: `ICED_BACKEND=tiny-skia`.
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

/// Ask for a folder with the desktop's own dialog (zenity or yad), starting in
/// `~/Projects` when it exists. `None` if cancelled. Without a dialog tool,
/// fall back to the current directory.
fn pick_folder() -> Option<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let start = [home.join("Projects"), home.clone()].into_iter().find(|p| p.is_dir()).unwrap_or(home);
    let start = format!("{}/", start.display());
    let title = "Open a folder in mira";
    let tools: [(&str, Vec<&str>); 2] = [
        ("zenity", vec!["--file-selection", "--directory", "--title", title, "--filename", &start]),
        ("yad", vec!["--file", "--directory", "--title", title, "--filename", &start]),
    ];
    for (tool, args) in tools {
        match std::process::Command::new(tool).args(&args).output() {
            Ok(out) if out.status.success() => {
                let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
                return (!dir.is_empty()).then_some(dir);
            }
            Ok(_) => return None,
            Err(_) => continue,
        }
    }
    std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned())
}

fn main() -> iced::Result {
    pick_renderer();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let fresh = args.iter().any(|a| a == "--fresh");
    let pick = args.iter().any(|a| a == "--pick");
    args.retain(|a| a != "--fresh" && a != "--pick");
    let mut arg = args.into_iter().next();
    if matches!(arg.as_deref(), Some("-h" | "--help")) {
        println!(
            "Usage: mira [--fresh] [--pick] [DIRECTORY]\n\nOpens DIRECTORY (default: current directory) in mira.\n\
             --fresh forgets the recorded agent turns; --pick asks for the folder when none is given."
        );
        return Ok(());
    }
    if matches!(arg.as_deref(), Some("-V" | "--version")) {
        println!("mira {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if pick && arg.is_none() {
        match pick_folder() {
            Some(dir) => arg = Some(dir),
            // Cancelled: nothing to open.
            None => return Ok(()),
        }
    }

    let root = arg
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .and_then(|p| p.canonicalize().ok())
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| {
            eprintln!("mira: not a directory");
            std::process::exit(2);
        });

    iced::application(move || app::App::new(root.clone(), fresh), app::App::update, app::App::view)
        .title(app::App::title)
        .subscription(app::App::subscription)
        .theme(|_: &app::App| theme::theme())
        .default_font(theme::ui_font())
        .window_size((1400.0, 900.0))
        .antialiasing(true)
        .run()
}
