# Privacy and network

mira has **no telemetry**, no update checks, and no account is required.

## The one network request

mira only goes online when you click ✦ to generate a commit message, and only to the provider
you configured:

| Provider | Endpoint |
|---|---|
| `chatgpt` | `chatgpt.com/backend-api/codex/responses` (sign-in at `auth.openai.com`) |
| `openai` | `api.openai.com/v1/responses` |
| `command` | none from mira; whatever your command does |

What is sent: the instructions for writing a Conventional Commit, the `git diff --stat` of
the next commit (capped at about 4,000 characters) and its diff (capped at 12,000 characters,
lockfiles and minified files left out). Only files in the opened folder are included. Nothing
else (other files, paths outside the repository, settings) is sent.

Requests are made with your system's `curl`, so mira ships no HTTP or TLS code of its own.
Tokens and keys reach `curl` on its standard input, never on its command line, where other
processes could read them.

## ChatGPT sign-in

mira asks first: a dialog explains these steps before anything opens. Signing in uses OAuth with PKCE in your browser, through the public client that Codex CLI uses,
with a callback on `localhost:1455` (or `1457`). Tokens are stored in
`~/.local/state/mira/chatgpt.json` with `0600` permissions and refreshed when needed. The
account icon signs out and deletes them. The endpoint behind the subscription is undocumented
and may change.

## On your machine

- Agent turns, including the prompts you typed, are kept per folder in
  `~/.local/state/mira/` (`0600`), never inside the project.
- Prompts are read from the agents' own local logs (`~/.codex/sessions`, `~/.kiro/sessions`,
  `~/.claude/projects`)
  to title the turns. They are never sent anywhere.
- Snapshots are git objects in your repository; they are not committed or pushed.
- The terminal never answers clipboard read requests (OSC 52) from programs running in it.

To stay fully offline, use `commit_provider = "command"` with a local model, or don't click ✦.
