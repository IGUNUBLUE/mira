# Rust/UI review for mira

Reviewed 2026-10-23 against the public component pages and the upstream source at
[`7120636e68556ddd023a4ec8e083663c7b745318`](https://github.com/rust-ui/ui/tree/7120636e68556ddd023a4ec8e083663c7b745318/app_crates/registry/src/ui).
The source checkout was used for inspection only. The deployed website is a separate observation;
its deployment is not proven to use that exact revision.

## What was reviewed

Sixteen component families were inspected for anatomy, geometry, color roles, pointer states,
overflow, and keyboard implementation. This is a focused review of components relevant to mira,
not an audit of every Rust/UI component or framework target.

| Family | Upstream finding | Native rule / implementation |
|---|---|---|
| Button | Explicit variants and 32/36/40 px size scale; 14 px medium labels; fixed icon sizes | 28/32/36 px desktop scale, shared `Variant` styles, stable icon/spinner slots |
| Input | Bounded width, border and focus ring; 36 px desktop height | Shared 32 px `ui::field`, persistent labels where context could disappear |
| Textarea | Content-sized field with minimum height and bounded width | Keep mira's auto-growing commit editor and existing undo; share its surface with inputs |
| Tabs | Quiet grouped track; active surface; horizontal/vertical and line variants | Group inspector and view choices; active surface plus outline, compact labels below 320 px |
| Badge | Separate outline, muted, status and primary roles; 12 px default text | References use a quiet border and readable text instead of saturated pills; long refs wrap |
| Dialog | 24 px padding, title/body/footer anatomy, 16 px corner, backdrop and Escape handler | Shared 24 px padding and 8 px floating corner; preserve bounded native overlay ownership |
| Alert dialog | Dialog wrapper disables backdrop dismissal | Retain mira's one-shot confirmation, affected targets, Cancel-first selection and existing keyboard controls |
| Context menu | 4 px inset, 200 px default width, 32 px item rhythm; outside-click/Escape close | 220 px native menus, shared 32 px rows; placement accounts for their real height and window edges |
| Dropdown menu | Grouping, separators, Escape handling and radio-specific roles | Keep mira's own settings/branch interactions; reuse quiet surface and menu rhythm |
| Command | Search input, grouped results, selected row, empty state; arrow/Enter handling | Style the existing quick-open and branch pickers; retain their ranking, selection and folder guards |
| Tooltip | Secondary 12 px copy; CSS hover visibility and unconstrained no-wrap content | Native bounded/wrapping tooltips; retain delayed row hints and persistent critical errors |
| Scroll area | Separate viewport, track and thumb; inherited corners | Reserve a gutter for row actions and use a shared narrow scrollbar on inspector/form lists |
| Separator | One structural line, orientation-specific | Existing one-pixel palette divider; add separation only between meaningful groups |
| Field | Label/content/help/error anatomy; grouped and responsive arrangements | Settings retain labels and help; commit/search get labels that remain visible after typing |
| Card | Header/content/footer roles; 16/24 px inner scales | Use spacing and section hierarchy in the inspector; floating dialogs and gallery cards have bounded surfaces |
| Empty | Title, explanation and optional action; generous website padding | Compact explanation and useful action in the real panel; preserve room for the terminal |

Source files use the matching names in `app_crates/registry/src/ui/`, including `alert_dialog.rs`,
`context_menu.rs`, `dropdown_menu.rs`, and `scroll_area.rs`. The native implementation lives in
[`src/theme/ui.rs`](../../src/theme/ui.rs), with composition in [`src/view.rs`](../../src/view.rs).

## Behavior findings that affect the adaptation

The browser dialog opened and closed with Escape. Immediately after opening, focus stayed on the
trigger outside the dialog, and the open panel had neither `role="dialog"` nor `aria-modal`. The
reviewed source installs click and Escape handlers but contains no focus transfer/trap/restore logic.
This observation does not establish the behavior of other revisions or every Rust/UI application.

The inspected tabs implement click selection and focus styling, but their root/list/trigger/content
markup lacks tab-specific roles and arrow-key handling. Menus provide Escape handling; the command
component adds explicit arrow/Enter selection. A shared appearance therefore does not imply shared
keyboard completeness.

Rust/UI's tooltip is driven by CSS hover and `whitespace-nowrap`. Its stock treatment is useful as
an appearance reference but unsuitable as the only home for a long file name, branch, or persistent
Git error. mira retains its bounded tooltip and expanded detail behaviors.

The source button uses press scaling and broad transitions. mira keeps geometry fixed for pointer
and busy states. The source disabled style dims the whole element; mira instead uses contrast-tested
muted text and a quiet surface. These are deliberate adaptations for readability and terminal work.

Iced 0.14's built-in button still does not supply browser-style Tab focus. This migration preserves
existing shortcut, palette, modal and confirmation handling; it does not claim universal keyboard
navigation or complete assistive-technology parity for every button.

## Evidence

The native review also found that status-colored filenames on the old `section` hover background
could miss 4.5:1: Tech Light warning text measured 4.16:1, Calm Light added text 3.97:1, and Calm Dark
error text 4.13:1. A regression test reproduced the failure. Rows now use the already-readable
`bg`/`sidebar` surfaces plus state outlines; the test covers all semantic text colors, palettes,
hover/pressed/disabled states, and selection.

- [Measured component values](rust-ui-measurements.json), including the initial button/input/tab review.
- [Browser behavior observations](rust-ui-behavior-review.json), including source revision and dialog/menu states.
- [Open reference dialog](rust-ui-dialog-review.png).
- [Open reference context menu](rust-ui-context-menu-review.png).
- [Native product captures and validation](README.md).

## Implementation rules carried forward

Written decisions belong in `design.md`. Reusable geometry, variants, surfaces and fields belong in
`src/theme/ui.rs`. Product views compose those primitives while their existing messages continue to
own behavior. The gallery imports the same implementation, so it cannot silently diverge into a
second design system.

Keep names complete in expanded details, keep an outline/surface for selection after hover ends,
preserve pointer targets across spinners, and validate final pixels as well as layout bounds. Recheck
the same fixture views at narrow widths and in every LAGC palette when changing a shared primitive.
