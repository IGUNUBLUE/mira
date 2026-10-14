---
name: iced-ui
description: iced 0.14 and iced_term UI patterns for mira (Elm architecture, Msg/update/view, Task, Subscription, pane_grid, terminal widget, theming, icons, shortcuts). Use when adding views, messages, panels, shortcuts or async work, or when fixing UI bugs, lag or layout in mira.
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

## Terminal (iced_term 0.8 over alacritty_terminal 0.25)
- mira sees every `iced_term::Event::BackendCall` before forwarding it (`Msg::Term`), so it can intercept writes and alacritty events.
- Known gaps in iced_term 0.8, covered from mira without patching: wheel/right/middle/Alt/bracketed paste in `termio::TermInput` (only while herdr runs), focus in/out written on `Msg::WindowFocus`, `ColorRequest`/`ClipboardStore` answered in `App::on_term_event`. Still missing: kitty keyboard protocol. Never fork or patch dependencies; contribute upstream instead.
- App shortcuts that must work while the terminal has focus are swallowed with `AddBindings` (`SHORTCUT_LETTERS` in app.rs).

## Shortcuts
- App shortcuts are `Ctrl+Shift+<letter>`; check app.rs `on_key` and the swallow list first.
- Never take `C`/`V` (terminal copy/paste). `Ctrl+B` is herdr's prefix: don't bind it while the terminal has focus.

## Memory
- Bound everything that grows (changes `MAX_CHANGES`, search results, diff size); free contents of closed tabs.
