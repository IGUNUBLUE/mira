---
name: herdr-client
description: Facts for building mira's client of the herdr socket API (paths, NDJSON protocol, agent status, events, persistence, terminal requirements, keybindings). Use when writing or debugging mira code that launches herdr, talks to its socket, or reacts to agent status. Not for driving herdr panes as an agent (that is the global herdr skill).
---

# herdr client facts (verified against herdr 0.9.3 binary and v0.9.5 docs/source, protocol 22)

Docs index: https://herdr.dev/llms.txt. Raw pages: https://raw.githubusercontent.com/herdrdev/herdr/v0.9.5/docs/next/website/src/content/docs/<page>.mdx (socket-api, session-state, concepts, agents, keyboard, configuration). Schema: `herdr api schema --json` (static, safe). Never run bare `herdr` from scripts (it launches the TUI); never stop the user's server.

## Facts
- herdr is a Rust binary only (no lib crate). Integrate as a program in the terminal + a hand-written socket client.
- Socket: `~/.config/herdr/herdr.sock` (default session), `~/.config/herdr/sessions/<name>/herdr.sock` (named). Resolution: `--session`, `HERDR_SOCKET_PATH`, `HERDR_SESSION`, default. Owner-only. A stale file can exist: connection refused = not running.
- Panes get `HERDR_ENV=1`, `HERDR_SOCKET_PATH`, `HERDR_*_ID`. Strip `HERDR_*` when mira spawns herdr.
- Protocol: newline-delimited JSON over a unix socket. `{"id","method","params"}` → `{"id","result"}` or `{"id","error":{"code","message"}}`. `params` always present (`{}`). Ignore unknown fields.
- Useful methods: `ping`, `session.snapshot`, `pane.list`, `pane.get {pane_id}`, `agent.list`, `events.subscribe`, `events.wait`.
- `AgentStatus`: `idle | working | blocked | done | unknown`. `idle` and `done` both mean ready for input (done = not yet seen). Panes report `cwd` and `foreground_cwd`; agents have `completion_seq` (changes when a working period completes).
- `pane.agent_status_changed` subscriptions need a `pane_id` (no global subscription); the pushed event has no cwd. Falling behind → `events_lost` error and close; recover by resubscribing and reconciling with `session.snapshot`.
- Simplest robust approach: poll `agent.list` on the 2 s tick, one short connection per request, detect `working|blocked → idle|done` or a `completion_seq` change.
- Persistence: detaching the client keeps processes running. A server restart restores layout and shells; agents resume only through native integrations (`herdr integration install codex|claude|...`). Kiro has state detection but no native resume.
- Multiple clients can attach; when two view the same tab, the last one to interact sets pane sizes.
- Keys: prefix `ctrl+b` by default (`[keys] prefix` in `~/.config/herdr/config.toml`). Ctrl+Space prefix doesn't reach herdr through iced_term 0.8.
- Theme: `[theme] name = "terminal"` makes herdr use ANSI colors, so mira's palette applies. herdr asks the host for colors with OSC 10/11/4 (iced_term drops `ColorRequest`; mira must answer it).
- Notifications: `[ui.toast] delivery = "system"` uses notify-send; pick one owner (herdr or mira) to avoid duplicates.
