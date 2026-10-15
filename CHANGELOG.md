# Changelog

All notable changes to mira are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org).

## [Unreleased]

### Added

- Line numbers in the editor, with the cursor's line highlighted. Long lines no longer wrap.
- Changes: per-file and per-section buttons like VSCodium (open file, discard, stage/unstage),
  shown on hover; Review (⇄) walks through a section's diffs with `‹ ›`; Discard from the diff.
- Agent turns of Claude Code are titled with the prompt too (from `~/.claude/projects`).
- Discard with Undo: a snapshot is taken first, so a discard (even of new files) can be undone
  from the status bar.
- Find in the open file (`Ctrl+F`): a find bar with Match Case, a `3/17` count and previous /
  next (`Enter` / `Shift+Enter`, `F3` / `Shift+F3`); lines with matches are marked in the line
  numbers.
- Quick open (`Ctrl+Shift+P`, or `Ctrl+P` outside the terminal): fuzzy search of the folder's
  files (.gitignore respected); `:` followed by a number, or `Ctrl+G` in the editor, goes to a line.

### Fixed

- "N new" counted files of older agent turns that were already committed.

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
