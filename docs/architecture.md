# Architecture

mira is a single Rust binary built on [iced](https://iced.rs) 0.14 (GPU rendering through
wgpu) and [alacritty_terminal](https://crates.io/crates/alacritty_terminal) 0.26 (Alacritty's
terminal emulation and PTY), drawn by mira's own terminal widget. It follows the Elm architecture: state in `App`, one `Msg` enum, `update` changes
the state, `view` draws it.

```text
┌───────────────────────── mira ─────────────────────────┐
│  terminal (src/term) ─── runs ──▶ herdr client         │
│        ▲                              │                │
│        │ socket API (NDJSON)          ▼                │
│  App ◀── polls agents, panes ── herdr server ── agents │
│   │                                                    │
│   ├─ git CLI: status, log, snapshots (temp index)      │
│   ├─ runs::Tracker: agent turns ⇄ changed files        │
│   └─ views: Changes · Search · Files · diff · editor   │
└────────────────────────────────────────────────────────┘
```

## Main ideas

- **herdr is the agent layer.** mira doesn't manage agent sessions. It runs a herdr client in
  its terminal and polls herdr's socket for agents and panes: which agents work in this folder,
  their status, and which workspace is focused.
- **Turns become snapshots.** `runs.rs` is a pure state machine: when an agent starts or
  finishes a working period, it asks for a snapshot. `git.rs` writes the working tree to a git
  tree object through a temporary index (nothing committed). The diff between the before and
  after trees is the turn's set of files.
- **Blocking work runs off the UI thread.** git, search, diffs and socket calls run in
  `spawn_blocking` and come back as messages. Results for a folder mira has since left are
  dropped (an epoch counter), as are stale git refreshes and searches (generation counters).
- **`view` is cheap.** iced rebuilds the view after every batch of messages, and the terminal
  sends one per batch of output, so the view only reads precomputed data. Diff rows are cached
  with `lazy`.
- **The terminal is mira's.** alacritty_terminal parses on its own thread; what the program
  asks of the terminal is answered there without blocking, and the UI only snapshots the
  visible rows when something changed. The widget draws backgrounds and the cursor as quads
  and text in runs, so the cursor shows on lines with their own background (Codex's prompt).
- **No patched dependencies.** Bugs in dependencies are fixed upstream, never with forks.

## Source map

| Path | Role |
|---|---|
| `src/main.rs` | CLI, window settings, renderer choice |
| `src/app/` | State and update logic, one module per area (`layout`, `terminal`, `follow`, `review`, `documents`, `scm`, `find`, `keys`) |
| `src/view.rs` | All views |
| `src/herdr.rs` | herdr socket client |
| `src/runs.rs` | Agent turns, tested without IO |
| `src/git.rs` | git CLI wrapper, snapshots, tree diffs |
| `src/diff.rs` | Side-by-side diff model (`similar`) |
| `src/history.rs` | Undo/redo for the editor |
| `src/search.rs` | Search & replace (`ignore` + `regex`) |
| `src/explorer.rs` | File tree |
| `src/commitgen.rs`, `src/openai.rs` | Commit messages: prompt, providers, streaming over `curl` |
| `src/term/` | Terminal: session, screen snapshot, key/mouse encoding, widget |
| `src/termio.rs` | Light/dark report for herdr |
| `src/agentlog.rs` | Prompt titles from agents' logs |
| `src/persist.rs` | Config file and per-folder state |
| `src/theme.rs`, `src/icons.rs` | Palettes, styles, Nerd Font icons |
| `src/proc.rs`, `src/files.rs` | Child-process streaming, text file reading |

[AGENTS.md](../AGENTS.md) has the detailed notes, conventions and gotchas. It is written for
both human contributors and coding agents working on mira.
