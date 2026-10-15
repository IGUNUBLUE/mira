# mira

Lightweight, terminal-first workspace for agent-driven development, in Rust (iced 0.14 + iced_term 0.8).
The user talks to coding agents (codex, kiro-cli, claude, …) in a terminal running herdr; mira is what
sits around that loop: review what the agents changed, edit, search, commit.

## Product rules

- **Terminal first.** The terminal is the priority; everything else supports the ask → act → review loop.
- **Light and fast.** Rust, low RAM, no Electron, no extension host. New features must keep memory low;
  avoid heavy dependencies.
- **No vendor lock-in, community-friendly.** Don't depend on a single company or service. No telemetry. mira's
  only network access is commit-message generation, only when the user clicks ✦, only to the provider they chose
  (ChatGPT subscription, OpenAI API key) and only the next commit's diff and `--stat`; a local `commit_command`
  keeps it fully offline. Network goes through `curl` (no HTTP/TLS crates), secrets on curl's stdin.
- **Integrate, don't re-implement.** herdr owns agents, workspaces, panes, status and persistence; mira uses it
  through its socket API and runs it in the terminal. Don't rebuild what a tool in the stack already does well.
- **Never patch or monkey-patch dependencies.** No forks, no `[patch]`, no vendored copies of iced_term or
  others. Work through public APIs (wrappers, composition); fix bugs upstream with issues/PRs.
- **Don't invent.** Before using any API (herdr, iced, iced_term, git, agent CLIs) read its docs, schema or
  source and verify against the installed version. State what was verified and what was not.
- **Best UX, familiar where it helps.** Many users come from VSCodium / VS Code: keep their good workflows (edit, side-by-side
  diff, search & replace, AI commit messages, file icons, resizable panels) without its weight.
