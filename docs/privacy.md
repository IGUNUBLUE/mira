# Privacy and network

mira has **no telemetry**, and no account is required. Automatic update checks are off by default.

## Commit-message requests

Clicking ✦ generates a commit message through the provider
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

## Optional updates

**Check for updates** contacts `api.github.com/repos/IGUNUBLUE/mira/releases/latest` and, when a newer
stable release exists, downloads small signed metadata files from that repository's GitHub Releases.
**Download update** fetches its archive from GitHub's release asset infrastructure. HTTPS redirects
are allowed only to HTTPS. Requests include Mira's version, the platform's release asset name and a
cached ETag; GitHub also sees the normal IP address. No project files, paths, prompts, account tokens
or device identifier are sent. GitHub is the current distribution host, not a runtime requirement:
you can build from source and keep checks disabled.

Enable **Automatic checks** in Settings to check in the background at startup or on focus, at most
once per day. There is no timer or download in the background. **Update and restart** is explicit;
Mira verifies a pinned Ed25519 signature and signed archive/binary checksums with OpenSSL 3 before
replacing an installer-owned user binary. Other installations must use their installer/package manager.

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

To stay fully offline, use `commit_provider = "command"` with a local model, or don't click ✦. Leave automatic updates off and do not use Check for updates.
