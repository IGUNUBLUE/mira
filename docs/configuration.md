# Configuration

mira works without any configuration. Most settings can be changed in **Settings** (the gear in
the status bar, or `Ctrl+,`); they live in `~/.config/mira/config`
(`$XDG_CONFIG_HOME/mira/config`), one `key = "value"` per line; `#` starts a comment. mira
writes some keys itself (theme, panel sizes) and keeps your other lines and comments.

```toml
# ~/.config/mira/config
theme = "LAGC Tech Dark"
autosave = "afterDelay"
commit_provider = "openai"
```

## Keys

| Key | Values (default first) | What it does |
|---|---|---|
| `theme` | `auto`, `LAGC Tech Light`, `LAGC Tech Dark`, `LAGC Calm Light`, `LAGC Calm Dark` | Palette. `auto` follows HyDE when installed. Set by `Ctrl+Shift+T`. |
| `terminal` | *(herdr)*, `shell` | `shell` runs your `$SHELL` instead of herdr. |
| `herdr_session` | *(default session)*, a name | Attach to a named herdr session. |
| `autosave` | `afterDelay`, `onFocusChange`, `onWindowChange`, `off` | Same meaning as VS Code's `files.autoSave`. |
| `autosave_delay` | `1000` | Milliseconds of no typing before `afterDelay` saves (minimum 100). |
| `commit_provider` | `auto`, `chatgpt`, `openai`, `command` | Where commit messages come from; see the [guide](guide.md#commit-messages). |
| `commit_model` | `gpt-6-luna` | Model for `chatgpt` and `openai`. |
| `commit_command` | *(none)* | Shell command for `command`: gets the prompt on stdin, prints the message. |
| `changes_view` | `files`, `runs` | Changes grouped by file or by agent run. Set from the panel. |
| `icon_font` | *(first Nerd Font found)* | Font family to take icons from. |
| `window_controls` | `auto`, `on`, `off` | Who draws close, minimize and maximize. `auto`: the desktop (tiling compositors such as Hyprland draw none). `on`: mira draws its own title bar (drag it to move, double-click to maximize). `off`: no title bar. |
| `inspector_width`, `history_height` | | Panel sizes, saved when you drag the dividers. |

A local model as commit-message generator, with [ollama](https://ollama.com):

```toml
commit_provider = "command"
commit_command = "ollama run qwen2.5-coder:7b"
```

## Environment

| Variable | What it does |
|---|---|
| `OPENAI_API_KEY` | API key for `commit_provider = "openai"` (or `auto`). |
| `MIRA_FONT`, `MIRA_MONO_FONT` | UI and monospace font families (defaults: Inter, JetBrains Mono). |
| `WGPU_BACKEND` | GPU backend. mira defaults to `gl` (on some Vulkan drivers colors came out washed). |
| `ICED_BACKEND=tiny-skia` | Draw on the CPU: less memory, but typing and resizing feel slower. |
| `XDG_CONFIG_HOME`, `XDG_STATE_HOME` | Where config and state live (useful to try mira in isolation). |

## Command line

```text
mira [--fresh] [FOLDER]
```

`--fresh` forgets the agent turns recorded for the folder.

## Files mira creates

| Path | Contents |
|---|---|
| `~/.config/mira/config` | Your settings. |
| `~/.config/mira/openai_key` | Optional API key, first line. mira refuses it unless it is `chmod 600`. |
| `~/.local/state/mira/<hash>.json` | Agent turns recorded per folder (prompts included), `0600`. |
| `~/.local/state/mira/chatgpt.json` | ChatGPT sign-in tokens, `0600`. Removed when you sign out. |
| `~/.local/state/mira/index/` | Temporary git indexes for snapshots, `0700`, removed after each one. |
| `~/.local/state/mira/last_root` | The last folder shown. |

Nothing is written inside your projects. Snapshot trees are ordinary git objects in the
repository's object store, not referenced by any branch; `git gc` cleans them up eventually.
