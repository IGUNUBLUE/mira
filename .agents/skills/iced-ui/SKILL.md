---
name: iced-ui
description: iced 0.14 UI patterns for mira (Elm architecture, Msg/update/view, Task, Subscription, pane_grid, terminal widget, theming, icons, shortcuts). Use when adding views, messages, panels, shortcuts or async work, or when fixing UI bugs, lag or layout in mira.
---

# iced UI in mira

For the iced 0.14 API itself (widgets, Task, Subscription, pane_grid, theming, custom widgets) read the `iced-rs` skill and its `examples/` before writing code; don't write 0.14 code from memory. This file holds mira's own conventions and wins on conflicts.

## Architecture (Elm)
- `update(&mut self, Msg) -> Task<Msg>` must stay cheap. git/search/diff/file IO go through `blocking()` (spawn_blocking) and come back as a `Msg`.
- Async results carry an id or generation; drop results that no longer match the state (closed tab, new search, other session).
- Long-lived IO (sockets) uses `Subscription::run` / `run_with` with a stream that reconnects with backoff; keep channels bounded.
- Polling lives in the 2 s `Msg::Tick`; don't add timers without a reason.
- Growing features get their own module with `State`, `Msg`, `update`, `view`, nested as `Msg::Git(git::Msg)` and mapped with `.map(...)` / `Task::map`.
- `view` takes `&self` only and never does IO. Don't clone large strings in view.

## Views
- Colors only through `theme::p().<field>`; never literal colors (themes switch at runtime).
- Icons through `icons::Icon` / `icons::file_icon` (Nerd Font with Unicode fallback); helpers `ico`, `icon_button`, `icon_label` in view.rs.
- Diff rows: the content row is the stack base layer; backgrounds go `push_under` so wrapped lines stay aligned.
- The inspector width is state (`inspector_width`), dragged via `Divider` and saved to config.

## Terminal (mira's widget over alacritty_terminal 0.26, `src/term/`)
- `Terminal` owns the alacritty `Term` and PTY loop. Its `Listener` runs on the PTY thread with the lock held: answer queries there, never block (unbounded channel to the UI, drained in batches → `Msg::Term(Vec<Event>)`).
- The widget publishes `Msg::TermAction(Action)`; `Terminal::apply` returns an `Effect` (clipboard, open link) for the app.
- Key/mouse/paste encoding is pure in `term/input.rs` (tested); add new sequences there with a test.
- The widget reads `Terminal::frame()` (visible rows only, rebuilt when dirty); call `redraw()` after a palette change.
- Still missing: kitty keyboard protocol, IME preedit. Never fork or patch dependencies.
- App shortcuts that must work while the terminal has focus are listed in `APP_LETTERS` (term/widget.rs): the widget leaves them uncaptured.

## Shortcuts
- App shortcuts are `Ctrl+Shift+<letter>`; check app/keys.rs `on_key` and the swallow list first.
- Never take `C`/`V` (terminal copy/paste). `Ctrl+B` is herdr's prefix: don't bind it while the terminal has focus.

## Memory
- Bound everything that grows (changes `MAX_CHANGES`, search results, diff size); free contents of closed tabs.