- **Files are files.** `.env` and other dotfiles open, show and edit in the editor like any file; no special panels.
- **Readable.** Good contrast and medium-weight fonts; never thin, low-contrast text.
- **Themes.** The four LAGC palettes (the author's VSCodium + kitty themes); `auto` follows HyDE when it is installed.
- **Dogfooding.** mira is developed from inside mira. Never leave a running mira broken: test risky
  changes in an isolated instance (`XDG_CONFIG_HOME`/`XDG_STATE_HOME` in a temp dir, a named herdr session)
  before installing and restarting, and keep agents alive across restarts (herdr does this).

## Structure

- `src/main.rs` — CLI args, iced application setup. `app::Start`: a folder, or `Follow` (launcher, cwd = home).
- Folder follows herdr: `poll_herdr` also reads `pane.list`; when the focused workspace changes, `App::follow` →
  `switch_root` (git, explorer, runs, search, commit box reset; diff tabs close). Async results tied to a folder go
  through `for_root` (`Msg::ForRoot(epoch, ..)`) so a reply for the old folder is dropped.
- `src/app/` — state and update logic. `mod.rs` holds `App`, the flat `Msg` (view.rs uses `crate::app::*`;
  child types are re-exported), `new`, `subscription`, `Tick`, `CloseRequested` and `update`, a dispatcher with no
  wildcard that hands each `Msg` group to a child module's `update_<module>`. Each child adds an `impl App` block
  (helpers shared across children are `pub(super)`):
  `layout.rs` (inspector, focus mode, dividers, window, theme, `sync_center`), `terminal.rs` (`TermKind`, spawn,
  `term_send`/`term_write`, `SHORTCUT_LETTERS`), `follow.rs` (herdr polling, `follow`/`switch_root`, `for_root`,
  `send_to_agent`), `review.rs` (run snapshots/files/prompts, review actions, `save_state`), `documents.rs`
  (`Tab`/`FileTab`/`DiffTab`, `AutoSave`, open/save/close, diffs, `active_diff`), `scm.rs` (git ops, commit,
  `CommitGen`), `find.rs` (`SearchState`, search & replace), `keys.rs` (`on_key`; Escape only when no widget captured it, see `escape`).
  One terminal running herdr (`TermKind`), herdr polled on
  `Msg::StatusTick` (700 ms; every 3rd while unfocused), runs fed to `runs::Tracker`; Changes shows git files with their run (`Tracker::origin`, `is_new`) or grouped by run. `Msg::Tick` (2 s; every 5th tick while unfocused): git,
  explorer only while Files is visible, reload clean tabs, active diff only when its file changed (`active_diff_stale`).
  `GitLoaded(gen, ..)` drops replies of an older refresh (`git_gen`) and is a no-op when the snapshot is unchanged;
  it also sets `repo_prefix`, the explorer's `Marks`, and says whether the folder is a repo (`has_repo` never runs git). `GitDone(epoch, GitOutcome)`
  always clears `git_busy` but only touches the commit box for the current folder. Dividers (`Divider::Inspector`, `Divider::History`) persist
  `inspector_width` / `history_height`. Every terminal command goes through `termio::Gate` (see Gotchas).
- `src/view.rs` — all views. Layout: pane_grid(document / terminal) | inspector (Changes · Search · Files) + status bar.
  `view` runs after every message batch (the terminal sends one per PTY event): no IO, no per-row formatting or
  cloning; precompute in update/models and borrow. Diff rows are built once per diff/anchor/palette (`lazy`).
- `src/herdr.rs` — blocking client of herdr's socket (NDJSON); `launch_command` strips `HERDR_*`. Facts: `herdr-client` skill.
- `src/runs.rs` — pure run state machine (`Tracker::observe` → `Effect::Before/After`), tested without IO. A derived
  index makes `origin`/`is_new`/`new_files` O(1); runs change only through `Tracker` methods (`edit`), which reindex.
- `src/agentlog.rs` — run titles from kiro/codex conversation logs.
- `src/termio.rs` — `TermInput` wrapper (wheel, right/middle, Alt, bracketed paste), `Gate`, color-query answers.
- `src/persist.rs` — per-folder runs (`$XDG_STATE_HOME/mira/<fnv>.json`, 0600) and `~/.config/mira/config`.
- `src/git.rs` — `git` CLI wrapper (no stdin, no prompts; status scoped to opened folder, capped at `MAX_CHANGES`;
  push/pull under a timeout); tree snapshots via a temporary `GIT_INDEX_FILE` in `$XDG_STATE_HOME/mira/index` (0700),
  tree diffs, restore (refuses when the tree is gone). `snapshot` reuses a `git::Cache` (repo root; `log` reread only
  when HEAD/refs mtimes move or after 60 s) and lists ignored files only while Files is visible. Commit All and commit messages are scoped to `repo_prefix`.
- `src/diff.rs` — side-by-side model on `similar` (patience, inline emphasis, hunks with 3 lines of context); a full
  view over `MAX_FULL_ROWS` falls back to hunks. Tabs hold `Arc<FileDiff>`; `FileDiff.id` keys the cached rows.
- `src/explorer.rs` — lazy tree, only expanded dirs are read, and only again when their mtime moves; git `Marks`
  (status, ignored) are computed per snapshot and copied onto entries.
- `src/history.rs` — undo/redo for a `text_editor::Content` (iced has none): word-grouped steps storing only the
  changed span, capped (500 steps / 8 MB). File tabs and the commit box own one; `FileTab.dirty` follows it.
  Autosave (`AutoSave`, VSCodium's names) lives in app/documents.rs: `save_tab(i, manual)`; autosave skips files whose
  mtime changed under unsaved edits (`FileTab.conflict`). The window doesn't exit on close: `Msg::CloseRequested`
  saves, then `iced::exit()`.
- `src/commitgen.rs` — commit message prompt/diff budget, provider choice (`commit_provider`), streams `Event`s
  (Status, Delta, Account, Done) to `Msg::CommitGen`; cancel restores the previous text.
- `src/openai.rs` — Responses API over `curl` with SSE streaming; ChatGPT OAuth PKCE login (codex's client,
  localhost:1455/1457), refresh, tokens in `$XDG_STATE_HOME/mira/chatgpt.json` (0600); API key from
  `OPENAI_API_KEY` or `~/.config/mira/openai_key` (must be 0600). Own SHA-256/base64url, no crypto crates.
- `src/search.rs` — `ignore` walker + `regex` (multi-line, CRLF-aware: `^`/`$` per line); replace expands `$1` only
  in regex mode and skips matches spanning lines, like the per-line results. Results keep a ~300-byte preview window;
  typing is debounced (150 ms) and a newer search cancels the running one (shared generation).
- `src/files.rs` — `read_text` (size cap, NUL sniff, UTF-8) for tabs and search; `test_dir` for tests.
- `src/proc.rs` — streams a child's stdout with stdin fed from a thread, cancel and timeout (commit_command, curl).
- `src/icons.rs` — `Icon` (Nerd Font codicons + Unicode fallback) and `file_icon` (seti/devicons). Font found via `fc-list`, never bundled.
- `src/theme.rs` — the four LAGC palettes, active palette via `p()`, `Choice` (auto = follow HyDE `staterc`), style fns.

## Conventions

- Blocking work (git, search, diff, sockets) runs via `blocking()` (`spawn_blocking`) and returns a `Msg`.
- Colors only through `theme::p().<field>`; never literal colors in views (themes switch at runtime). Text uses
  `fg`, `muted` or the `warn_text`/`error_text`/`added_text` variants (4.5:1, tested); `line_no` only for gutters.
- Destructive actions are two-step (`set_confirm`, `theme::danger` label, cleared with the status or Escape:
  `clear_confirms`); icon-only buttons get a `tip`; git buttons are disabled while `git_busy`.
- Diff rows: content row is the stack base layer, backgrounds go `push_under` so wrapped lines stay aligned.
- App shortcuts are `Ctrl+Shift+<letter>` and are swallowed in the terminal (`SHORTCUT_LETTERS` in app/terminal.rs).
  Never take `C`/`V` (terminal copy/paste) or plain `Ctrl+B` (herdr's prefix).
- Code, comments and docs in English.

## Gotchas

- Renderer: `main.rs` defaults `WGPU_BACKEND=gl` (on some Vulkan drivers colors were gamma-encoded twice). tiny-skia saves RAM but lags on
  typing and resizing: measure latency, not only RAM/CPU, before changing the default.
  Check colors with real pixels (grim) when touching rendering, not by eye.

- Never stop, restart or delete a running herdr server or sessions. Run every herdr command meant for a test
  session with `HERDR_*` unset (see the `herdr-client` skill): inside a pane `HERDR_SOCKET_PATH` overrides
  `--session` and `XDG_CONFIG_HOME`.

- iced_term 0.8 sends PTY events with `blocking_send` into a channel of 100 while holding the terminal lock.
  Calling `Terminal::handle` during an event burst deadlocks the UI (herdr asks for 256 colors at startup).
  Never call `handle` directly: use `App::term_send` / `term_write`, which go through `termio::Gate`.
- herdr's `terminal` theme maps tokens to ANSI colors and assumes a dark terminal: `overlay1` is White (15), which
  is the background color in mira's light palettes (tab labels vanished), and the active tab is DarkGray (8) on
  Blue (4). mira reports light/dark like Ghostty (`CSI ? 997 ; 1|2 n`, `termio::color_scheme_report`) at start,
  on focus and on theme change, so herdr's `auto_switch` can apply `[theme.custom.light]`/`[theme.custom.dark]`
  overrides; the recommended ones are in docs/guide.md (checked in all four palettes). ANSI 7 is dark in the
  light palettes, which herdr uses for the focused pane's scrollbar track (`pane_scrollbars = false`).

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
- Never edit git config; commit only when asked, never push without being asked.

## Skills

Repo-level skills live in `.agents/skills/`; `.kiro/skills/` entries are symlinks to them. mira's own skills are
tracked; third-party ones are pinned in `skills-lock.json` and restored with `npx skills experimental_install`
(update with `npx skills update -p`).

- `rust-quality`, `iced-ui`, `herdr-client` — mira's own: quality gate, UI conventions, herdr protocol facts.
- `rust-best-practices` (apollographql), `iced-rs` (iced 0.14 API reference), `systematic-debugging` (obra/superpowers).

On conflicts, these rules and mira's own skills win.
