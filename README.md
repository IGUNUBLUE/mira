# mira

A lightweight, terminal-first workspace for agent-driven development, written in Rust.

*Mira* is Spanish for “look!”, which is what it is for: see what your agents did. (In Latin it
means “wonderful”, as in the star Mira.)

You talk to your coding agents (codex, kiro-cli, claude, …) in a terminal; mira is built
around that loop: ask → the agent acts → you review, correct, accept.

- **herdr in the center.** The terminal runs [herdr](https://herdr.dev), the terminal
  workspace for coding agents: workspaces, tabs, panes, agent status, and agents that keep
  running when mira closes. mira opens (or creates) the herdr workspace for its folder.
  Without herdr installed it falls back to your shell.
  The terminal header lists the agents working in this folder with their status; click one to
  jump to its pane (across workspaces). The status bar's “need you” jumps to the blocked agent.
- **Changes, with authors.** The Changes panel is your git working tree (stage, commit,
  push), and each file says which agent changed it and when (`kiro · 2m`, hover for what you
  asked). mira asks herdr which agents work in this folder and snapshots the working tree
  around each of their turns (temporary git index, no commits). Files changed since you last
  opened them get a dot; the tab and status bar count them. Switch to **By agent run** to see
  each turn with its files, titled with your prompt (read from kiro's and codex's logs), and
  stage or revert a whole turn. Turns without changes never show up; a turn interrupted by a
  quick follow-up stays one turn.
- **Diffs that talk back.** Side-by-side diff with intra-line highlights; `‹ ›` steps
  through a turn's files. Click a line, write a comment, and it is typed into the pane of the
  agent that changed that file, through herdr (not whatever pane has focus), ready for you to
  press Enter.
- **Source control**: stage / unstage, commit, push, pull, recent history.
- **Commit message generation** (the sparkle button next to the commit box, which grows with
  the message): Conventional Commits from the
  staged diff (or all changes when nothing is staged), with lockfiles excluded and the diff
  capped at 12k chars — same prompt as the `chatgpt-commit-gen` extension. The message streams
  into the box as it is written; while generating, the button spins and clicking it stops and
  restores what you had. Providers (`commit_provider` in `~/.config/mira/config`):
  - `chatgpt` — your ChatGPT subscription. The first click opens the browser to sign in (same
    OAuth flow as `codex login`); the account icon next to Pull/Push signs out.
  - `openai` — an API key from `OPENAI_API_KEY` or `~/.config/mira/openai_key` (chmod 600).
  - `command` — `commit_command = "…"`, any command that reads the prompt on stdin and prints
    the message (e.g. a local model via ollama), streamed as it prints.
  - `auto` (default) — ChatGPT session if signed in, else API key, else `commit_command`, else
    sign in with ChatGPT.

  `commit_model = "gpt-6-luna"` picks the model. Only the diff and its `--stat` are sent, and
  only when you click; this is the one network request mira makes (through `curl`). The
  subscription endpoint is undocumented and OpenAI may change it.
- **Search & replace** (respects `.gitignore`), **explorer** and a syntax-highlighted editor
  whose clean tabs reload when an agent changes the file.
- **Undo / redo** (`Ctrl+Z`, `Ctrl+Shift+Z` / `Ctrl+Y`) in the editor and the commit box, one
  step per word like VSCodium. Reloads caused by an agent are undoable too.
- **Autosave**, on by default: files save 1 s after you stop typing, and when you leave the
  window, close a tab or quit. Set `autosave = "afterDelay" | "onFocusChange" |
  "onWindowChange" | "off"` and `autosave_delay = "1000"` (ms) in `~/.config/mira/config`.
  Autosave never overwrites a file that changed on disk under your edits (e.g. an agent wrote
  it): it pauses for that tab and tells you; `Ctrl+S` overwrites.

No extension host, no telemetry, no accounts required. Git operations shell out to your `git`,
so hooks, signing and credential helpers behave exactly as in your terminal.

## Build & run

```bash
cargo build --release
./target/release/mira [--fresh] [DIRECTORY]
```

mira never asks for a folder; herdr's workspaces decide. With `DIRECTORY` (or when run from a
project directory) mira opens it and focuses its herdr workspace, creating one if needed. From
the app launcher it opens the folder of herdr's focused workspace (the last folder if herdr
isn't up yet). After that it follows herdr: switch workspace in herdr and mira's Changes,
Files, Search and history switch with it; open file tabs stay. `--fresh` forgets the recorded
agent turns. Config (`~/.config/mira/config`):
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
resize it, and the one above History in Changes to split it; both sizes are remembered.

Fonts default to Inter and JetBrains Mono (medium weight); set `MIRA_FONT` /
`MIRA_MONO_FONT` to use other installed families.

Linux needs the usual Wayland/X11 and OpenGL libraries. mira uses the GPU through wgpu's OpenGL
backend by default (its Vulkan path showed washed-out colors). `ICED_BACKEND=tiny-skia` draws on
the CPU instead: about a quarter of the memory, but typing and panel resizing feel laggy.

## Shortcuts

| Keys | Action |
|---|---|
| ``Ctrl+` `` | Focus the terminal |
| `Ctrl+B` … | herdr's prefix (`Ctrl+B ?` lists its keys) |
| `Ctrl+Shift+M` | Focus mode (hide the inspector) |
| `Ctrl+Shift+G` / `F` / `E` | Inspector: Changes / Search / Files (again to hide) |
| `Ctrl+Shift+B` | Toggle inspector |
| `Ctrl+Shift+T` | Cycle theme |
| `Ctrl+Z` / `Ctrl+Shift+Z`, `Ctrl+Y` | Undo / redo (editor, commit box) |
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
