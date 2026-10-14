# mira

A lightweight, terminal-first workspace for agent-driven development, written in Rust.

You talk to your coding agents (codex, kiro-cli, claude, …) in a terminal; mira is built
around that loop: ask → the agent acts → you review, correct, accept.

- **herdr in the center.** The terminal runs [herdr](https://herdr.dev), the terminal
  workspace for coding agents: workspaces, tabs, panes, agent status, and agents that keep
  running when mira closes. mira opens (or creates) the herdr workspace for its folder.
  Without herdr installed it falls back to your shell.
- **Review inbox.** mira asks herdr which agents work in this folder. Every time one goes
  from working to done, that *run* gets before/after snapshots of the working tree
  (temporary git index, no commits) and lands in Review, titled with what you asked
  (read from kiro's and codex's own logs), with its files, time and status. Mark runs
  reviewed, stage them or revert them. Review state survives restarts.
- **Diffs that talk back.** Side-by-side diff with intra-line highlights; `‹ ›` steps
  through a run's files. Click a line, write a comment, and it is typed into *that run's
  agent pane* through herdr (not whatever pane has focus), ready for you to press Enter.
- **Source control**: stage / unstage, commit, push, pull, recent history.
- **Commit message generation** (`✦ Generate message` in Git): Conventional Commits from the
  staged diff (or all changes when nothing is staged), with lockfiles excluded and the diff
  capped at 12k chars — same prompt as the `chatgpt-commit-gen` extension. mira runs
  `codex exec` (your ChatGPT login) in an empty temp dir with a read-only sandbox, so the
  agent only sees the prompt. Configure in `~/.config/mira/config`:
  `commit_model = "gpt-6-luna"`, or `commit_command = "…"` for any command that reads the
  prompt on stdin and prints the message (e.g. a local model via ollama).
- **Search & replace** (respects `.gitignore`), **explorer** and a syntax-highlighted editor
  whose clean tabs reload when an agent changes the file.

No extension host, no telemetry, no accounts. Git operations shell out to your `git`,
so hooks, signing and credential helpers behave exactly as in your terminal.

## Build & run

```bash
cargo build --release
./target/release/mira [--fresh] [DIRECTORY]
```

`--fresh` starts with an empty Review inbox. Config (`~/.config/mira/config`):
`herdr_session = "name"` attaches to a named herdr session instead of the default one;
`terminal = "shell"` skips herdr.

Recommended herdr settings (`~/.config/herdr/config.toml`): `[theme] name = "terminal"`
so herdr draws with mira's palette, and keep `[ui.toast] delivery = "system"` for
notifications (mira does not send its own, to avoid duplicates).

Themes: the four LAGC palettes (Tech / Calm, light / dark). The default, `auto`,
follows the active HyDE theme live (`~/.local/state/hyde/staterc`) and falls back to
LAGC Tech Light. `Ctrl+Shift+T` or the status-bar button cycles auto → each palette;
the choice is saved in `~/.config/mira/config` (`theme = "LAGC Tech Dark"`).

Icons come from any installed [Nerd Font](https://www.nerdfonts.com) (codicons for the UI,
seti/devicons for file types); set `icon_font = "…"` in the config to pick one. Without a
Nerd Font mira falls back to plain symbols. Drag the divider next to the inspector to
resize it; the width is remembered.

Fonts default to Inter and JetBrains Mono (medium weight); set `MIRA_FONT` /
`MIRA_MONO_FONT` to use other installed families.

Linux needs the usual Wayland/X11 and Vulkan or OpenGL libraries. Force the CPU
renderer (lower memory, no GPU) with `ICED_BACKEND=tiny-skia`.

## Shortcuts

| Keys | Action |
|---|---|
| ``Ctrl+` `` | Focus the terminal |
| `Ctrl+B` … | herdr's prefix (`Ctrl+B ?` lists its keys) |
| `Ctrl+Shift+M` | Focus mode (hide the inspector) |
| `Ctrl+Shift+R` / `G` / `F` / `E` | Inspector: Review / Git / Search / Files (again to hide) |
| `Ctrl+Shift+B` | Toggle inspector |
| `Ctrl+Shift+T` | Cycle theme |
| `Ctrl+Shift+V` | Paste in the terminal (bracketed) |
| `Ctrl+S` / `Ctrl+W` | Save / close tab |

`Ctrl+Shift+…` and `` Ctrl+` `` work even while the terminal has focus. Inside herdr, text
is copied on selection; the mouse wheel, right/middle clicks and Alt+key reach herdr.

## Roadmap

- Editor: line numbers, find in file, scroll-to-line, tree-sitter highlighting
- LSP completion (optional, per language)
- Fonts and keybindings in the config file
- Hunk navigation and per-hunk staging

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
