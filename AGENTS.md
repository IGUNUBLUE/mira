# mira

Lightweight terminal-first workspace in Rust (iced 0.14 + iced_term 0.8).

## Structure

- `src/main.rs` — CLI args, iced application setup.
- `src/app.rs` — state, `Msg`, `update`, keyboard. One terminal running herdr (`TermKind`), herdr polled on
  `Msg::StatusTick` (700 ms), runs fed to `runs::Tracker`, review actions. `Msg::Tick` (2 s): explorer, git, reload clean tabs, active diff.
- `src/view.rs` — all views. Layout: pane_grid(document / terminal) | inspector (Review · Git · Search · Files) + status bar.
- `src/herdr.rs` — blocking client of herdr's socket (NDJSON); `launch_command` strips `HERDR_*`. Facts: `herdr-client` skill.
- `src/runs.rs` — pure run state machine (`Tracker::observe` → `Effect::Before/After`), tested without IO.
- `src/agentlog.rs` — run titles from kiro/codex conversation logs.
- `src/termio.rs` — `TermInput` wrapper (wheel, right/middle, Alt, bracketed paste) and color-query answers; iced_term is used unpatched.
- `src/persist.rs` — per-folder runs (`$XDG_STATE_HOME/mira/<fnv>.json`, 0600) and `~/.config/mira/config`.
- `src/git.rs` — `git` CLI wrapper (status scoped to opened folder, capped at `MAX_CHANGES`); tree snapshots via a temporary `GIT_INDEX_FILE`, tree diffs, restore.
- `src/diff.rs` — side-by-side model on `similar` (patience, inline emphasis, hunks with 3 lines of context).
- `src/explorer.rs` — lazy tree, only expanded dirs are read.
- `src/commitgen.rs` — commit message prompt/diff budget; runs `codex exec` or `commit_command` (prompt on stdin), cancellable.
- `src/search.rs` — `ignore` walker + `regex`; replace uses `NoExpand` for literal mode.
- `src/icons.rs` — `Icon` (Nerd Font codicons + Unicode fallback) and `file_icon` (seti/devicons). Font detected via `fc-list`, never bundled.
- `src/theme.rs` — the four LAGC palettes (source: `~/Projects/My-HyDE-dots` VSCodium/kitty themes), active
  palette via `p()`, `Choice` (auto = follow HyDE `staterc`), style fns. Never hardcode colors in views: use `p().<field>`.

## Skills

Repo-level skills live in `.agents/skills/` (the `.kiro/skills/` entries are symlinks to them). Third-party ones are
pinned in `skills-lock.json` (update with `npx skills update -p`):

- `rust-quality`, `iced-ui`, `herdr-client` — mira's own: quality gate, UI conventions, herdr protocol facts.
- `rust-best-practices` (apollographql), `iced-rs` (iced 0.14 API reference), `systematic-debugging` (obra/superpowers).

On conflicts, mira's own skills and this file win.

## Shortcuts

App shortcuts are `Ctrl+Shift+<letter>` and are swallowed in the terminal (`SHORTCUT_LETTERS` in app.rs).
Never take `C`/`V` (terminal copy/paste) or plain `Ctrl+B` (herdr's prefix).

## Commands

```bash
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
# live check against a real herdr session (see runs.rs `e2e`):
MIRA_E2E_SOCKET=… MIRA_E2E_ROOT=… cargo test e2e -- --ignored --nocapture
```

## Conventions

- Blocking work (git, search, diff) runs via `blocking()` (`spawn_blocking`) and returns a `Msg`.
- Diff rows: content row is the stack base layer, backgrounds go `push_under` so wrapped lines stay aligned.
- mira itself makes no network calls and has no telemetry (AI features go through the user's agent CLIs). New features must keep memory low; avoid heavy deps.
- Conventional Commits; no AI attribution trailers.
