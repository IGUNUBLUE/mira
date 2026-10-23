# mira

Lightweight, terminal-first workspace for agent-driven development, in Rust (iced 0.14 + alacritty_terminal 0.26, drawn by mira's own widget).
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
- **Never patch or monkey-patch dependencies.** No forks, no `[patch]`, no vendored copies of
  dependencies. Work through public APIs (wrappers, composition); fix bugs upstream with issues/PRs.
- **Don't invent.** Before using any API (herdr, iced, alacritty_terminal, git, agent CLIs) read its docs, schema or
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
  `term_write`, terminal events and actions), `follow.rs` (herdr polling, `follow`/`switch_root`, `for_root`,
  `send_to_agent`), `review.rs` (run snapshots/files/prompts, review actions, `save_state`), `documents.rs`
  (`Tab`/`FileTab`/`DiffTab`, `AutoSave`, open/save/close, diffs, `active_diff`), `scm.rs` (git ops, commit,
  `CommitGen`), `find.rs` (`SearchState`, search & replace), `navigate.rs` (find bar per file tab in
  `FileTab::find`, quick-open `QuickOpen` palette, go to line; cursor moves go through `FileTab::follow`), `keys.rs` (`on_key`; Escape only when no widget captured it, see `escape`).
  One terminal running herdr (`TermKind`), herdr polled on
  `Msg::StatusTick` (700 ms; every 3rd while unfocused), runs fed to `runs::Tracker`; Changes shows git files with their run (`Tracker::origin`, `is_new`) or grouped by run. `Msg::Tick` (2 s; every 5th tick while unfocused): git,
  explorer only while Files is visible, reload clean tabs, active diff only when its file changed (`active_diff_stale`).
  `GitLoaded(gen, ..)` drops replies of an older refresh (`git_gen`) and is a no-op when the snapshot is unchanged;
  it also sets `repo_prefix`, the explorer's `Marks`, and says whether the folder is a repo (`has_repo` never runs git). `GitDone(epoch, GitOutcome)`
  always clears `git_busy` but only touches the commit box for the current folder. Dividers (`Divider::Inspector`, `Divider::History`) persist
  `inspector_width` / `history_height`.
- `src/view.rs` — all views. Layout: pane_grid(document / terminal) | inspector (Changes · Search · Files) + status bar.
  `view` runs after every message batch (the terminal sends one per batch of PTY output): no IO, no per-row formatting or
  cloning; precompute in update/models and borrow. Diff rows are built once per diff/anchor/palette (`lazy`).
- `src/herdr.rs` — blocking client of herdr's socket (NDJSON); `launch_command` strips `HERDR_*`. Facts: `herdr-client` skill.
- `src/runs.rs` — pure run state machine (`Tracker::observe` → `Effect::Before/After`), tested without IO. A derived
  index makes `origin`/`is_new`/`new_files` O(1); runs change only through `Tracker` methods (`edit`), which reindex.
- `src/agentlog.rs` — run titles from kiro/codex/Claude Code conversation logs.
- `src/term/` — the terminal. `mod.rs`: `Terminal` (alacritty `Term` + PTY event loop), its `Listener` answers
  `PtyWrite`/color/size queries on the PTY thread without blocking and sends `Wakeup`/clipboard/exit through an
  unbounded channel drained in batches (`subscription`); `apply(Action) -> Effect` (input, resize, scroll,
  selection, links). `frame.rs`: snapshot of the visible rows only, colors resolved against `theme::p()` and the
  program's OSC overrides, cursor. `input.rs`: pure xterm key/mouse/paste encoding. `widget.rs`: the iced widget
  (keys incl. `APP_LETTERS` passed to the app, mouse modes, Shift bypasses mouse reporting, selection, wheel,
  Ctrl+click links, draw: background runs as quads, cursor, `boxdraw.rs` lines/blocks, text runs).
- `src/termio.rs` — light/dark report for herdr (`color_scheme_report`, `is_dark`).
- `src/persist.rs` — per-folder runs (`$XDG_STATE_HOME/mira/<fnv>.json`, 0600) and `~/.config/mira/config`.
- `src/git.rs` — `git` CLI wrapper (no stdin, no prompts; status scoped to opened folder, capped at `MAX_CHANGES`;
  push/pull under a timeout); tree snapshots via a temporary `GIT_INDEX_FILE` in `$XDG_STATE_HOME/mira/index` (0700),
  tree diffs, restore (refuses when the tree is gone). `snapshot` reuses a `git::Cache` (repo root; `log` reread only
  when HEAD/refs mtimes move or after 60 s) and lists ignored files only while Files is visible. Commit All and commit messages are scoped to `repo_prefix`.
- `src/diff.rs` — side-by-side model on `similar` (patience, inline emphasis, hunks with 3 lines of context); a full
  view over `MAX_FULL_ROWS` falls back to hunks. Tabs hold `Arc<FileDiff>`; `FileDiff.id` keys the cached rows.
