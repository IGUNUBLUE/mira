---
name: herdr-client
description: Facts for building mira's client of the herdr socket API (paths, NDJSON protocol, agent status, events, persistence, terminal requirements, keybindings). Use when writing or debugging mira code that launches herdr, talks to its socket, or reacts to agent status. Not for driving herdr panes as an agent (that is the global herdr skill).
---

# herdr client facts (verified against herdr 0.9.3 binary and v0.9.5 docs/source, protocol 22)

Docs index: https://herdr.dev/llms.txt. Raw pages: https://raw.githubusercontent.com/herdrdev/herdr/v0.9.5/docs/next/website/src/content/docs/<page>.mdx (socket-api, session-state, concepts, agents, keyboard, configuration). Schema: `herdr api schema --json` (static, safe). Never run bare `herdr` from scripts (it launches the TUI); never stop the user's server.

## Safety (learned the hard way)
- Inside a herdr pane, `HERDR_SOCKET_PATH` points at the user's real server and **wins over `--session` and
  `XDG_CONFIG_HOME`**. A `herdr session stop <test>` or `herdr server stop` run from there stops the user's
  server and kills every pane and agent. For any test session run herdr through
  `env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_SESSION -u HERDR_PANE_ID -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID`,
  and never run stop/delete commands without it.
- Verified: closing mira (SIGTERM or closing the window) leaves the herdr server and its agents running.

## Facts
- herdr is a Rust binary only (no lib crate). Integrate as a program in the terminal + a hand-written socket client.
- Socket: `~/.config/herdr/herdr.sock` (default session), `~/.config/herdr/sessions/<name>/herdr.sock` (named). Resolution: `--session`, `HERDR_SOCKET_PATH`, `HERDR_SESSION`, default. Owner-only. A stale file can exist: connection refused = not running.
- Panes get `HERDR_ENV=1`, `HERDR_SOCKET_PATH`, `HERDR_*_ID`. Strip `HERDR_*` when mira spawns herdr.
- Protocol: newline-delimited JSON over a unix socket. `{"id","method","params"}` → `{"id","result"}` or `{"id","error":{"code","message"}}`. `params` always present (`{}`). Ignore unknown fields.
- Useful methods: `ping`, `session.snapshot`, `pane.list`, `pane.get {pane_id}`, `agent.list`, `events.subscribe`, `events.wait`.
- `AgentStatus`: `idle | working | blocked | done | unknown`. `idle` and `done` both mean ready for input (done = not yet seen). Panes report `cwd` and `foreground_cwd`; agents have `completion_seq` (changes when a working period completes).
- `pane.agent_status_changed` subscriptions need a `pane_id` (no global subscription); the pushed event has no cwd. Falling behind → `events_lost` error and close; recover by resubscribing and reconciling with `session.snapshot`.
- mira polls `agent.list` every 700 ms (one short connection per request) and detects `working|blocked → idle|done` or a `completion_seq` change (`runs.rs`). Verified live with kiro: `completion_seq` is null while working and set once done.
- `pane.send_text` types without Enter; `workspace.create {cwd,label,focus:true}` focuses the new workspace; killing a client (SIGHUP) leaves the server and its panes running.
- Workspaces have no cwd (`workspace.list`: id, number, label, focused, counts, optional `worktree`). The folder
  comes from panes: `pane.list` marks exactly one pane `focused: true` server-wide (in the focused workspace).
  Verified on 0.9.5.
- Persistence: detaching the client keeps processes running. A server restart restores layout and shells; agents resume only through native integrations (`herdr integration install codex|claude|...`). Kiro has state detection but no native resume.
- Multiple clients can attach; when two view the same tab, the last one to interact sets pane sizes.
- Keys: prefix `ctrl+b` by default (`[keys] prefix` in `~/.config/herdr/config.toml`). Ctrl+Space prefix doesn't reach herdr through iced_term 0.8.
- Theme: `[theme] name = "terminal"` makes herdr use ANSI colors, so mira's palette applies. herdr asks the host for colors with OSC 10/11/4 (iced_term drops `ColorRequest`; mira must answer it).
- Light/dark: herdr parses `CSI ? 997 ; 1 n` (dark) / `; 2 n` (light) from its input at any time (`RawInputEvent::HostColorSchemeChanged`, explicit, wins over OSC 11 inference) and, with `auto_switch = true`, uses `dark_name`/`light_name` plus `[theme.custom.dark|light]` on top of `[theme.custom]`. It queries `CSI ? 996 n` and OSC 10/11 on focus-in; alacritty_terminal answers neither 996 nor mode 2031, so mira types the report itself. Palette tokens (herdr `Palette::terminal`): text Reset, overlay0/subtext0 Gray(7), overlay1 White(15), surface1/surface_dim/active_row_bg DarkGray(8), accent Blue(4); tabs: inactive overlay0, labelled overlay1, active `surface_dim` on accent. Verified in v0.9.5 source.
- Notifications: `[ui.toast] delivery = "system"` uses notify-send; pick one owner (herdr or mira) to avoid duplicates.
