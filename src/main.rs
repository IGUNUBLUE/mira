//! mira — a lightweight, terminal-first workspace for reviewing and
//! committing changes made by coding agents.

mod app;
mod diff;
mod explorer;
mod git;
mod envfiles;
mod search;
mod session;
mod theme;
mod view;

use std::path::PathBuf;

fn main() -> iced::Result {
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("-h" | "--help")) {
        println!("Usage: mira [DIRECTORY]\n\nOpens DIRECTORY (default: current directory) in mira.");
        return Ok(());
    }
    if matches!(arg.as_deref(), Some("-V" | "--version")) {
        println!("mira {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
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

    iced::application(move || app::App::new(root.clone()), app::App::update, app::App::view)
        .title(app::App::title)
        .subscription(app::App::subscription)
        .theme(|_: &app::App| theme::theme())
        .window_size((1400.0, 900.0))
        .antialiasing(true)
        .run()
}
