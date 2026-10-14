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

/// Default to iced's CPU renderer (tiny-skia). For a text UI it is the better
/// trade, measured on the owner's machine (same window, herdr inside):
///
/// | renderer      | RSS idle | RSS streaming | CPU streaming |
/// |---------------|----------|---------------|---------------|
/// | wgpu Vulkan   | 199 MB   | 231 MB        | 134 %         |
/// | wgpu GL       | 112 MB   | 145 MB        | 133 %         |
/// | tiny-skia     |  32 MB   |  65 MB        |  55 %         |
///
/// It also draws exact colors, while wgpu on Vulkan gamma-encoded them twice
/// here (`#3B6ADF` showed as `#84ACF0`). An explicit `ICED_BACKEND` or
/// `WGPU_BACKEND` from the user wins (`ICED_BACKEND=wgpu` for the GPU).
fn pick_renderer() {
    if std::env::var_os("WGPU_BACKEND").is_none() && std::env::var_os("ICED_BACKEND").is_none() {
        // SAFETY: first statement of `main`, before any thread exists, so no
        // other code can be reading the environment concurrently.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("ICED_BACKEND", "tiny-skia");
        }
    }
}

fn main() -> iced::Result {
    pick_renderer();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let fresh = args.iter().any(|a| a == "--fresh");
    args.retain(|a| a != "--fresh");
    let arg = args.into_iter().next();
    if matches!(arg.as_deref(), Some("-h" | "--help")) {
        println!(
            "Usage: mira [--fresh] [DIRECTORY]\n\nOpens DIRECTORY (default: current directory) in mira.\n\
             Agent sessions open there last time are resumed; --fresh starts clean."
        );
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

    iced::application(move || app::App::new(root.clone(), fresh), app::App::update, app::App::view)
        .title(app::App::title)
        .subscription(app::App::subscription)
        .theme(|_: &app::App| theme::theme())
        .default_font(theme::ui_font())
        .window_size((1400.0, 900.0))
        .antialiasing(true)
        .run()
}
