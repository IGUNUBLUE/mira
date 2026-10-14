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
  Clicking a turn opens its first changed file in a single preview tab; `‹ ›` steps through
  the rest. Click a line, write a comment, and it is typed into the agent's prompt.
- **Env**: discovers `.env*` files, masks values, compares against `.env.example`,
  shows per-turn key changes and warns when a `.env` is tracked or not ignored.
- **Source control**: stage / unstage, commit, push, pull, recent history.
- **Resume where you left off**: closing mira keeps your sessions. Reopening the same
  folder relaunches each agent on its own conversation (`kiro-cli --resume-id`,
  `codex resume <id>`, `claude --resume`, `opencode --session`, or "continue last") and
  restores the turns. Agents started by hand in a shell are adopted too. `mira --fresh`
  starts clean. State is kept in `~/.local/state/mira/` (mode 0600), never in the project.
- **Search & replace** (respects `.gitignore`), **explorer** and a syntax-highlighted editor
  whose clean tabs reload when an agent changes the file.

No extension host, no telemetry, no accounts. Git operations shell out to your `git`,
so hooks, signing and credential helpers behave exactly as in your terminal.

## Build & run

```bash
cargo build --release
./target/release/mira [--fresh] [DIRECTORY]
```

Fonts default to Inter and JetBrains Mono (medium weight); set `MIRA_FONT` /
`MIRA_MONO_FONT` to use other installed families.

Linux needs the usual Wayland/X11 and Vulkan or OpenGL libraries. Force the CPU
renderer (lower memory, no GPU) with `ICED_BACKEND=tiny-skia`.

## Shortcuts

| Keys | Action |
|---|---|
| ``Ctrl+` `` | Focus the agent terminal |
| `Ctrl+Shift+N` | New session (agent presets) |
| `Ctrl+Shift+M` | Focus mode (hide side panels) |
| `Ctrl+Shift+J` / `K` | Next / previous session |
| `Ctrl+Shift+R` / `G` / `D` / `F` / `E` | Inspector: Review / Git / .env / Search / Files (again to hide) |
| `Ctrl+B` | Toggle inspector |
| `Ctrl+Shift+C` / `V` | Copy / paste in the terminal |
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
