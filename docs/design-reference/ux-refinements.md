# Small UX refinements after 0.10.1

Reviewed 2026-10-23. The product brief is to improve clarity without adding panels, dependencies,
animations or redundant controls. These sources inform decisions; their browser or Windows APIs are
not imported into Mira. The governing rules remain in [`design.md`](../../design.md).

## Evidence and decisions

| Source | Useful principle | Mira decision |
|---|---|---|
| [W3C: Focus Appearance](https://www.w3.org/WAI/WCAG22/Understanding/focus-appearance) | A focus indicator needs enough area and contrast to be found reliably. The cited criterion is AAA. | Give existing fields and the commit editor a 2 px focus border, measured against the idle border and adjoining surfaces. Keep geometry unchanged. |
| [Fluent 2: Tooltip](https://fluent2.microsoft.design/components/web/react/core/tooltip/usage) | Hover help supplements the interface; essential information belongs in the visible UI. | Keep persistent labels and written states, identify required commit input, and describe the current blocker in disabled Commit help. |
| [Microsoft: Tooltip delay](https://learn.microsoft.com/en-us/windows/win32/controls/ttm-setdelaytime) | The documented default timing example waits half a second before showing a tooltip. | Choose 500 ms for ordinary hover help. Retain 700 ms for full list-row details. These are Mira's fixed choices, not a reading of OS settings. |
| [NN/G: Empty states](https://www.nngroup.com/articles/empty-state-interface-design/) | Empty content should explain what happened and how to proceed; loading is a different state. | Use short messages for clean Git state, initial history and completed searches without matches. Never replace loading or errors with an empty-success message. |
| [VS Code: Views](https://code.visualstudio.com/api/ux-guidelines/views) | Keep view actions and empty-view content limited, and guide the next task. | Explain the next step in ordinary text; add no new buttons, cards, illustrations or views. |

The desktop timing is an adaptation, not a universal latency recommendation. The W3C page concerns
web accessibility; the native implementation borrows measurable visual goals and does not establish
full accessibility conformance. In particular, Iced's button keyboard-focus limitations still apply.

## Implementation

- `src/theme.rs` owns a `focus` role in each LAGC palette. Three match their existing accent; Calm Light
  uses its existing darker green (`576A3C`) so focus contrasts with its idle border above 3:1. Its former
  accent-to-idle-border contrast was approximately 2.92:1.
- `src/theme/ui.rs` shares the field border, tooltip timing and unboxed empty-state layout. Both field
  types use the same focus rule, and the component gallery reuses the empty-state implementation.
- `src/view.rs` uses the shared hover helper for existing hints. Long row details keep the longer delay.
  The Commit label identifies required input, while its existing tooltip prioritizes busy Git, conflicts,
  a clean tree and an empty message before describing the commit operation.
- Search keeps the existing results, debounce, generation checks and keyboard routes. Its completed
  empty view suggests a different term or search options; its count remains in the header.

Native keyboard validation also reproduced an existing 0.10.1 bug: X11 can attach printable comma text
to Ctrl+comma, and Iced's editor fallback inserted it. File and commit editors now dispatch Settings
before that fallback. A regression covers the command, normal comma input and an unfocused editor.

No Git guards, terminal input, provider integration or dependencies were changed.
The removed selector frames remain removed, and the existing 28/32/36 px control heights remain.

## Verification

The focus regression check reproduced the previous 1 px stroke before the change. It now checks the
2 px stroke and at least 3:1 contrast against field, panel and idle-border colors in all four palettes.
Native validation and final quality-gate results are recorded in the evidence README.
