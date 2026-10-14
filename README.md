# mira

A lightweight, terminal-first workspace for agent-driven development, written in Rust.

You talk to your coding agents (codex, kiro-cli, claude, gemini, …) in a terminal;
mira is built around that loop: ask → the agent acts → you review, correct, accept.

- **Agent sessions**: each session is a terminal running an agent CLI (presets found on
  `$PATH`) or a plain shell, with status (working / waiting / idle) and a notification
  when an unfocused agent needs you.
- **Turns**: every prompt opens a turn; when the agent goes quiet mira snapshots the
  working tree (temporary git index, no commits), so each turn has its own before/after
  diff and can be reverted.
- **Review**: side-by-side diff with intra-line highlights, per turn or for the whole session.
- **Env**: discovers `.env*` files, masks values, compares against `.env.example`,
  shows per-turn key changes and warns when a `.env` is tracked or not ignored.
- **Source control**: stage / unstage, commit, push, pull, recent history.
- **Search & replace** (respects `.gitignore`), **explorer** and a syntax-highlighted editor
  whose clean tabs reload when an agent changes the file.

No extension host, no telemetry, no accounts. Git operations shell out to your `git`,
so hooks, signing and credential helpers behave exactly as in your terminal.

## Build & run

```bash
cargo build --release
./target/release/mira [DIRECTORY]
```

Linux needs the usual Wayland/X11 and Vulkan or OpenGL libraries. Force the CPU
renderer (lower memory, no GPU) with `ICED_BACKEND=tiny-skia`.

## Shortcuts

| Keys | Action |
|---|---|
| ``Ctrl+` `` | Focus the agent terminal |
| `Ctrl+Shift+N` | New session (agent presets) |
| `Ctrl+Shift+M` | Focus mode (hide side panels) |
| `Ctrl+Shift+R` / `G` / `V` / `F` / `E` | Inspector: Review / Git / Env / Search / Files |
| `Ctrl+B` | Toggle inspector |
| `Ctrl+S` / `Ctrl+W` | Save / close tab |

`Ctrl+Shift+…` and `` Ctrl+` `` work even while the terminal has focus.

## Roadmap

- Editor: line numbers, find in file, scroll-to-line, tree-sitter highlighting
- Commit message generation (provider-agnostic: any OpenAI-compatible endpoint, local models)
- LSP completion (optional, per language)
- Config file (fonts, theme, keybindings), dark theme
- Hunk navigation and per-hunk staging

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
