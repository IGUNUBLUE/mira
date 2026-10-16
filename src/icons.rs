//! Icons from an installed Nerd Font: codicons (the VSCodium UI set) for the
//! UI and seti / devicons for file types. Nothing is bundled; without a Nerd
//! Font the UI falls back to plain Unicode symbols and files get no icon.
//!
//! Font: `icon_font` in `~/.config/mira/config`, else the first Nerd Font that
//! `fc-list` reports (preferring the symbols-only and proportional cuts).

use crate::theme::p;
use iced::{Color, Font};
use std::sync::OnceLock;

static FONT: OnceLock<Option<Font>> = OnceLock::new();

/// The icon font, if one is installed. Detected once.
pub fn font() -> Option<Font> {
    *FONT.get_or_init(|| {
        let name = crate::persist::config_get("icon_font").filter(|f| !f.trim().is_empty()).or_else(detect)?;
        let name: &'static str = Box::leak(name.into_boxed_str());
        Some(Font::with_name(name))
    })
}

fn detect() -> Option<String> {
    let out = std::process::Command::new("fc-list")
        .args([":", "family"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    pick(&String::from_utf8_lossy(&out.stdout))
}

/// Best Nerd Font family in `fc-list : family` output.
fn pick(list: &str) -> Option<String> {
    let families: Vec<&str> = list.lines().flat_map(|l| l.split(',')).map(str::trim).collect();
    let rank = |f: &str| match f {
        "Symbols Nerd Font" => 0,
        f if f.ends_with(" Nerd Font Propo") => 1,
        f if f.ends_with(" Nerd Font") => 2,
        "Symbols Nerd Font Mono" => 3,
        f if f.ends_with(" Nerd Font Mono") => 4,
        _ => 99,
    };
    // Same rank: prefer the JetBrains Mono build, which matches the code font.
    families
        .iter()
        .copied()
        .filter(|f| rank(f) < 99)
        .min_by_key(|f| (rank(f), !f.starts_with("JetBrainsMono Nerd"), *f))
        .map(str::to_string)
}

/// A UI icon: Nerd Font codepoint (codicon) and a Unicode fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Add,
    Remove,
    Check,
    ChevronRight,
    ChevronDown,
    ChevronLeft,
    ArrowUp,
    ArrowDown,
    Refresh,
    Files,
    SourceControl,
    Search,
    Terminal,
    Sparkle,
    Discard,
    Diff,
    Send,
    GoToFile,
    Fold,
    Unfold,
    ScreenFull,
    ScreenNormal,
    PanelRight,
    PanelRightOff,
    ColorMode,
    Bell,
    Branch,
    Folder,
    FolderOpen,
    DotFilled,
    Account,
    Close,
    Preview,
    Gear,
    WinMinimize,
    WinMaximize,
    WinClose,
}

impl Icon {
    fn glyphs(self) -> (char, &'static str) {
        use Icon::*;
        match self {
            Add => ('\u{EA60}', "+"),
            Remove => ('\u{EB3B}', "−"),
            Check => ('\u{EAB2}', "✓"),
            ChevronRight => ('\u{EAB6}', "▸"),
            ChevronDown => ('\u{EAB4}', "▾"),
            ChevronLeft => ('\u{EAB5}', "‹"),
            ArrowUp => ('\u{EAA1}', "↑"),
            ArrowDown => ('\u{EA9A}', "↓"),
            Refresh => ('\u{EB37}', "⟳"),
            Files => ('\u{EAF0}', ""),
            SourceControl => ('\u{EA68}', ""),
            Search => ('\u{EA6D}', ""),
            Terminal => ('\u{EA85}', ""),
            Sparkle => ('\u{EC10}', "✦"),
            Discard => ('\u{EAE2}', "↶"),
            Diff => ('\u{EAE1}', "⇄"),
            Send => ('\u{EC0F}', "›"),
            GoToFile => ('\u{EA94}', "↗"),
            Fold => ('\u{EAF5}', "⇡"),
            Unfold => ('\u{EB73}', "⇣"),
            ScreenFull => ('\u{EB4C}', "⤢"),
            ScreenNormal => ('\u{EB4D}', "⤡"),
            PanelRight => ('\u{EBF4}', "▥"),
            PanelRightOff => ('\u{EC00}', "▢"),
            ColorMode => ('\u{EAC6}', "◐"),
            Bell => ('\u{EAA2}', "◆"),
            Branch => ('\u{E725}', "⑂"),
            Folder => ('\u{EA83}', "□"),
            FolderOpen => ('\u{EAF7}', "■"),
            DotFilled => ('\u{EA71}', "●"),
            Account => ('\u{EB99}', "@"),
            Close => ('\u{EA76}', "×"),
            Preview => ('\u{EB2F}', "◫"),
            Gear => ('\u{EB51}', "⚙"),
            WinMinimize => ('\u{EABA}', "—"),
            WinMaximize => ('\u{EAB9}', "□"),
            WinClose => ('\u{EAB8}', "✕"),
        }
    }

    /// The glyph to draw, or the fallback (possibly empty) without a Nerd Font.
    pub fn glyph(self) -> String {
        let (nerd, plain) = self.glyphs();
        if font().is_some() { nerd.to_string() } else { plain.to_string() }
    }
}

/// File-type icon and its color (seti-like hues taken from the palette).
pub fn file_icon(name: &str) -> (char, Color) {
    let c = p();
    let (blue, green, yellow, orange, red, purple, grey) =
        (hex(c.ansi[12]), c.added, hex(c.ansi[11]), c.modified, c.deleted, hex(c.ansi[13]), c.muted);
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let by_name = match lower.as_str() {
        "cargo.toml" | "cargo.lock" => Some(('\u{E68B}', orange)),
        "dockerfile" | "containerfile" | "compose.yml" | "compose.yaml" | "docker-compose.yml" => {
            Some(('\u{E650}', blue))
        }
        "makefile" | "justfile" => Some(('\u{E673}', orange)),
        "license" | "license-mit" | "license-apache" | "copying" => Some(('\u{E60A}', yellow)),
        ".gitignore" | ".gitattributes" | ".gitmodules" => Some(('\u{E65D}', grey)),
        "package.json" => Some(('\u{E71E}', red)),
        ".editorconfig" => Some(('\u{E652}', grey)),
        "agents.md" | "claude.md" => Some(('\u{EB08}', purple)),
        _ if lower == ".env" || lower.starts_with(".env.") || lower.ends_with(".env") => Some(('\u{E615}', yellow)),
        _ if lower.starts_with("readme") => Some(('\u{E66A}', blue)),
        _ => None,
    };
    if let Some(i) = by_name {
        return i;
    }
    match ext {
        "rs" => ('\u{E68B}', orange),
        "py" | "pyi" => ('\u{E606}', blue),
        "js" | "mjs" | "cjs" => ('\u{E60C}', yellow),
        "ts" | "mts" | "cts" => ('\u{E628}', blue),
        "jsx" | "tsx" => ('\u{E625}', blue),
        "go" => ('\u{E627}', blue),
        "c" | "h" => ('\u{E649}', blue),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => ('\u{E646}', blue),
        "java" => ('\u{E66D}', red),
        "kt" | "kts" => ('\u{E634}', orange),
        "swift" => ('\u{E699}', orange),
        "rb" => ('\u{E605}', red),
        "php" => ('\u{E608}', purple),
        "lua" => ('\u{E620}', blue),
        "zig" => ('\u{E6A9}', orange),
        "dart" => ('\u{E64C}', blue),
        "ex" | "exs" => ('\u{E62D}', purple),
        "hs" => ('\u{E61F}', purple),
        "nix" => ('\u{E843}', blue),
        "sh" | "bash" | "zsh" | "fish" => ('\u{E691}', green),
        "ps1" => ('\u{E683}', blue),
        "md" | "mdx" | "markdown" => ('\u{E609}', blue),
        "txt" | "log" => ('\u{E64E}', grey),
        "json" | "jsonc" | "json5" => ('\u{E80B}', yellow),
        "toml" => ('\u{E6B2}', grey),
        "yml" | "yaml" => ('\u{E6A8}', purple),
        "xml" => ('\u{E619}', orange),
        "ini" | "conf" | "cfg" => ('\u{E615}', grey),
        "lock" => ('\u{E672}', grey),
        "html" | "htm" => ('\u{E60E}', orange),
        "css" => ('\u{E614}', blue),
        "scss" | "sass" => ('\u{E603}', purple),
        "vue" => ('\u{E6A0}', green),
        "svelte" => ('\u{E697}', red),
        "svg" => ('\u{E698}', purple),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" => ('\u{E60D}', purple),
        "mp3" | "wav" | "flac" | "ogg" => ('\u{E638}', red),
        "mp4" | "mkv" | "webm" | "mov" => ('\u{E69F}', red),
        "pdf" => ('\u{E67D}', red),
        "zip" | "gz" | "tar" | "xz" | "zst" | "7z" => ('\u{E6AA}', grey),
        "sql" | "db" | "sqlite" => ('\u{E64D}', yellow),
        "csv" => ('\u{E64A}', green),
        "graphql" | "gql" => ('\u{E662}', purple),
        "tf" | "hcl" => ('\u{E69A}', purple),
        "vim" => ('\u{E62B}', green),
        "ttf" | "otf" | "woff" | "woff2" => ('\u{E659}', red),
        _ => ('\u{E64E}', grey),
    }
}

/// `#RRGGBB` → color.
fn hex(s: &str) -> Color {
    match u32::from_str_radix(s.trim_start_matches('#'), 16) {
        Ok(v) if s.len() == 7 => Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8),
        _ => p().muted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_symbols_then_proportional_nerd_fonts() {
        let list = "Inter\nJetBrainsMono Nerd Font Mono,JetBrainsMono NFM\nJetBrainsMono Nerd Font,JetBrainsMono NF\nCaskaydiaCove Nerd Font Propo\n";
        assert_eq!(pick(list).as_deref(), Some("CaskaydiaCove Nerd Font Propo"));
        assert_eq!(pick("Symbols Nerd Font\nFoo Nerd Font Propo").as_deref(), Some("Symbols Nerd Font"));
        assert_eq!(
            pick("Atk Nerd Font Propo\nJetBrainsMono Nerd Font Propo").as_deref(),
            Some("JetBrainsMono Nerd Font Propo")
        );
        assert_eq!(pick("Inter\nDejaVu Sans"), None);
    }

    #[test]
    fn maps_file_types() {
        assert_eq!(file_icon("main.rs").0, '\u{E68B}');
        assert_eq!(file_icon(".env.local").0, '\u{E615}');
        assert_eq!(file_icon("README.md").0, '\u{E66A}');
        assert_eq!(file_icon("unknown.xyz").0, '\u{E64E}');
    }
}
