# mira visual design

Status: implemented in mira's shared native styles and product views, with a runnable Iced specimen.
The initial visual direction was approved; the latest product captures remain available for review.
This records implementation rules and validation evidence; it does not establish complete keyboard accessibility.

## Purpose and priorities

mira is a compact desktop workspace for asking an agent to act, reviewing its changes, and editing
or committing the result. Keep the terminal as the dominant working area. Make the surrounding
controls feel deliberate, consistent, and easy to scan.

Use Rust/UI as the explicit reference for component anatomy, visual hierarchy, and states. Preserve
mira's four LAGC palettes, medium-weight text, native rendering, file workflows, and herdr ownership.
The design must work offline with the existing dependencies and system fonts.

Resolve competing requirements in this order:

1. Preserve data, keyboard behavior, confirmation safeguards, and running agent sessions.
2. Keep text legible and controls usable at narrow inspector widths.
3. Protect terminal and editor space.
4. Reuse the geometry, semantic roles, and component variants defined here.
5. Refine decoration only when it helps identify a surface or state.

## Reference and evidence

The method follows Vercel's [design.md article](https://vercel.com/blog/how-our-agents-build-on-brand-pages-with-design-md):
combine written judgment with reusable implementation and repeatable visual review. Here, native
Iced helpers provide the shared implementation. A stylesheet is not part of the application.

