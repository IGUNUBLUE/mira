# Changelog

All notable changes to mira are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.11.1]

### Fixed

- Close the extracted executable before validating its version, so Linux permits execution
  and signed downloads can finish. Cover extraction and execution with a real ELF regression test.

## [0.11.0]

### Added

- Optional startup/focus update checks, at most daily, with manual checks and quiet availability notices.
- Explicit download and restart with pinned Ed25519 signatures, signed archive and binary checksums,
  bounded downloads, persistent staging and rollback when the updated process cannot start.
- Installer ownership records, authenticated release metadata for both Linux architectures and
  release signing in CI. Automatic checks remain off by default; no project data is sent.

### Changed

- Preserve edited workspace tabs, commit drafts, current Git operations and herdr sessions during updates.
- Update dependency notices with the executable and keep the previous binary until startup succeeds.

## [0.10.2]

### Fixed

- Open Settings from file and commit editors with Ctrl+, without inserting a comma.

### Changed

- Make field focus clearer in every palette without moving text or resizing controls.
- Delay optional hover help and clarify clean, empty-search and first-commit states.
- Mark required commit input and explain the current reason Commit is unavailable.

## [0.10.1]

### Fixed

- Center button labels and icon rows vertically across controls, menus and segmented choices.
- Remove redundant frames around inspector navigation and the Files / By agent run selector.

## [0.10.0]

### Added

- Share visual rules across buttons, inputs, menus, badges, dialogs and document tabs,
  with a native component gallery and a documented design guide adapted from Rust/UI.

### Changed

- Use medium-weight text, consistent control sizes and clearer section hierarchy
  across Changes, History, Search, Files and Settings in all four LAGC palettes.
- Keep inspector navigation readable at narrow widths and let long History labels wrap.

### Fixed

- Preserve readable Git status text on hovered and selected rows in every palette.
- Keep context menus inside short windows with consistent rows and scrolling.

## [0.9.0]

### Added

- Right-click History commits to view their changes or copy the full hash, short hash,
  or commit subject. The menu closes on Escape, outside clicks and workspace changes.

## [0.8.1]

### Fixed

- Visual regression checks use actual syntax-shaped glyphs and antialiased colors,
  so they also work with fallback fonts on Linux runners.

## [0.8.0]

### Added

- Automatically outline matching brackets and connect the active block with a colored
  guide. Indentation guides follow the editor's shaped text, including tabs and scrolling.

### Fixed

- File/image tabs now belong to their workspace and restore on return, including cursor
  positions and unsaved edits. Autosave and quit confirmation cover parked workspace tabs.
- Apply horizontal scrolling consistently to editor text, caret/selection, mouse hit testing
  and input-method positioning through a public-API adapter.

## [0.7.0]

### Added

- Code brackets use three readable colors by nesting level in all four palettes.
  Syntax grammars exclude literal brackets in strings, comments and regular expressions.
- The editor's **{ }** button and `Ctrl+Shift+\` jump to and select a matching bracket,
  or the innermost block's closing bracket when the cursor is inside the block.

## [0.6.1]

### Fixed

- History file names are clipped before the fixed-width status column, keeping long
  paths from painting underneath the M/A/D status letters in narrow panels.

## [0.6.0]

### Added

- Click a History commit to inspect its message and affected files, then open read-only
  side-by-side diffs and move between files. Comparisons handle initial commits, merges
  against their first parent, renames and deletions without including local edits.

## [0.5.2]

### Fixed

- History branch and tag badges use larger semibold text with more padding. Refs appear
  below the commit subject and wrap within the panel so long names remain readable.

## [0.5.1]

### Fixed

- Activity indicators reserve the same fixed space as idle icons, keeping Push, Pull, Refresh
  and neighboring controls stationary throughout the animation.

## [0.5.0]

### Added

- Click the branch in Changes or the status bar to search local and cached remote branches.
  Remote selection creates a local tracking branch; switching preserves local edits when Git
  can do so safely and protects unsaved editors, ignored files and other worktrees.
- History offers Undo Last Commit with explicit confirmation. Working files and existing staged
  edits are preserved, the removed commit's changes remain staged, and its message fills an empty
  commit box. Known shared commits are refused in favor of revert; stale approvals are rejected.

## [0.4.3]

### Fixed

- Unpublished branches ask which remote to publish to instead of assuming `origin`. Push/pull
  failures show persistent context, Git output and recovery actions, including an explicit IPv4
  retry for connection errors. Default OpenSSH connections use bounded address attempts and
  batch mode; custom transports are preserved. Delayed publication and retry reject changed
  branches, commits, folders or remote routing. Successful publication names its destination.
- Changes and History reserve space for their scrollbars so they do not cover row text or actions.
- Discarding files or hunks now requires an explicit confirmation, as do Discard all, reverting
  an agent run, restoring discarded files, moving files/folders to the trash, Replace all,
  signing out, and losing unsaved edits on close or overwrite. Dialogs explain the scope and
  recovery limits, start on Cancel, and reject stale or reused approvals.

