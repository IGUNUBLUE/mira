# Changelog

All notable changes to mira are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.3.0]

### Added

- Right-click menu in Files: new file or folder, rename in place, delete to the trash (with
  Undo), copy the path or relative path, send the path to the agent.

### Changed

- mira draws its terminal itself (over Alacritty's terminal emulation) instead of using
  iced_term: only the visible rows are copied per frame, box-drawing lines join without gaps,
  cursor shapes (bar, underline) and an outline cursor when unfocused, curly/double/dotted
  underlines, Ctrl+click links, `Shift`-drag selection over herdr, `Shift+Insert` and middle
  click paste.

### Fixed

- The cursor was invisible on lines with their own background, such as Codex's prompt.
- `Ctrl+U` sent `Q` instead of deleting the line.
- Opening quick open (or another dialog) reset the panels: the terminal lost the keyboard after
  `Escape` and scroll positions jumped back.

## [0.2.0]

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
- Stage, unstage or discard a single change (git's hunk) from its bar in the diff; a discarded
  hunk can be undone.
- The line under the pointer is highlighted in diffs; Commit, Push and Pull spin while they run;
  `Ctrl+Shift+G` focuses the commit message, as in VSCodium.
- Open images (PNG, JPEG, GIF, WebP, BMP, ICO) with zoom and pan; SVG files get a Preview.

### Changed

- A new icon: binoculars (*mira* is Spanish for "look") whose lenses are a diff, − and +.

### Fixed

- Diffs could show an empty body: the cached rows were given no width.
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
