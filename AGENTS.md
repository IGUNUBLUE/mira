# mira

Lightweight terminal-first workspace in Rust (iced 0.14 + iced_term 0.8).

## Structure

- `src/main.rs` — CLI args, iced application setup.
- `src/app.rs` — state, `Msg`, `update`, keyboard. Sessions → turns; when a session goes quiet the
  running turn gets an "after" snapshot. Polls every 2s (`Msg::Tick`): explorer, git, env, reload clean tabs, active diff.
- `src/view.rs` — all views. Layout: sessions + turns | pane_grid(document / agent terminal) | inspector
  (Review · Git · Env · Search · Files) + status bar.
- `src/session.rs` — agent presets (only those on `$PATH`), `Session`, `Turn`, input tracking, `Status`.
- `src/persist.rs` — per-folder session state (`$XDG_STATE_HOME/mira/<fnv>.json`) and discovery of
  kiro/codex conversation ids from their on-disk session files, for resuming.
- `src/envfiles.rs` — `.env*` discovery, parsing, masking, safe edits, per-turn key changes. Values never leave this process unmasked.
- `src/git.rs` — `git` CLI wrapper (status scoped to opened folder, capped at `MAX_CHANGES`); tree snapshots via a temporary `GIT_INDEX_FILE`, tree diffs, restore.
- `src/diff.rs` — side-by-side model on `similar` (patience, inline emphasis, hunks with 3 lines of context).
- `src/explorer.rs` — lazy tree, only expanded dirs are read.
- `src/search.rs` — `ignore` walker + `regex`; replace uses `NoExpand` for literal mode.
- `src/theme.rs` — LAGC Tech Light palette and style fns.

## Shortcuts

App shortcuts are `Ctrl+Shift+<letter>` and are swallowed in the terminal (`new_session`).
Never take `C`/`V` (terminal copy/paste).

## Commands

```bash
cargo build --release
cargo test
cargo clippy
```

## Conventions

- Blocking work (git, search, diff) runs via `blocking()` (`spawn_blocking`) and returns a `Msg`.
- Diff rows: content row is the stack base layer, backgrounds go `push_under` so wrapped lines stay aligned.
- No network calls, no telemetry. New features must keep memory low; avoid heavy deps.
- Conventional Commits; no AI attribution trailers.
