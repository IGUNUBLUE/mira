<p align="center">
  <img src="assets/mira.svg" width="96" alt="mira icon: binoculars whose lenses are a diff, minus and plus">
</p>

<h1 align="center">mira</h1>

<p align="center">
  A light, terminal-first workspace for reviewing what your coding agents change.
  <br>
  <a href="https://github.com/IGUNUBLUE/mira/actions/workflows/ci.yml"><img src="https://github.com/IGUNUBLUE/mira/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="License: MIT OR Apache-2.0"></a>
  <img src="https://img.shields.io/badge/platform-Linux-informational" alt="Platform: Linux">
</p>

![mira: the terminal on the left, the agent's changes on the right](docs/images/mira.png)

*Mira* is Spanish for "look!". You talk to your agents in the terminal; mira sits around that
loop so you can **see what they did**, fix it, and commit it.

mira's terminal runs [herdr](https://herdr.dev), so **every agent herdr recognizes works with
mira, with nothing to set up per agent**: Claude Code, Codex, GitHub Copilot CLI, Cursor Agent,
OpenCode, Gemini CLI, Kiro, Amp, Droid, Qwen Code and
[more than twenty others](https://herdr.dev/docs/agents/). When herdr learns a new agent, mira
tracks it too.

> [!NOTE]
> **mira is in early development.** I use it every day, but it is young: expect rough edges,
> missing features and breaking changes before 1.0. Linux only for now.
> [Bug reports and ideas](https://github.com/IGUNUBLUE/mira/issues) are very welcome.

## Why

I moved most of my coding to agents that run in a terminal, and kept an editor open only to
review their work. The editor took more memory than the agents, and it still couldn't tell me
which agent changed which file. mira is the small tool I wanted instead: a native Rust app
with the terminal at the center and the review around it. [The longer story →](docs/why.md)

## Features

- **Any agent, through herdr.** herdr tells mira which agent works in which pane and whether
  it is working, done or waiting for you; mira turns that into reviewable turns. Run several
  agents side by side in herdr's workspaces and panes: they keep running when mira closes, and
  mira follows the workspace you are in. [What herdr brings →](docs/guide.md#what-herdr-brings)
- **Changes, with authors.** Every changed file says which agent touched it and when. Review
  turn by turn, titled with your prompt; stage or revert a whole turn.
- **Diffs that talk back.** Side-by-side diffs. Click a line, write a comment, and it is typed
  into the pane of the agent that made the change.
- **Commit messages that stream.** Conventional Commits from your ChatGPT subscription, an
  OpenAI API key, or any local command (e.g. ollama).
- **The VS Code basics, without the weight.** Source control, search & replace, an explorer,
  a syntax-highlighted editor with undo and autosave, an image viewer, file icons, four themes.
- **Private by default.** No telemetry, no account needed, no network unless you ask for a
  commit message or check for updates. [What leaves your machine →](docs/privacy.md)

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/IGUNUBLUE/mira/main/install.sh | sh
```

This installs a release binary (or builds from source with cargo) into `~/.local`, with a
desktop entry and icon. Linux only for now (x86_64, aarch64). Uninstall with
`… | sh -s -- --uninstall`.

Install [herdr](https://herdr.dev) too: it is what lets mira know which agent changed what (see
the [recommended herdr settings](docs/guide.md#herdr-settings)). Without it, mira runs your shell
and still shows your git changes, diffs and history, but not who made them. A
[Nerd Font](https://www.nerdfonts.com) gives the icons. mira needs `git` and `curl`. Signed updates also need `tar` and OpenSSL 3.

Update through **Settings → Updates**, or rerun the installer. Automatic checks are off by default;
when enabled, they run at startup/focus at most once a day. Downloading and restarting require a click.
Package-manager and `cargo install` installations stay managed by their original tool.

From source: `cargo install --git https://github.com/IGUNUBLUE/mira --locked`.

## Use

Open mira from your app launcher, or run `mira [folder]`. Ask an agent to do something in the
terminal; its changes show up on the right as it works. Key shortcuts: ``Ctrl+` `` terminal ·
`Ctrl+Shift+P` open a file · `Ctrl+Shift+G` changes · `Ctrl+Shift+F` search · `Ctrl+Shift+E` files.

- [User guide](docs/guide.md): every panel, shortcuts, commit messages, herdr
- [Configuration](docs/configuration.md): `~/.config/mira/config`, environment, files
- [Privacy & network](docs/privacy.md)
- [Architecture](docs/architecture.md), for contributors · [Roadmap](docs/roadmap.md)

## Contributing

Issues and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first; the
quality gate is `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
Security issues: see [SECURITY.md](SECURITY.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Release binaries include `THIRD_PARTY_LICENSES.html` with the licenses of the crates compiled
into them.
