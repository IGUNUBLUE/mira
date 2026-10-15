# Why mira

## The loop changed

Most of my coding now happens in conversation with agents (Codex, Claude Code, Kiro) running in
a terminal. I ask, the agent edits files, I look at what it did, I correct it or accept it, and
I commit. The terminal is where the work happens. Everything else is there to support the
*look* step.

For that step I kept a full editor open, VSCodium in my case, almost only to review diffs and
write commit messages. It worked, but it never fit:

- **It was the heaviest thing on my machine.** An Electron app and its extension host used more
  memory than the agents themselves.
- **It didn't know about agents.** Git shows *what* changed, not *who*. With two or three agents
  working in the same repository, I couldn't tell which one changed a file, or what I had
  asked it to do.
- **Feedback went the long way.** To comment on a line I copied the path and line number,
  switched to the right terminal, and pasted.
- **The terminal was a side panel.** The tool I spent the day in was the smallest thing on screen.

## What I wanted instead

A small native app where the terminal is the center and the review sits around it:

- the agents' changes, labelled with the agent and the prompt that caused them;
- a side-by-side diff where clicking a line sends a comment straight to the agent that wrote it;
- the editor basics I use every day (source control, search & replace, an editor with undo and
  autosave, file icons), and nothing I don't;
- commit messages from my ChatGPT subscription, streamed into the commit box, as I had with my
  [VSCodium extension](https://github.com/IGUNUBLUE/chatgpt-commit-gen);
- fast to start, light on memory, smooth on the GPU, no telemetry.

## Principles

- **Integrate, don't rebuild.** [herdr](https://herdr.dev) already manages agent sessions,
  workspaces and status, and keeps agents alive when the window closes. mira runs it in its
  terminal and reads its socket API. Git operations call your own `git`, so hooks, signing and
  credentials behave as they do in your shell.
- **No lock-in.** No account is required, there is no telemetry, and the AI features are
  optional and replaceable (a ChatGPT subscription, an API key, or a local model).
- **Never patch dependencies.** Gaps in the libraries mira uses are worked around through their
  public APIs and reported upstream.
- **Familiar where it helps.** People coming from VS Code / VSCodium should find the
  workflows they already know.

## The name

*Mira* means "look!" in Spanish, which is the step mira is for. In Latin it means "wonderful",
as in the variable star Mira.
