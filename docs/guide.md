# User guide

![The window: terminal on the left, inspector on the right](images/mira.png)

mira has three parts: the **terminal** (center), the **inspector** on the right (Changes,
Search, Files) and the **document area**, which opens above the terminal when you open a
file or a diff. The status bar shows the branch, new changes and the theme.

## Folders and herdr workspaces

mira never asks for a folder; [herdr](https://herdr.dev) workspaces decide.

- `mira <folder>`, or `mira` run from a project directory, opens that folder and focuses its
  herdr workspace, creating one if needed.
- From the app launcher, mira opens the folder of herdr's focused workspace (or the last
  folder, until herdr answers).
- After that mira **follows herdr**: switch workspace in herdr and Changes, Files, Search and
  history switch with it. Open file tabs stay; diff tabs close.

Without herdr installed, mira runs your shell and shows a note in the terminal header. Set
`terminal = "shell"` to choose that on purpose (the note goes away).

## The terminal

The terminal runs herdr (`Ctrl+B` is herdr's prefix; `Ctrl+B ?` lists its keys). Agents keep
running in herdr when mira closes and are there again when it opens.

- The header lists the agents working in this folder with their status. Click one to jump
  to its pane, even in another workspace.
- "need you" in the status bar jumps to an agent that is waiting for you.
- Text is copied on selection; the mouse wheel, right/middle clicks and Alt+key reach herdr.
- **Scrolling long output** (an agent's diff, a test run): drag the scrollbar at the right
  edge of the pane, or click its track to jump. From the keyboard, `Ctrl+B [` enters herdr's
  copy mode: `PageUp`/`PageDown`, `Ctrl+U`/`Ctrl+D`, `/` to search, `q` to leave.
- `Ctrl+Shift+V` pastes (bracketed paste).

### herdr settings

These go in `~/.config/herdr/config.toml`. They make herdr draw with mira's colors and stay
readable in light and dark palettes (herdr's `terminal` theme alone assumes a dark terminal):

```toml
[theme]
name = "terminal"
auto_switch = true      # mira tells herdr whether its palette is light or dark
dark_name = "terminal"
light_name = "terminal"

[theme.custom.light]
overlay0 = "darkgray"   # tabs, and the scrollbar track
overlay1 = "gray"       # named tabs, the right side of the tab bar, the scrollbar thumb
surface_dim = "white"   # text of the active tab
active_row_bg = "white"

[theme.custom.dark]
overlay1 = "blue"       # stands out from the track
surface_dim = "black"
active_row_bg = "darkgray"

[ui]
pane_scrollbars = true  # draggable scrollbars (herdr's default)

[ui.toast]
delivery = "system"     # notifications (mira doesn't send its own)
```

Then restart mira (or choose `reload config` in herdr's menu).

## Changes: what the agents did

![Changes grouped by agent run](images/runs.png)

The Changes panel is your git working tree, and each file says **which agent changed it and
when** (`codex · 2m`; hover to see what you asked). While an agent works, mira takes snapshots
of the working tree before and after each of its turns. Snapshots use a temporary git index:
nothing is committed and your index is not touched.

- Files changed since you last opened them get a dot; the tab and the status bar count them.
  `✓` marks them all as seen.
- **By agent run** lists each turn with its files, titled with your prompt (read from the
  agents' own logs; Codex and Kiro are supported). You can stage or revert a whole turn.
  Revert asks for a second click.
- Turns that changed nothing don't show up. A quick follow-up during a turn stays part of it.

## Diffs and comments

Click a file to open a side-by-side diff with highlights inside changed lines. `‹ ›` steps
through the files of a turn. Very large files are shown as hunks, or not diffed when they
are over 8 MB.

Click a line, type a comment and send it. It is **typed into the pane of the agent that
changed that file** (through herdr, whichever pane has focus), prefixed with `path:line`,
ready for you to review and press Enter.

## Source control

Stage, unstage, commit, push and pull. History is under the Changes list; drag the divider to
resize it.

- **Row buttons** appear on hover (or on the open file), as in VSCodium: open the file, discard
  its changes, stage or unstage it. Section headers have the same for every file, plus
  **Review** (⇄), which opens the first diff; `‹ ›` in the diff then walks through the section.
- **Discard is undoable.** mira snapshots the working tree first (the same snapshots it takes
  around agent turns), so one file discards with one click and the status bar offers **Undo**.
  Discard all asks for a second click. New (untracked) files are deleted, and come back with Undo.
- The "N new" count only counts files that are still in Changes and you haven't opened since an
  agent changed them.

- With nothing staged, **Commit All** stages every listed change of this folder (the button
  shows how many; untracked files are counted in its tooltip).
- Merge conflicts get their own section and block the commit until you fix and stage them.
- A folder that isn't a repository offers **Initialize repository**.
- git runs as your own `git`, so hooks, signing and credential helpers behave as in your
  terminal. Errors show the line that matters, with the full output in a tooltip.

## Commit messages

Click ✦ next to the commit box. mira writes a [Conventional Commits](https://www.conventionalcommits.org)
message from the staged diff (or from all changes, when nothing is staged). Lockfiles are left
out and the diff is capped at 12k characters. The text streams into the box; click ✦ again to
stop and get back what you had. `Ctrl+Z` after it finishes also brings that back.

Pick a provider with `commit_provider` in the [config](configuration.md):

| Provider | What it uses |
|---|---|
| `chatgpt` | Your ChatGPT subscription. The first time, a dialog explains the sign-in and asks before opening your browser (the same flow as `codex login`); if the browser doesn't open, copy the link from the dialog. The account icon next to Pull/Push signs out (two clicks). |
| `openai` | An API key from `OPENAI_API_KEY` or `~/.config/mira/openai_key` (must be `chmod 600`). |
| `command` | `commit_command`: any command that reads the prompt on stdin and prints the message, for example a local model with ollama. Its output streams too. |
| `auto` (default) | The ChatGPT session if you are signed in, else an API key, else `commit_command`, else asks you to sign in. |

The ChatGPT subscription endpoint is undocumented and may change; an API key or a local
command are the stable options. See [privacy](privacy.md) for exactly what is sent.

## Editor, search and files

- **Editor**: syntax highlighting and line numbers (the cursor's line is highlighted), `Ctrl+S`
  to save, `Ctrl+W` or middle-click to close a tab. Long lines don't wrap; the view follows the
  cursor sideways.
  Tabs without unsaved edits reload when an agent changes the file.
- **Undo / redo**: `Ctrl+Z`, `Ctrl+Shift+Z` or `Ctrl+Y`, in the editor and the commit box, one
  step per word as in VS Code. A reload caused by an agent is undoable too.
- **Autosave** is on: files save 1 s after you stop typing, and when you leave the window,
  close a tab or quit. It never overwrites a file that changed on disk under your edits (for
  example, an agent wrote it): it pauses for that tab and tells you; `Ctrl+S` overwrites.
- **Search & replace** respects `.gitignore`, with match case, whole word and regex options.
  Replace all works line by line, exactly like the results show, and asks for a second click.
- **Files** shows the tree with git status colors; ignored files are dimmed.

## Look

Four palettes, LAGC Tech and Calm, each light and dark. The default, `auto`, follows the
active [HyDE](https://github.com/HyDE-Project/HyDE) theme when HyDE is installed, else LAGC
Tech Light. `Ctrl+Shift+T` or the status bar cycles through them.

Icons come from any installed [Nerd Font](https://www.nerdfonts.com); without one mira uses
plain symbols. Fonts default to Inter and JetBrains Mono (medium weight); see
[configuration](configuration.md) to change them. Panel sizes are remembered.

## Shortcuts

| Keys | Action |
|---|---|
| ``Ctrl+` `` | Focus the terminal |
| `Ctrl+B` … | herdr's prefix |
| `Ctrl+Shift+G` / `F` / `E` | Show Changes / Search (focuses the query) / Files |
| `Ctrl+Shift+B` | Show or hide the inspector |
| `Ctrl+Shift+M` | Focus mode |
| `Ctrl+Shift+T` | Next theme |
| `Ctrl+Shift+V` | Paste into the terminal |
| `Ctrl+S` / `Ctrl+W` | Save / close tab |
| `Ctrl+Z` / `Ctrl+Shift+Z`, `Ctrl+Y` | Undo / redo |
| `Ctrl+Enter` | Commit (in the commit box) |
| `Escape` | Cancel, one at a time: the sign-in dialog, a pending confirmation, the diff's selected line, its comment, the status message |

`Ctrl+Shift+…` and ``Ctrl+` `` work while the terminal has focus. `Escape` stays with the
terminal and with focused inputs; mira only acts on it when nothing else does.