## [0.4.2]

### Changed

- Manual Changes refresh, stage/unstage, discard, run actions, repository initialization and
  hunk actions show activity at their buttons. Background refresh stays quiet; hunk spinners
  animate without rebuilding diff rows.
- Replace all and Send to agent show progress and reject repeated clicks while working.
  Replies from a previous folder do not clear the current comment or rerun its search.
- Git actions with no output report completion instead of leaving their progress message behind.

## [0.4.1]

### Fixed

- Paths with glob characters (`app/[id]/page.tsx`) no longer also stage, discard or revert
  other files (`app/i/page.tsx`).
- Discard and Revert keep their Undo when part of them fails, and go on with the other files.
  Discard refuses a nested repository before touching anything; Revert snapshots first and
  waits for a running git action instead of doing nothing.
- A config file mira can't read (not UTF-8, permissions) is no longer replaced when a setting
  changes.
- Closing the window with edits autosave couldn't write asks for a second close.
- A file an agent made too large or binary is no longer overwritten by autosave with the old text.
- `Ctrl+S` on a file that changed or was deleted on disk asks first (`Ctrl+S` again replaces it);
  on a clean tab it writes nothing.
- A review comment stays in its box until it reaches the agent; with no terminal the status says so.
- A program that copies (OSC 52) right before exiting still reaches the clipboard.
- Unstaging a staged rename unstages the old path's deletion too, and worktree renames
  (`git add -N`) no longer show as a bogus extra row.
- A rejected Commit All (e.g. by a hook) unstages what it staged, and an untracked file deleted
  since the list no longer fails the whole commit.
- Git actions right after start or a folder switch run in the repository root, not the opened
  subfolder.
- A slow, older diff load no longer replaces a newer one (or another folder's), with its hunks.
- `onFocusChange` autosave saves on every tab switch (opening a file, a diff, quick open), not
  only on a tab click.
- A workspace switch keeps the active tab; a rename in Files moves its open diffs and pending
  confirmations along.
- A typed setting that can't be saved keeps Settings open with the reason instead of vanishing.
- `Escape` in a name being typed in Files cancels it, and a name left behind no longer keeps
  `Escape` from closing the find bar, confirmations or the status.
- An agent turn that ends while mira shows another folder is still snapshotted and reviewable
  there. A turn left open when mira closed ends on the first poll instead of absorbing later
  turns.
- With herdr's server not running yet, mira no longer reports a workspace error and stops
  following herdr for 30 s.
- Cancelling or timing out a `commit_command` pipeline stops all of it, not just `sh`.
- The ChatGPT sign-in can only be ended by its own redirect, and rotated tokens that couldn't
  be written no longer end the session.
- Shortcuts of the file editor and the commit box acted even when they didn't have the keyboard:
  `Ctrl+Enter` in the terminal committed, `Ctrl+Z` there undid the open file and the message.
- A turn mira didn't watch (another folder shown, or a restart) is no longer split in two when
  herdr reports nothing happened in its pane meanwhile.
- Discard removes untracked links to folders, leaves nested repositories alone while discarding
  the rest, and a worktree rename (`git add -N`) goes back to the old file instead of emptying the
  new one. A rejected Commit All restores the index exactly.
- With `autocrlf` or `eol=crlf`, unstaged diffs no longer show every line as changed.
- Some hunks had no Stage/Discard bar when git placed the change a line away from mira's view.
- Saves and Replace All write through a temporary file and a rename: a full disk no longer leaves
  half a file. A failed save says so and `Ctrl+S` retries it at once.
- Opening a FIFO or a device file no longer hangs.
- `file://` links in the terminal open the file in mira instead of being handed to `xdg-open`.
- A folder name that isn't UTF-8 no longer crashes mira at start; unknown options are reported.
- Commit messages: textconv filters no longer run, and files with thousands of changed lines stay
  in the summary but out of the diff sent to the model.
- `Backspace` at the start or `Delete` at the end of a file no longer marks it edited or drops redo.
- Regex replace reads `$1a` as group 1 followed by `a`, as VSCodium does.
- The status bar's column counts characters; search counts matches per file like its summary.

### Changed

- Changes rows show their whole path on hover; the side panel narrows on a small window.
- Placeholders and hints use the muted text colour, for contrast.

## [0.4.0]

### Added

- Settings (`Ctrl+,`, or the gear in the status bar): theme, window controls, autosave, terminal,
  commit messages and the Changes view, written to the same config file (your comments stay).
- Keyboard Shortcuts and About (version, herdr's version, links, "Copy info") from the gear;
  Check for Updates opens the releases page.
- `window_controls = "on"`: mira's own title bar with minimize, maximize and close, for desktops
  that draw none (Hyprland).

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