- `src/app/settings.rs` — gear menu, `Dialog` (Settings, Shortcuts, About; drawn in the overlay layer), `Drafts`
  (what Settings shows, read when it opens: `view` never reads the config), `WindowControls` (`decorated()` in
  main.rs; `on` draws `view_title_bar`), `SHORTCUTS` (keep in sync with docs/guide.md).
- `src/app/files.rs` — Files context menu (`ExplorerMenu`, opened by `rightclick::right_click`, which reports the
  pointer in window coordinates after the row's `mouse_area` says what was clicked), in-place `NameEdit`
  (`NAME_INPUT`), trash with Undo. `src/fileops.rs` — create/rename (never replace), freedesktop home trash
  (`.trashinfo`, `restore`); tested in temp dirs.
- `src/explorer.rs` — lazy tree, only expanded dirs are read, and only again when their mtime moves; git `Marks`
  (status, ignored) are computed per snapshot and copied onto entries.
- `src/gutter.rs` — editor line numbers. iced 0.14 exposes no editor scroll, so `gutter::Scroll` mirrors
  cosmic-text's three rules (wheel, follow the cursor, clamp) with wrapping off; tested step by step against a real
  cosmic-text editor (dev-dependency). Every file-editor action must go through `FileTab::follow` (or `scrolled`
  for `Action::Scroll`); the text area height comes from a `sensor` (`Msg::EditorViewport`).
- `src/history.rs` — undo/redo for a `text_editor::Content` (iced has none): word-grouped steps storing only the
  changed span, capped (500 steps / 8 MB). File tabs and the commit box own one; `FileTab.dirty` follows it.
  Autosave (`AutoSave`, VSCodium's names) lives in app/documents.rs: `save_tab(i, manual)`; autosave skips files whose
  mtime changed under unsaved edits (`FileTab.conflict`). The window doesn't exit on close: `Msg::CloseRequested`
  saves, then `iced::exit()`.
- ChatGPT sign-in is a modal (`App::signin`, `SignInStage::Ask` → `Waiting`, app/scm.rs; drawn with `stack` +
  `opaque` in `view`): ✦ checks `commitgen::needs_sign_in()` and asks before `openai::login` opens the browser;
  focusing `DIALOG_FOCUS` (no such widget) takes the keyboard from the terminal so Escape cancels it.
- `src/git/commits.rs` / `src/app/commits.rs` — read-only History selection and affected files;
  immutable commit diffs use the first parent (empty for roots), original rename paths,
  bounded blob reads and selection IDs plus folder epochs to reject stale responses.
- `src/git/branches.rs` / `src/app/branches.rs` — cached local/remote branch picker and scoped
  Undo Last Commit. Switch uses Git without force or automatic stash; block unsaved editor tabs,
  ignored-file overwrites and other worktrees. Undo requires `confirm.rs`, preserves index/files
  (`reset --soft`), refuses known shared commits and rechecks HEAD/branch before acting. Cancel
  in-flight commit generation before either operation changes HEAD; clear stale diff tabs on switch.
- `src/git/remote.rs` — local push/pull preparation, explicit publication, guarded retries and
  failure advice. `src/app/remote.rs` owns persistent Git error/publication dialogs. Remote writes
  never retry automatically; IPv4 retry uses Git's option. Default OpenSSH uses bounded connection
  attempts and batch mode; custom commands/variants are preserved. Foreground errors must remain
  readable/copyable until dismissed; folder changes invalidate publication/retry approval.
- `src/commitgen.rs` — commit message prompt/diff budget, provider choice (`commit_provider`), streams `Event`s
  (Delta, Account, Done) to `Msg::CommitGen`; cancel restores the previous text.
- `src/openai.rs` — Responses API over `curl` with SSE streaming; ChatGPT OAuth PKCE login (codex's client,
  localhost:1455/1457), refresh, tokens in `$XDG_STATE_HOME/mira/chatgpt.json` (0600); API key from
  `OPENAI_API_KEY` or `~/.config/mira/openai_key` (must be 0600). Own SHA-256/base64url, no crypto crates.
- `src/search.rs` — `ignore` walker + `regex` (multi-line, CRLF-aware: `^`/`$` per line); replace expands `$1` only
  in regex mode and skips matches spanning lines, like the per-line results. Results keep a ~300-byte preview window;
  typing is debounced (150 ms) and a newer search cancels the running one (shared generation).
- `src/textfind.rs` — find in file: literal matches per line with byte columns (as the editor's), next/previous
  from the cursor. `src/quickopen.rs` — file list (Search's `ignore` walk, capped) and the fuzzy score/rank.
- `src/hunks.rs` — git's own hunks of one file (`git::file_patch`, patience) for per-hunk stage/unstage/discard
  via `git::apply` (patch on stdin); `FileDiff::patch`, toolbars placed at each hunk's first changed line.
- `src/files.rs` — `read_text` (size cap, NUL sniff, UTF-8) for tabs and search; `test_dir` for tests.
- `src/imagefile.rs` — image tabs (`Tab::Image`): which extensions open as images, header-only read (size and
  pixel caps); iced decodes on draw (`image-without-codecs` + our `image` with a few formats; `svg` for previews).
- `src/proc.rs` — streams a child's stdout with stdin fed from a thread, cancel and timeout (commit_command, curl).
- `src/icons.rs` — `Icon` (Nerd Font codicons + Unicode fallback) and `file_icon` (seti/devicons). Font found via `fc-list`, never bundled.
- `src/theme.rs` — the four LAGC palettes, active palette via `p()`, `Choice` (auto = follow HyDE `staterc`), style fns.

## Conventions

- Read [`design.md`](design.md) before changing UI. It defines the Rust/UI-derived visual language,
  LAGC color roles, compact geometry, component states, and visual review scenarios. The isolated
  shared primitives in `src/theme/ui.rs` are used by the product and the `examples/design_system/`
  specimen. Reuse those primitives rather than duplicating styles in individual views. Keep the
  guide, specimen, and accepted screenshots in sync; source findings are in `docs/design-reference/rust-ui-review.md`.
- Blocking work (git, search, diff, sockets) runs via `blocking()` (`spawn_blocking`) and returns a `Msg`.
- Colors only through `theme::p().<field>`; never literal colors in views (themes switch at runtime). Text uses
  `fg`, `muted` or the `warn_text`/`error_text`/`added_text` variants (4.5:1, tested); `line_no` only for gutters.
- Changes rows: `change_button` with a `hover` key reports the mouse (`Msg::HoverChange`); build row buttons with
  `row_actions` (only for the hovered/open row). Discard always snapshots first (`git::snapshot_tree`) and offers
  Undo via `status_action` (`restore_from_tree`); without a snapshot nothing is discarded.
- Destructive actions go through `app/confirm.rs`: an explicit modal with a warning, affected targets,
  Cancel selected initially and a `theme::danger` action. Approval is one-shot, tied to the folder and
  stable targets; status expiration never dismisses it. Escape cancels (`clear_confirms` also runs on
  folder switches). Icon-only buttons get a `tip`; git buttons are disabled while `git_busy`.
- Diff rows: content row is the stack base layer, backgrounds go `push_under` so wrapped lines stay aligned.
- App shortcuts are `Ctrl+Shift+<letter>`; the terminal widget leaves those in `APP_LETTERS` (term/widget.rs) to the app.
  Never take `C`/`V` (terminal copy/paste) or plain `Ctrl+B` (herdr's prefix).
- Code, comments and docs in English.

## Gotchas

- Renderer: `main.rs` defaults `WGPU_BACKEND=gl` (on some Vulkan drivers colors were gamma-encoded twice). tiny-skia saves RAM but lags on
  typing and resizing: measure latency, not only RAM/CPU, before changing the default.
  Check colors with real pixels (grim) when touching rendering, not by eye.

- Never stop, restart or delete a running herdr server or sessions. Run every herdr command meant for a test
  session with `HERDR_*` unset (see the `herdr-client` skill): inside a pane `HERDR_SOCKET_PATH` overrides
  `--session` and `XDG_CONFIG_HOME`.

- The PTY thread holds the terminal lock while it parses and calls `Listener::send_event`: never block or take
  the lock there (no bounded channel sends). Replies the program waits for (color queries, size, DA) are answered
  right there, not via the UI. The UI locks only to snapshot (`Terminal::frame`, once per change) and to apply input.
- Within one layer iced draws every quad before any text: the terminal relies on it (cursor and backgrounds never
  cover glyphs). Text runs must stay ASCII-only with the measured advance as cell width, or columns drift.
- herdr's `terminal` theme maps tokens to ANSI colors and assumes a dark terminal: `overlay1` is White (15), which
  is the background color in mira's light palettes (tab labels vanished), and the active tab is DarkGray (8) on
  Blue (4). mira reports light/dark like Ghostty (`CSI ? 997 ; 1|2 n`, `termio::color_scheme_report`) at start,
  on focus and on theme change, so herdr's `auto_switch` can apply `[theme.custom.light]`/`[theme.custom.dark]`
  overrides; the recommended ones are in docs/guide.md (checked in all four palettes). ANSI 7 is dark in the
  light palettes; the recommended overrides also keep herdr's draggable pane scrollbars visible (thumb vs track).

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

`src/term/` keeps 100 % line and function coverage (`cargo llvm-cov --summary-only`; the widget is tested on
iced's headless tiny-skia renderer and through a recording `Painter`). New code there comes with its tests.
Snapshot speed vs a full grid clone: `MIRA_BENCH=1 cargo test --release snapshot_beats -- --nocapture`.

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
