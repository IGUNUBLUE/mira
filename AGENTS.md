# mira

Lightweight, terminal-first workspace for agent-driven development, in Rust (iced 0.14 + iced_term 0.8).
The user talks to coding agents (codex, kiro-cli, claude, …) in a terminal running herdr; mira is what
sits around that loop: review what the agents changed, edit, search, commit.

## Product rules (from the owner)

- **Terminal first.** The terminal is the priority; everything else supports the ask → act → review loop.
- **Light and fast.** Rust, low RAM, no Electron, no extension host. New features must keep memory low;
  avoid heavy dependencies.
- **No vendor lock-in, community-friendly.** Don't depend on a single company or service. mira itself makes
  no network calls and has no telemetry; AI features go through the user's own agent CLIs (e.g. `codex exec`)
  or a configurable local command.
- **Integrate, don't re-implement.** herdr owns agents, workspaces, panes, status and persistence; mira uses it
  through its socket API and runs it in the terminal. Don't rebuild what a tool in the stack already does well.
- **Never patch or monkey-patch dependencies.** No forks, no `[patch]`, no vendored copies of iced_term or
  others. Work through public APIs (wrappers, composition); fix bugs upstream with issues/PRs.
- **Don't invent.** Before using any API (herdr, iced, iced_term, git, agent CLIs) read its docs, schema or
  source and verify against the installed version. State what was verified and what was not.
- **Best UX, familiar where it helps.** The owner comes from VSCodium: keep its good workflows (edit, side-by-side
  diff, search & replace, AI commit messages, file icons, resizable panels) without its weight.
- **Files are files.** `.env` and other dotfiles open, show and edit in the editor like any file; no special panels.
- **Readable.** Good contrast and medium-weight fonts; never thin, low-contrast text.
- **Themes.** The LAGC palettes from `~/Projects/My-HyDE-dots` (VSCodium + kitty themes); `auto` follows HyDE.
- **Dogfooding.** mira is developed from inside mira. Never leave the user's running mira broken: test risky
  changes in an isolated instance (`XDG_CONFIG_HOME`/`XDG_STATE_HOME` in a temp dir, a named herdr session)
  before installing and restarting, and keep agents alive across restarts (herdr does this).

## Structure

- `src/main.rs` — CLI args, iced application setup.
- `src/app.rs` — state, `Msg`, `update`, keyboard. One terminal running herdr (`TermKind`), herdr polled on
  `Msg::StatusTick` (700 ms), runs fed to `runs::Tracker`; Changes shows git files with their run (`Tracker::origin`, `is_new`) or grouped by run. `Msg::Tick` (2 s): explorer, git,
  reload clean tabs, active diff. Every terminal command goes through `termio::Gate` (see Gotchas).
- `src/view.rs` — all views. Layout: pane_grid(document / terminal) | inspector (Changes · Search · Files) + status bar.
- `src/herdr.rs` — blocking client of herdr's socket (NDJSON); `launch_command` strips `HERDR_*`. Facts: `herdr-client` skill.
- `src/runs.rs` — pure run state machine (`Tracker::observe` → `Effect::Before/After`), tested without IO.
- `src/agentlog.rs` — run titles from kiro/codex conversation logs.
- `src/termio.rs` — `TermInput` wrapper (wheel, right/middle, Alt, bracketed paste), `Gate`, color-query answers.
- `src/persist.rs` — per-folder runs (`$XDG_STATE_HOME/mira/<fnv>.json`, 0600) and `~/.config/mira/config`.
- `src/git.rs` — `git` CLI wrapper (status scoped to opened folder, capped at `MAX_CHANGES`); tree snapshots via a
  temporary `GIT_INDEX_FILE`, tree diffs, restore.
- `src/diff.rs` — side-by-side model on `similar` (patience, inline emphasis, hunks with 3 lines of context).
- `src/explorer.rs` — lazy tree, only expanded dirs are read.
- `src/commitgen.rs` — commit message prompt/diff budget; runs `codex exec` or `commit_command` (prompt on stdin), cancellable.
- `src/search.rs` — `ignore` walker + `regex`; replace uses `NoExpand` for literal mode.
- `src/icons.rs` — `Icon` (Nerd Font codicons + Unicode fallback) and `file_icon` (seti/devicons). Font found via `fc-list`, never bundled.
- `src/theme.rs` — the four LAGC palettes, active palette via `p()`, `Choice` (auto = follow HyDE `staterc`), style fns.

## Conventions

- Blocking work (git, search, diff, sockets) runs via `blocking()` (`spawn_blocking`) and returns a `Msg`.
- Colors only through `theme::p().<field>`; never literal colors in views (themes switch at runtime).
- Diff rows: content row is the stack base layer, backgrounds go `push_under` so wrapped lines stay aligned.
- App shortcuts are `Ctrl+Shift+<letter>` and are swallowed in the terminal (`SHORTCUT_LETTERS` in app.rs).
  Never take `C`/`V` (terminal copy/paste) or plain `Ctrl+B` (herdr's prefix).
- Code, comments and docs in English; talk to the owner in Spanish.

## Gotchas

- Never stop, restart or delete the user's herdr server or sessions. Run every herdr command meant for a test
  session with `HERDR_*` unset (see the `herdr-client` skill): inside a pane `HERDR_SOCKET_PATH` overrides
  `--session` and `XDG_CONFIG_HOME`.

- iced_term 0.8 sends PTY events with `blocking_send` into a channel of 100 while holding the terminal lock.
  Calling `Terminal::handle` during an event burst deadlocks the UI (herdr asks for 256 colors at startup).
  Never call `handle` directly: use `App::term_send` / `term_write`, which go through `termio::Gate`.
- herdr's `terminal` theme maps tokens to ANSI colors; mira's light palettes put a dark color in ANSI 7, which
  herdr uses for some UI elements (e.g. the focused pane's scrollbar track).

## Quality gate

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
# live check against a real herdr session (see runs.rs `e2e`):
MIRA_E2E_SOCKET=… MIRA_E2E_ROOT=… cargo test e2e -- --ignored --nocapture
```

Lints live in `Cargo.toml [lints]` (no `unsafe`, no `unwrap`/`expect` outside tests); CI runs the gate.

## Commits

- Conventional Commits; message says why, not only what. No AI attribution or co-author trailers.
- Author: `Lenin Garizabalo <934933+IGUNUBLUE@users.noreply.github.com>` (pass with `git -c`, never edit git config).
- Commit only when the owner asks; never push without being asked.

## Skills

Repo-level skills live in `.agents/skills/` (installed with `npx skills`, pinned in `skills-lock.json`, update with
`npx skills update -p`); `.kiro/skills/` entries are symlinks to them.

- `rust-quality`, `iced-ui`, `herdr-client` — mira's own: quality gate, UI conventions, herdr protocol facts.
- `rust-best-practices` (apollographql), `iced-rs` (iced 0.14 API reference), `systematic-debugging` (obra/superpowers).

On conflicts, these rules and mira's own skills win.
