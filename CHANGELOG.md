# Changelog

All notable changes to mira are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.1.0]

First public release.

### Added

- Terminal running [herdr](https://herdr.dev), with the agents of the folder in its header
  and a jump to any agent's pane. mira follows herdr's focused workspace.
- Changes panel with the agent that changed each file, new-change dots, and a view by agent
  turn titled with the prompt (Codex and Kiro logs); stage or revert a whole turn.
- Side-by-side diffs with inline highlights; comments on a line are typed into the pane of the
  agent that made the change.
- Source control: stage, unstage, commit, push, pull, history, merge-conflict section,
  `git init`.
- Streaming commit-message generation from a ChatGPT subscription, an OpenAI API key or a
  local command.
- Editor with syntax highlighting, undo/redo and autosave; search & replace; file explorer.
- Four LAGC themes (light and dark), following HyDE when installed; Nerd Font icons.
- Install script, release binaries for Linux x86_64 and aarch64, desktop entry and icon.
