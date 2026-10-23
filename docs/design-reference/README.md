# Design reference and validation

Captured 2026-10-23. The governing guide is [`design.md`](../../design.md).

## Rust/UI evidence

The reference was inspected in a separate Chromium session with `agent-browser`, at a 1440 × 1100
viewport and device pixel ratio 1. Measurements are browser-computed CSS, not estimates from images.

- [Button, light](rust-ui-buttons-light.png): variants and their relative emphasis.
- [Button, dark](rust-ui-button-dark.png): the same variants in the dark reference theme.
- [Input focus, dark](rust-ui-input-dark.png): field geometry and focused treatment.
- [Tabs, dark](rust-ui-tabs-dark.png): the grouped track and active/inactive choice.
- [Computed measurements](rust-ui-measurements.json): URLs, theme state, typography, sizes, spacing,
  borders, and colors. The light input record was collected before focus; the dark record includes focus.

Sources: [Button](https://rust-ui.com/docs/components/button),
[Input](https://rust-ui.com/docs/components/input), [Tabs](https://rust-ui.com/docs/components/tabs).
The implementation method is informed by Vercel's
[design.md article](https://vercel.com/blog/how-our-agents-build-on-brand-pages-with-design-md).
The native primitives were written for Iced; reference component code was not vendored.

The [deeper source and behavior review](rust-ui-review.md) covers sixteen component families at a
recorded upstream revision. [Dialog/menu observations](rust-ui-behavior-review.json) distinguish
measured browser behavior from native design choices.

## Product implementation

The real application now uses [`src/theme/ui.rs`](../../src/theme/ui.rs) for shared geometry,
variants, fields, row feedback, references, scrollbars and floating surfaces. Existing messages,
Git operations, folder epochs, confirmation approval and keyboard routing continue to own behavior.
The gallery re-exports these primitives instead of maintaining a second implementation.

These product captures use the real application with a temporary repository, XDG config/state,
and an isolated Xvfb display. The shell runs only inside that instance; no user herdr session is
opened or restarted. Normal captures are 1400 × 900 logical/physical pixels at scale 1.

| Capture | Surface checked |
|---|---|
| [Before](product-before-changes.png) / [After](product-after-changes.png) | Changes and History, same fixture data |
| [Hover](product-after-hover.png) | Status-colored file text and stable row actions |
| [Search](product-after-search.png) | Persistent labels, options and replace controls |
| [Files](product-after-files.png) | Tree density, dotfiles, selection |
| [Context menu](product-after-file-menu.png) | Shared menu row geometry |
| [Confirmation](product-after-confirmation.png) | Affected path, Cancel-first selection, destructive action |
| [Diff](product-after-diff.png) / [Editor](product-after-editor.png) | Real document/terminal composition |
| [Settings](product-after-settings.png) | Group hierarchy, labels, wrapped choices and bounded scrolling |
| [Quick open](product-after-quick-open.png) | Existing palette interaction with the shared field/surface |
| [Tech Dark](product-after-tech-dark.png) | Theme switching in the running product instance |
| [Calm Light](product-after-calm-light.png) / [Calm Dark](product-after-calm-dark.png) | Same product views in the other palettes |
| [Narrow Changes](product-narrow-changes.png) / [Narrow Search](product-narrow-search.png) | 240 px inspector, readable navigation labels |
| [Scale 2×](product-hidpi-changes.png) | Same logical layout at a 2800 × 1800 physical resolution |
| [Alternative fonts](product-fallback-changes.png) | DejaVu Sans and DejaVu Sans Mono |

Native interaction checks opened a changed-file diff, opened a file through quick open, navigated
Search/Files/Changes, opened Settings, switched all four palettes, and opened a Files context menu.
The delete confirmation was canceled using its existing Tab/Enter selection; the fixture file was
verified to remain present. No network-facing button was activated.

The production layout test checks Changes/Search/Files at 240/360/480 px and both Files and History
menus at the lower window edge. Shared row tests cover all semantic text colors on idle, hover,
pressed, selected and disabled backgrounds; they reproduced and fixed sub-4.5:1 hover contrast.

## Control refinement

The [refined controls](product-controls-refined.png) crop records the follow-up to 0.10.0: vertically
center the complete content of fixed-height buttons and remove the enclosing frames around inspector
navigation and Files / By agent run. The active choice retains its own surface and outline. This is a
crop of the same native product fixture, not a mockup. The four palettes and existing interaction flow
were checked again; layout tests cover label and icon/label content at 28, 32 and 36 px.

## Native Iced specimen

The example imports the same palette and primitive implementation as mira, with static fixture data. It runs independently
of the production application and does not start terminals, connect to herdr, execute Git, or access
credentials. Changes, History, button variants, density, inputs, badges, status, and empty states are
present. Search and Files tabs are navigation placeholders, not implementations of those workflows.

| Capture | Configuration |
|---|---|
| [Tech Light](mira-tech-light.png) | Palette 0, 360 px inspector |
| [Tech Dark](mira-tech-dark.png) | Palette 1, 360 px inspector |
| [Calm Light](mira-calm-light.png) | Palette 2, 360 px inspector |
| [Calm Dark](mira-calm-dark.png) | Palette 3, 360 px inspector |
| [Narrow inspector](mira-narrow.png) | Palette 1, 240 px inspector |
| [Wide inspector](mira-wide.png) | Palette 0, 480 px inspector |
| [Dialog](mira-dialog.png) | Palette 0, confirmation specimen |
| [Busy action](mira-busy.png) | Native pointer interaction, simulated commit in progress |
| [Focused empty field](mira-input-focus.png) | Native keyboard interaction, empty commit disabled |

Captures are 1160 × 1040, scale 1, using the real Iced window and OpenGL renderer in an isolated Xvfb
display with temporary XDG config/state roots. They are not browser mockups. The display used software
GL, so these results do not establish GPU performance or typing latency on hardware.

The four captures were reviewed for hierarchy and readable states. Background and sidebar pixels
match `theme.rs` exactly in all four palettes. An independent X11 window capture of the 240 px case
matched the Iced screenshot byte-for-byte in pixel values. Reference badges wrap at narrow widths;
Pull, Push, Stage, and the inspector width controls remain visible.

Native pointer/keyboard smoke checks exercised staging, the simulated busy/completed commit, field
focus and clearing, opening the dialog, Escape dismissal, and changing the palette. The busy and
focused-field screenshots preserve representative states. This checks the example's behavior;
production Git and confirmation workflows remain governed by their existing implementation.

## Reproduce

```sh
WGPU_BACKEND=gl cargo run --example design_system
```

Use the palette buttons and inspector-width choices to compare variants. Click Stage all, Commit,
or Destructive to inspect the local specimen states. Clear the message to inspect a disabled commit.
Escape closes the dialog.

For a deterministic native screenshot on a machine with Xvfb:

```sh
xvfb-run -a -s '-screen 0 1280x1200x24' \
  env -u WAYLAND_DISPLAY WINIT_UNIX_BACKEND=x11 WGPU_BACKEND=gl \
  MIRA_DESIGN_THEME=1 MIRA_DESIGN_WIDTH=360 \
  MIRA_DESIGN_CAPTURE="$PWD/docs/design-reference/mira-tech-dark.png" \
  cargo run --example design_system
```

The screenshot destination's parent directory must exist. `MIRA_DESIGN_DIALOG=1` starts with the
confirmation specimen open. A capture exits after saving; without a capture destination the example
stays open for interaction.

## Checks and boundaries

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and
  `cargo build --release` passed.
- The example is marked `test = true` in Cargo.toml, so the normal test command also runs its contrast,
  wrapping, and layout checks alongside the shared palette tests.
- Button-label contrast is at least 4.5:1 for every variant in all four palettes, including hover,
  pressed, and disabled states. Existing palette text contrast checks also pass.
- Layout checks cover 240/360/480 px inspectors; long-reference checks also exercise smaller content
  widths of 140/208/328 px.
- The existing 90-second herdr E2E read test passed against the active session and observed the working
  agent. It did not exercise a restart or a completed agent turn.

These captures record validation of the shared design in the product; they do not establish user
acceptance of the final appearance. Full button keyboard navigation and
assistive-technology parity remain outside this migration; see the Iced limitations in `design.md`.
Scale 2× and alternative fonts were checked in Xvfb; hardware latency and Wayland scale behavior
were not benchmarked. The component example's Search/Files placeholders remain intentional; their
real product views are exercised by the separate native product captures above.
