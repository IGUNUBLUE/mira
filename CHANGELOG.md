# Changelog

All notable changes to mira are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.1.1]

### Added

- Release archives include `THIRD_PARTY_LICENSES.html`, the licenses of every crate compiled
  into mira (generated with cargo-about); the installer puts it and mira's licenses in
  `~/.local/share/doc/mira`.

## [0.1.0]

First public release. mira is in early development: expect breaking changes before 1.0.

### Added

- Terminal running [herdr](https://herdr.dev), with the agents of the folder in its header
  and a jump to any agent's pane. mira follows herdr's focused workspace and reports its
  light/dark palette to herdr (as Ghostty does), so herdr's theme can follow.
- Changes panel with the agent that changed each file, new-change dots, and a view by agent
  turn titled with the prompt (Codex and Kiro logs); stage or revert a whole turn.
- Side-by-side diffs with inline highlights; comments on a line are typed into the pane of the
  agent that made the change.
- Source control: stage, unstage, commit, push, pull, history, merge-conflict section,
  `git init`.
- Streaming commit-message generation from a ChatGPT subscription, an OpenAI API key or a
  local command. Signing in with ChatGPT goes through a dialog that explains the flow
  and asks before opening the browser.
- Editor with syntax highlighting, undo/redo and autosave; search & replace; file explorer.
- Four LAGC themes (light and dark), following HyDE when installed; Nerd Font icons.
- Install script, release binaries for Linux x86_64 and aarch64, desktop entry and icon.
