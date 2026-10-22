# Reading code structure

mira's editor supports syntax highlighting, line numbers, a current-line gutter marker,
path breadcrumbs, find, quick open, go to line, bracket pair colors, and a matching-bracket
action. Bracket colors and navigation share TextMate grammars with syntax highlighting.
The grammar bundle already shipped with iced; no language server, extension host or new
transitive dependency is needed. Parsing checkpoints and bracket nesting have fixed limits.

## Features to adopt from Code OSS / VSCodium

The upstream references describe VS Code's editor. VSCodium builds Code OSS; availability
of language-powered features additionally depends on its installed language support.

| Feature | Reading benefit | Fit for mira |
| --- | --- | --- |
| Bracket pair colors | Identify corresponding `{}`, `[]`, `()` through nested expressions. | Implemented, with three palette-specific colors and incremental parsing. |
| Go to matching bracket | Reach the start/end of a pair without scrolling through a long block. | Implemented: editor-only `Ctrl+Shift+\` and the **{ }** button. The destination is selected. |
| Indentation and active bracket guides | Vertical lines connect indentation levels or the active pair. | Next priority. iced 0.14's public text-editor API does not expose glyph bounds or horizontal scroll, so a correctly aligned overlay needs additional integration; a fixed-width approximation would drift with tabs or sideways scrolling. |
| Active-pair highlighting | Highlight both endpoints beside the cursor. | Next priority alongside guides. The current action selects its destination; automatic endpoint highlighting is not implemented. |
| Sticky Scroll | Keep enclosing function/class opening lines visible while scrolling. | Useful for long review sessions. Start with a grammar/indentation model; do not claim semantic function names without a symbol provider. Needs the same accurate viewport integration as guides. |
| Code folding | Collapse unrelated blocks while reviewing a function. | Useful, but iced's public editor has no fold/projection model. Hiding lines externally would break editing, selection, undo and gutter alignment. Requires a supported editor integration first. |
| Symbol breadcrumbs / outline | Show the containing function/class and navigate declarations. | Useful later. File-path breadcrumbs exist; reliable symbols require an optional language provider, rather than regex guesses for every language. |
| Render whitespace | Distinguish tabs, spaces and trailing whitespace. | Useful for indentation review. Needs glyph decoration support so markers do not modify the actual file. |

Indentation guides and active-pair highlighting give the most immediate additional value
for reading structure. Folding and symbol navigation should follow only with a sound editor
model; the terminal remains the primary workspace.

## Verified references

- [Code navigation: bracket matching and pair colorization](https://code.visualstudio.com/docs/editing/editingevolved#_bracket-matching).
- [Editor UI: Sticky Scroll, indentation guides and breadcrumbs](https://code.visualstudio.com/docs/editing/getting-started/userinterface#_sticky-scroll).
- [Basic editing: folding and indentation](https://code.visualstudio.com/docs/editing/codebasics#_folding).
- [VSCodium's relationship to Code OSS](https://github.com/VSCodium/vscodium#why-does-this-exist).
- Installed source checked: iced_widget 0.14.2 `text_editor::highlight_with` and `Content`,
  iced_core 0.14.0 `text::Highlighter`, two-face 0.4.5's grammar bundle and syntect 5.3.0
  `ParseState`, `ScopeStack`, and `Highlighter`. These are public APIs; dependencies are
  neither patched nor vendored.
