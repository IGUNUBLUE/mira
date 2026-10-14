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
- **Source control**: stage / unstage, commit, push, pull, recent history.
- **Commit message generation** (`✦ Generate message` in Git): Conventional Commits from the
  staged diff (or all changes when nothing is staged), with lockfiles excluded and the diff
  capped at 12k chars — same prompt as the `chatgpt-commit-gen` extension. mira runs
  `codex exec` (your ChatGPT login) in an empty temp dir with a read-only sandbox, so the
  agent only sees the prompt. Configure in `~/.config/mira/config`:
  `commit_model = "gpt-6-luna"`, or `commit_command = "…"` for any command that reads the
  prompt on stdin and prints the message (e.g. a local model via ollama).
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

Themes: the four LAGC palettes (Tech / Calm, light / dark). The default, `auto`,
follows the active HyDE theme live (`~/.local/state/hyde/staterc`) and falls back to
LAGC Tech Light. `Ctrl+Shift+T` or the status-bar button cycles auto → each palette;
the choice is saved in `~/.config/mira/config` (`theme = "LAGC Tech Dark"`).

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
| `Ctrl+Shift+R` / `G` / `F` / `E` | Inspector: Review / Git / Search / Files (again to hide) |
| `Ctrl+B` | Toggle inspector |
| `Ctrl+Shift+T` | Cycle theme |
| `Ctrl+Shift+C` / `V` | Copy / paste in the terminal |
| `Ctrl+S` / `Ctrl+W` | Save / close tab |

`Ctrl+Shift+…` and `` Ctrl+` `` work even while the terminal has focus.

## Roadmap

- Editor: line numbers, find in file, scroll-to-line, tree-sitter highlighting
- LSP completion (optional, per language)
- Fonts and keybindings in the config file
- Hunk navigation and per-hunk staging

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