Rust/UI's [buttons](https://rust-ui.com/docs/components/button),
[inputs](https://rust-ui.com/docs/components/input), and [tabs](https://rust-ui.com/docs/components/tabs)
are the initial references. Browser measurements and screenshots are recorded in
[`docs/design-reference/`](docs/design-reference/README.md), dated 2026-10-23. Treat the measured
component pages as evidence; the homepage's example theme values can differ from the deployed
component stylesheet.

The [deeper review](docs/design-reference/rust-ui-review.md) covers sixteen component families,
including dialogs, menus, fields, command palettes, tooltips and scrolling. It records the source
revision and observed behavior separately from the native adaptation.
The [UX refinement review](docs/design-reference/ux-refinements.md) records the later W3C, Fluent,
Microsoft, NN/G and VS Code research and the limited changes adopted from it.

Keep measured reference values separate from mira decisions:

| Property | Rust/UI observation | mira adaptation |
|---|---|---|
| Control text | Inter, 14 CSS px, medium button labels | Inter, 14 logical px, medium labels and values |
| Supporting text | Smaller, secondary text | 12 logical px, medium, contrast-tested |
| Button heights | Small 32, default 36, large 40 CSS px | Compact 28, default 32, dialog action 36 logical px |
| Button corner | 8 CSS px in the measured page | 6 logical px |
| Input | 36 CSS px high, 1 px border, 8 px corner | 32 logical px high, 1 px border, 6 px corner |
| Spacing base | 4 CSS px | 4 logical px; use 8, 12, 16, 24 for composition |
| Button variants | Primary, secondary, outline, ghost, destructive, link | Same roles; link styling reserved for actual navigation |
| Color vocabulary | Background, foreground, primary, secondary, border, ring | Map roles to LAGC; never import the reference's neutral palette |
| Disabled state | Reduced whole-control opacity | Readable muted label and quiet surface, with activation removed |

Browser CSS pixels and Iced logical pixels are compared at scale 1. Respect the application's display
scale; do not multiply dimensions by the monitor scale yourself. These are desktop densities, not
touch target recommendations.

## Typography and geometry

Use `theme::ui_font()` for interface copy and `theme::mono_font()` for code and terminal text. System
font overrides continue to work. Do not download or bundle fonts as part of this work.

| Role | Size | Weight | Use |
|---|---:|---|---|
| Dialog title | 20 | Medium | One clear decision or task |
| Section title | 16 | Medium | Changes, History, a settings section |
| Body / control | 14 | Medium | File names, commit subjects, buttons, field values |
| Metadata | 12 | Medium | Paths, hashes, time, counts, help text |

Prefer sentence case. Reserve monospace for values whose character structure matters. A commit
subject is prose. Branch names and paths may wrap at any glyph when they exceed the available width.
Keep important names complete in an expanded/detail surface; a tooltip alone is insufficient there.

Use 8 px between closely related controls, 12 px between fields or groups inside a panel, 16 px
panel padding, and 24 px between independent specimen sections. List rows can use 8–12 px horizontal
insets and 4–8 px vertical padding. Avoid padding that scales with window width.

Fixed-height buttons use `ui::control` (or `ui::action` / `ui::primary_wide`). Center the
whole content vertically in the available height, including icon/label rows and segmented choices;
aligning the children of a row alone does not center that row inside its button.

Controls use a 6 px radius; small badges use 4 px; contained surfaces use 8 px. Window and split-pane
edges remain structural. Use a 1 px border for surfaces and controls that need an edge. Avoid nested
cards around every subsection: the specimen's component cards are a catalog presentation, not a
requirement to put every production section in a card.

## Semantic color roles

All colors come from `theme::p()`. Use roles consistently across Tech Light, Tech Dark, Calm Light,
and Calm Dark. Preserve the existing ANSI palettes and syntax colors.

| Role | Existing palette field | Rule |
|---|---|---|
| Main / input surface | `bg` | Terminal, editor, fields, selected tabs |
| Supporting surface | `sidebar` | Inspector and secondary chrome |
| Group surface | `section` | Secondary buttons with ordinary foreground labels |
| Divider / quiet outline | `border` | Structural separation |
| Main text | `fg` | Names, subjects, control labels |
| Secondary text | `muted` | Paths, help, timestamps; retain readable contrast |
| Primary action | `button`, `button_hover`, `on_accent` | One primary action per task area |
| Field focus | `focus` | A 2 px outline contrasted against the idle border, field and panel |
| Activity / emphasis | `accent` | Active operation indicator and existing interaction emphasis |
| Selection | `selection` | Existing editor/terminal selection; adapt list selection with readable text |
| Modified / warning | `warn_text` | Written status paired with this color |
| Added / success | `added_text` | Written status paired with this color |
| Deleted / error | `error_text` | Error text and destructive fill paired with `on_accent` |
| Floating shadow | `shadow` | Palette-owned shadow used only by floating surfaces |

The shared styles use text-safe `error_text` for destructive button backgrounds so light themes retain
contrast with `on_accent`. Keep text at least 4.5:1 against its actual composited surface. Do not apply
opacity to an entire control merely to dim it. `line_no` is reserved for gutters.

Rows containing status-colored text use `sidebar` on hover and `bg` for selection, with an outline
that distinguishes the states. Do not put `warn_text`, `error_text`, or `added_text` directly on
`section`: several palette combinations fall below 4.5:1 there. The shared row test guards this rule.

## Component contracts

### Buttons

- Primary: the next deliberate action, such as committing the reviewed changes.
- Secondary: a substantial companion action inside the same task.
- Outline: a quieter, clearly bounded action, such as canceling a dialog or pushing a branch.
- Ghost: toolbar and row actions whose location already provides context.
- Destructive: the final destructive action inside the existing confirmation flow.

Keep icon and label geometry stable across idle and busy states. Production icons continue through
`icons::Icon`, `icons::file_icon`, and their shared helpers. Reserve a fixed icon slot; keep a tooltip
on icon-only controls. The specimen uses text labels to avoid requiring the user's icon configuration.

Hover must be visible without changing geometry. Pressed feedback must not move surrounding content.
A selected control keeps a visible surface or outline after the pointer leaves. Busy actions retain
their label area, prevent duplicate submission, and report completion or failure. Background refresh
stays quiet. Do not add scale, bounce, or continuous animations to ordinary controls.

### Inputs

Use a persistent label above the field when its purpose would otherwise disappear after typing.
Place help and validation below it. Use 14 px medium text with an 18 px line height and 7 px vertical
padding for the 32 px field. Share this geometry with adjacent buttons. Mark required input in its
persistent label, as in Commit message (required). A focused field or commit editor uses the palette's
`focus` color and a 2 px border; the idle border stays 1 px. Require at least 3:1 contrast against the
idle border, field background and surrounding panel in each palette. The border is paint-only: keep
padding, text position and control bounds unchanged. Errors use a written explanation and the semantic
error color. This is a native visual rule inspired by W3C focus guidance, not a claim of WCAG compliance.

An app shortcut handled inside a focused editor must publish its command before Iced's printable-text
fallback. For example, Ctrl+, opens Settings and never inserts a comma; an ordinary comma still types.
Unfocused editors must not capture shortcuts belonging to the active widget.

### Tabs and segmented choices

Distinguish navigation (Changes / Search / Files, document tabs) from choices within a view
(Files / By agent run). Place related choices directly on the panel surface, without an enclosing
border, background or padded track. Give only the active choice its own surface and outline, avoiding
nested boxes. Do not rely on an accent-colored label alone. Use the same height and spacing
across sibling tabs. Preserve document close affordances and existing drag/focus behavior.

Below a 320 px inspector width, show navigation labels without their decorative icons/counters so
Changes, Search and Files remain readable. Change counts remain in their section headings. Keep the
underlying messages and shortcuts identical across the two presentations.

### Lists and badges

The file name or commit subject is the first reading level. Path, time, hash, and origin are secondary.
Align status and actions consistently. Keep row actions stable when revealed on hover.

Use small bordered badges for counts and references. A branch reference is secondary to its commit
subject. Put long references on a separate full-width row where needed, with `WordOrGlyph` wrapping.
Never shrink text to fit a reference. A narrow inspector must still expose the complete expanded name.
History subjects also wrap; their length must not obscure adjacent controls.

### Menus, dialogs, and feedback

Menus use the same body text and compact row scale. Group related commands, align shortcuts on the
right, and preserve keyboard dismissal. Shadows, if needed, belong to floating surfaces only.
Optional tooltips use `ui::help`: show after 500 ms of hover, or 700 ms for full row/path/commit
details. Dismiss when the pointer leaves; wrap long names within a bounded surface. Keep action labels,
required input and loading/empty/error state visible without hover. A disabled action's tooltip explains
the current blocker instead of promising an available action. Use Iced's one-shot redraw scheduling;
do not add a polling timer for hover help.

Menu placement includes row heights, item gaps, separators, and padding. Constrain oversized menus
to the viewport and allow their contents to scroll instead of placing actions beyond the window.

Dialogs have a title, a short explanation, affected targets, and clearly ordered actions. Keep the
existing folder-bound, one-shot confirmation machinery and Cancel-first selection. Visible errors
remain readable/copyable until dismissed. Empty states explain what will appear or how to proceed
without a large illustration. Use `ui::empty_state` for a short title and one actionable explanation,
directly on the panel with no extra card, icon or button. Distinguish a clean working tree, history
before its first commit and a completed search with no matches. Never show a completed empty result
while loading or after an error. Preserve previous results during a running search. Status text must
fit or wrap; it must not hide actions.

## Composition in mira

**Changes:** navigation first; branch and synchronization together; labeled commit composition and
one primary commit action; then staging controls and file rows. Avoid giving every toolbar icon the
same visual weight as Commit. Group through alignment and spacing before introducing extra borders.

**History:** section title, commit subject, secondary time/hash, then reference badges. Keep long refs
below the subject. Expanded affected files use the same file-row conventions as Changes.

**Search:** query first, options grouped beside or below it, replace clearly separated, then count and
results. Maintain draft/input focus, existing debounce, and cancellation behavior.

**Files:** keep tree density and indentation stable. Use the same row actions and menu treatments.
Dotfiles remain ordinary files. Name editing stays in place.

**Settings:** group related controls, label fields consistently, and align companion actions. Use
help text only where it explains a choice. Keep the four palette names and existing configuration.

**Terminal/editor:** preserve text size, cell geometry, input ownership, and rendering behavior.
Changes to surrounding chrome must not alter the terminal's keyboard routing or add redraw timers.

## Native implementation and present limits

[`src/theme/ui.rs`](src/theme/ui.rs) owns shared geometry, button variants, fields, delayed hover help, empty states, selection, badges,
scrollbars, input/editor surfaces, menu measurements and floating surfaces. `src/theme.rs` retains
the palette and compatibility style entry points. [`src/view.rs`](src/view.rs) composes these primitives
in Changes/History, Search, Files, Settings, branch/quick-open pickers, menus, confirmations, sign-in,
and surrounding document/terminal chrome.

[`examples/design_system/components.rs`](examples/design_system/components.rs) re-exports the same
native implementation. [`examples/design_system/main.rs`](examples/design_system/main.rs) composes
it with static fixtures and interactive local states. There is one palette and primitive implementation.

Use built-in Iced 0.14 widgets and style functions. Extend the shared helpers for new product surfaces.
Keep view construction free of IO and expensive
per-row work. Do not add Dioxus, Tailwind, a WebView, or an animation dependency for this adaptation.

Iced 0.14's built-in button exposes Active/Hovered/Pressed/Disabled styles and does not provide a
keyboard-focus state or built-in button tab navigation. Pointer states, input focus, and existing app
keyboard routes are validated; this does not establish complete assistive-technology acceptance.
Preserve existing app keyboard handling and confirmation selection,
and explicitly implement/test any new keyboard-selectable control before claiming parity with the
browser reference. A painted focus outline is not proof of keyboard support.

## Review loop

Start a review from the same fixtures and dimensions. Change the guide and shared primitive together
when a rule changes. Record why a deliberate deviation from Rust/UI improves desktop use.

Run the specimen with:

```sh
WGPU_BACKEND=gl cargo run --example design_system
```

Environment options: `MIRA_DESIGN_THEME=0..3` selects the palette in `theme::PALETTES` order;
`MIRA_DESIGN_WIDTH=240..480` sets the inspector width; `MIRA_DESIGN_DIALOG=1` opens the dialog specimen;
`MIRA_DESIGN_CAPTURE=/absolute/path.png` saves a window screenshot and exits. The destination parent
must exist. Launch with temporary XDG config/state roots when validating on a live desktop.

Review these scenarios:

| Scenario | Inspect |
|---|---|
| All four palettes, same viewport | Contrast, surface hierarchy, readable disabled controls |
| Inspector at 240, 360, 480 logical px | Long refs, action visibility, field width, wrapping |
| Primary, secondary, outline, ghost, destructive | Consistent height, radius, weight, pointer feedback |
| Input click/type, empty message | Focus border, readable value, disabled empty commit |
| Stage/unstage; simulated commit | Stable geometry, no duplicate busy action, visible completion |
| Dialog open/close | Targets, Cancel/Discard order, Escape, no real file mutation |
| Narrow layout with fallback fonts | Names and badges stay within the panel |

Use mechanical checks for contrast and bounds, and rendered screenshots for hierarchy and rhythm.
Human review decides whether the composition is good. Product captures use the real application and
temporary repositories/configuration. Specimen captures demonstrate the component vocabulary. The
evidence README records what has actually been checked and the remaining acceptance limits.

For Rust changes, run the repository quality gate; shared and example checks run in `cargo test`.
Keep screenshots limited to useful reference states. No commits,
installation, or restarts are implied by updating this document.


### Update feedback

Update checks never take focus or open a dialog automatically. Show availability as a quiet
status-bar action. Keep Download and Update and restart separate, explicit steps; no countdown,
forced restart, repeated toast or progress animation for background checks. The Updates dialog
uses shared controls, plain progress text and persistent copyable errors. Explain any unsaved
work that postpones restarting and retain the verified download for later.
