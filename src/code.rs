//! Structural code reading, using the same TextMate grammars as iced's syntax highlighter.
//! No language server or second grammar bundle is loaded.

use iced::advanced::text::highlighter::Format;
use iced::widget::text_editor::Position;
use iced::{Color, Font};
use std::ops::Range;
use std::sync::LazyLock;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use two_face::re_exports::syntect::{highlighting, parsing};

static SYNTAXES: LazyLock<parsing::SyntaxSet> = LazyLock::new(two_face::syntax::extra_no_newlines);
static THEMES: LazyLock<highlighting::ThemeSet> = LazyLock::new(highlighting::ThemeSet::load_defaults);
static IGNORED: LazyLock<Vec<parsing::Scope>> = LazyLock::new(|| {
    ["comment", "string", "constant.other.regex"].into_iter().filter_map(|s| parsing::Scope::new(s).ok()).collect()
});
// TextMate grammars use both generic interpolation and embedded/template scopes.
static EMBEDDED: LazyLock<Vec<parsing::Scope>> = LazyLock::new(|| {
    ["meta.interpolation", "meta.template.expression", "meta.embedded"]
        .into_iter()
        .filter_map(|s| parsing::Scope::new(s).ok())
        .collect()
});

const CHECKPOINT_LINES: usize = 50;
const MAX_CHECKPOINTS: usize = 256;
const MAX_DEPTH: usize = 1024;
const MAX_BRACKETS: usize = 100_000;

#[derive(Debug, Clone, Copy)]
pub struct Pair {
    pub open: Position,
    pub close: Option<Position>,
    pub depth: usize,
    parent: Option<usize>,
}

#[derive(Debug)]
struct Token {
    position: Position,
    pair: usize,
    opening: bool,
}

/// A bounded, immutable index built off the UI thread. Cursor queries are logarithmic.
#[derive(Debug, Default)]
pub struct Index {
    pairs: Vec<Pair>,
    tokens: Vec<Token>,
    pub indent: usize,
}

fn key(p: Position) -> (usize, usize) {
    (p.line, p.column)
}

impl Index {
    pub fn parse(text: &str, token: &str, cancel: &AtomicBool) -> Result<Option<Arc<Self>>, String> {
        let mut parser = Parser::new(token);
        let mut index = Self { indent: 4, ..Self::default() };
        let mut stack: Vec<(u8, usize)> = Vec::new();
        let mut exceeded = false;
        let mut min_indent = 8;
        for (line_no, line) in text.lines().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            let spaces = line.bytes().take_while(|&b| b == b' ').count();
            if !line.trim().is_empty() && (2..=8).contains(&spaces) {
                min_indent = min_indent.min(spaces);
            }
            parser.line(line, |range, scopes| {
                if !is_code(scopes) || exceeded {
                    return;
                }
                for column in range {
                    let byte = line.as_bytes()[column];
                    let position = Position { line: line_no, column };
                    match byte {
                        b'(' | b'[' | b'{' => {
                            if stack.len() >= MAX_DEPTH || index.tokens.len() >= MAX_BRACKETS {
                                exceeded = true;
                                return;
                            }
                            let pair = index.pairs.len();
                            index.pairs.push(Pair {
                                open: position,
                                close: None,
                                depth: stack.len(),
                                parent: stack.last().map(|&(_, id)| id),
                            });
                            stack.push((byte, pair));
                            index.tokens.push(Token { position, pair, opening: true });
                        }
                        b')' | b']' | b'}' => {
                            let Some(&(open, pair)) = stack.last() else { continue };
                            if closing(open) != byte {
                                continue;
                            }
                            if index.tokens.len() >= MAX_BRACKETS {
                                exceeded = true;
                                return;
                            }
                            stack.pop();
                            index.pairs[pair].close = Some(position);
                            index.tokens.push(Token { position, pair, opening: false });
                        }
                        _ => {}
                    }
                }
            })?;
            if exceeded {
                return Err("Code guides exceed the 100000-bracket or 1024-level limit".into());
            }
        }
        index.indent = if min_indent <= 3 { 2 } else { 4 };
        Ok(Some(Arc::new(index)))
    }

    /// Adjacent brackets take precedence; inside a block, use its innermost pair.
    pub fn active(&self, cursor: iced::widget::text_editor::Cursor) -> Option<Pair> {
        let position = cursor
            .selection
            .filter(|s| s.line == cursor.position.line && s.column.abs_diff(cursor.position.column) == 1)
            .map_or(cursor.position, |s| if s.column < cursor.position.column { s } else { cursor.position });
        let at = self.tokens.partition_point(|t| key(t.position) < key(position));
        let adjacent = self.tokens.get(at).filter(|t| t.position == position).or_else(|| {
            at.checked_sub(1)
                .and_then(|i| self.tokens.get(i))
                .filter(|t| t.position.line == position.line && t.position.column + 1 == position.column)
        });
        let mut pair = if let Some(token) = adjacent {
            Some(token.pair)
        } else {
            let previous = at.checked_sub(1).and_then(|i| self.tokens.get(i))?;
            if previous.opening { Some(previous.pair) } else { self.pairs[previous.pair].parent }
        };
        while let Some(id) = pair {
            let p = self.pairs[id];
            if p.close.is_some() {
                return Some(p);
            }
            pair = p.parent;
        }
        None
    }
}

#[derive(Default)]
pub struct Structure {
    pub index: Option<Arc<Index>>,
    pub indent: Option<usize>,
    request: Option<Arc<AtomicBool>>,
}

impl Structure {
    pub fn request(&mut self) -> Option<Arc<AtomicBool>> {
        if self.index.is_some() || self.request.is_some() {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.request = Some(Arc::clone(&cancel));
        Some(cancel)
    }

    pub fn cancel(&mut self) {
        if let Some(cancel) = self.request.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn invalidate(&mut self) {
        self.cancel();
        self.index = None;
    }
}

impl Drop for Structure {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn syntax(token: &str) -> &'static parsing::SyntaxReference {
    SYNTAXES.find_syntax_by_token(token).unwrap_or_else(|| SYNTAXES.find_syntax_plain_text())
}

/// The innermost string/comment wins, except an embedded interpolation expression.
fn is_code(scopes: &[parsing::Scope]) -> bool {
    for &scope in scopes.iter().rev() {
        if IGNORED.iter().any(|s| s.is_prefix_of(scope)) {
            return false;
        }
        if EMBEDDED.iter().any(|s| s.is_prefix_of(scope)) {
            return true;
        }
    }
    true
}

#[derive(Clone)]
struct Parser {
    state: parsing::ParseState,
    scopes: parsing::ScopeStack,
}

impl Parser {
    fn new(token: &str) -> Self {
        Self { state: parsing::ParseState::new(syntax(token)), scopes: parsing::ScopeStack::new() }
    }

    /// Ranges are UTF-8 byte offsets, as required by iced and the editor cursor.
    fn line(&mut self, line: &str, mut visit: impl FnMut(Range<usize>, &[parsing::Scope])) -> Result<(), String> {
        let ops = self.state.parse_line(line, &SYNTAXES).map_err(|e| e.to_string())?;
        let mut start = 0;
        for (end, op) in ops {
            if start < end {
                visit(start..end, self.scopes.as_slice());
            }
            self.scopes.apply(&op).map_err(|e| e.to_string())?;
            start = end;
        }
        if start < line.len() {
            visit(start..line.len(), self.scopes.as_slice());
        }
        Ok(())
    }
}

fn closing(open: u8) -> u8 {
    match open {
        b'(' => b')',
        b'[' => b']',
        _ => b'}',
    }
}

#[derive(Clone, PartialEq)]
pub struct Settings {
    pub token: String,
    pub theme: iced::highlighter::Theme,
    pub colors: [Color; 3],
    pub error: Color,
}

#[derive(Clone)]
struct State {
    parser: Parser,
    brackets: Vec<u8>,
    overflow: usize,
}

fn bracket(brackets: &mut Vec<u8>, overflow: &mut usize, byte: u8, settings: &Settings) -> Option<Color> {
    match byte {
        b'(' | b'[' | b'{' => {
            let depth = brackets.len() + *overflow;
            if brackets.len() < MAX_DEPTH && *overflow == 0 {
                brackets.push(byte);
            } else {
                *overflow = overflow.saturating_add(1);
            }
            Some(settings.colors[depth % settings.colors.len()])
        }
        b')' | b']' | b'}' => {
            if *overflow > 0 {
                *overflow -= 1;
            } else if brackets.last().is_some_and(|&b| closing(b) == byte) {
                brackets.pop();
            } else {
                return Some(settings.error);
            }
            Some(settings.colors[(brackets.len() + *overflow) % settings.colors.len()])
        }
        _ => None,
    }
}

/// Incremental highlighter with bounded checkpoints, including bracket depth.
pub struct Highlighter {
    settings: Settings,
    syntax: highlighting::Highlighter<'static>,
    state: State,
    checkpoints: Vec<State>,
    line: usize,
}

fn syntax_theme(theme: iced::highlighter::Theme) -> &'static highlighting::Theme {
    use iced::highlighter::Theme::*;
    let key = match theme {
        SolarizedDark => "Solarized (dark)",
        Base16Mocha => "base16-mocha.dark",
        Base16Ocean => "base16-ocean.dark",
        Base16Eighties => "base16-eighties.dark",
        InspiredGitHub => "InspiredGitHub",
    };
    // All five names are keys in syntect's bundled default ThemeSet (tested below).
    &THEMES.themes[key]
}

impl iced::advanced::text::Highlighter for Highlighter {
    type Settings = Settings;
    type Highlight = Format<Font>;
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, Self::Highlight)>;

    fn new(settings: &Settings) -> Self {
        let state = State { parser: Parser::new(&settings.token), brackets: Vec::new(), overflow: 0 };
        Self {
            settings: settings.clone(),
            syntax: highlighting::Highlighter::new(syntax_theme(settings.theme)),
            checkpoints: vec![state.clone()],
            state,
            line: 0,
        }
    }

    fn update(&mut self, settings: &Settings) {
        *self = Self::new(settings);
    }

    fn change_line(&mut self, line: usize) {
        let checkpoint = (line / CHECKPOINT_LINES).min(self.checkpoints.len() - 1);
        self.state = self.checkpoints[checkpoint].clone();
        self.checkpoints.truncate(checkpoint + 1);
        self.line = checkpoint * CHECKPOINT_LINES;
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        let mut spans = Vec::new();
        let State { parser, brackets, overflow } = &mut self.state;
        let parsed = parser.line(line, |range, scopes| {
            let style = self.syntax.style_mod_for_stack(scopes);
            let base = Format {
                color: style.foreground.map(|c| Color::from_rgba8(c.r, c.g, c.b, f32::from(c.a) / 255.0)),
                // Preserve mira's readable medium monospace, including for syntax tokens.
                font: style.font_style.filter(|s| !s.is_empty()).map(|s| Font {
                    weight: if s.contains(highlighting::FontStyle::BOLD) {
                        iced::font::Weight::Bold
                    } else {
                        crate::theme::mono_font().weight
                    },
                    style: if s.contains(highlighting::FontStyle::ITALIC) {
                        iced::font::Style::Italic
                    } else {
                        iced::font::Style::Normal
                    },
                    ..crate::theme::mono_font()
                }),
            };
            if !is_code(scopes) {
                spans.push((range, base));
                return;
            }
            let mut start = range.start;
            for i in range.clone() {
                if let Some(color) = bracket(brackets, overflow, line.as_bytes()[i], &self.settings) {
                    if start < i {
                        spans.push((start..i, base));
                    }
                    spans.push((i..i + 1, Format { color: Some(color), font: None }));
                    start = i + 1;
                }
            }
            if start < range.end {
                spans.push((start..range.end, base));
            }
        });
        if parsed.is_err() {
            // A grammar failure must leave the file readable and editable.
            spans.clear();
        }
        self.line += 1;
        if self.line.is_multiple_of(CHECKPOINT_LINES) && self.checkpoints.len() < MAX_CHECKPOINTS {
            self.checkpoints.push(self.state.clone());
        }
        spans.into_iter()
    }

    fn current_line(&self) -> usize {
        self.line
    }
}

/// Find an adjacent bracket's pair, or the innermost pair enclosing the cursor.
/// Called on a worker only on request, never on every cursor move or redraw.
pub fn matching(text: &str, token: &str, cursor: Position) -> Result<Option<Position>, String> {
    let mut parser = Parser::new(token);
    let mut stack: Vec<(u8, Position)> = Vec::new();
    let mut adjacent = None;
    let mut enclosing = None;
    let mut too_deep = false;
    let line_text = text.lines().nth(cursor.line).unwrap_or("");
    let near = [Some(cursor.column), cursor.column.checked_sub(1)]
        .into_iter()
        .flatten()
        .find(|&col| line_text.as_bytes().get(col).is_some_and(|b| b"()[]{}".contains(b)));
    for (line_no, line) in text.lines().enumerate() {
        parser.line(line, |range, scopes| {
            if !is_code(scopes) || too_deep {
                return;
            }
            for column in range {
                let byte = line.as_bytes()[column];
                let position = Position { line: line_no, column };
                match byte {
                    b'(' | b'[' | b'{' => {
                        if stack.len() == MAX_DEPTH {
                            too_deep = true;
                            return;
                        }
                        stack.push((byte, position));
                    }
                    b')' | b']' | b'}' => {
                        let Some(&(open, from)) = stack.last() else { continue };
                        if closing(open) != byte {
                            continue;
                        }
                        stack.pop();
                        if (from.line == cursor.line && near == Some(from.column))
                            || (position.line == cursor.line && near == Some(position.column))
                        {
                            adjacent = Some(if position.line == cursor.line && near == Some(position.column) {
                                from
                            } else {
                                position
                            });
                        }
                        if (from.line, from.column) <= (cursor.line, cursor.column)
                            && (cursor.line, cursor.column) <= (position.line, position.column)
                            && enclosing.is_none_or(|(old, _): (Position, Position)| {
                                (old.line, old.column) < (from.line, from.column)
                            })
                        {
                            enclosing = Some((from, position));
                        }
                    }
                    _ => {}
                }
            }
        })?;
        if adjacent.is_some() {
            return Ok(adjacent);
        }
        if near.is_none()
            && let Some((_, end)) = enclosing
        {
            return Ok(Some(end));
        }
        if too_deep {
            break;
        }
    }
    if too_deep {
        return Err("Bracket nesting exceeds the 1024-level reading limit".into());
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::text::Highlighter as _;

    #[test]
    fn structural_index_tracks_adjacent_and_enclosing_pairs_without_literal_brackets() {
        use iced::widget::text_editor::Cursor;
        let source = "fn f() {\n  call([1, 2]); // }\n  let s = \"}\";\n}";
        let index = Index::parse(source, "rs", &AtomicBool::new(false)).unwrap().unwrap();
        let at = |line, column| index.active(Cursor { position: Position { line, column }, selection: None }).unwrap();
        assert_eq!(at(0, 8).close, Some(Position { line: 3, column: 0 }));
        assert_eq!(at(3, 1).open, Position { line: 0, column: 7 });
        assert_eq!(at(1, 8).open, Position { line: 1, column: 7 });
        assert_eq!(at(1, 8).close, Some(Position { line: 1, column: 12 }));
        assert_eq!(at(2, 8).open, Position { line: 0, column: 7 });
        assert_eq!(index.indent, 2);
        assert!(Index::parse(source, "rs", &AtomicBool::new(true)).unwrap().is_none());
    }

    fn settings(token: &str) -> Settings {
        let p = &crate::theme::PALETTES[0];
        Settings { token: token.into(), theme: p.syntax, colors: p.brackets(), error: p.error_text }
    }

    fn colors(h: &mut Highlighter, line: &str) -> Vec<(usize, Color)> {
        h.highlight_line(line)
            .filter_map(|(r, f)| {
                (r.len() == 1 && b"()[]{}".contains(&line.as_bytes()[r.start])).then(|| (r.start, f.color.unwrap()))
            })
            .collect()
    }

    #[test]
    fn pairs_share_their_nesting_color_across_lines() {
        let s = settings("rs");
        let mut h = Highlighter::new(&s);
        assert_eq!(colors(&mut h, "{(["), vec![(0, s.colors[0]), (1, s.colors[1]), (2, s.colors[2])]);
        assert_eq!(colors(&mut h, "])}"), vec![(0, s.colors[2]), (1, s.colors[1]), (2, s.colors[0])]);
        assert_eq!(colors(&mut h, ")"), vec![(0, s.error)]);
    }

    #[test]
    fn grammar_excludes_strings_comments_regex_and_characters() {
        for (token, source) in [
            ("rs", "fn main() { let c = '}'; let s = r###\"[({\"###; /* } */\n}"),
            ("ts", "function f() { const s = '[}'; const r = /[{}]/; // }\n}"),
            ("py", "def f():\n    s = ''' {\n    } '''\n    return [1] # }"),
            ("json", "{\"text\": \"} [ (\", \"items\": [1]}"),
        ] {
            let mut h = Highlighter::new(&settings(token));
            for line in source.lines() {
                let _ = h.highlight_line(line).collect::<Vec<_>>();
            }
            assert!(h.state.brackets.is_empty(), "{token}: {:?}", h.state.brackets);
        }
    }

    #[test]
    fn interpolation_expressions_remain_code_inside_template_strings() {
        let source = "const s = `text { ${call({value: [1]})} }`;";
        let start = source.find("{value").unwrap();
        let end = source.find("})").unwrap();
        assert_eq!(
            matching(source, "ts", Position { line: 0, column: start }).unwrap(),
            Some(Position { line: 0, column: end })
        );
        let literal = source.find("text {").unwrap() + 5;
        assert_eq!(matching(source, "ts", Position { line: 0, column: literal }).unwrap(), None);
    }

    #[test]
    fn all_bundled_syntax_themes_and_theme_changes_are_supported() {
        let mut s = settings("rs");
        let mut h = Highlighter::new(&s);
        for &theme in iced::highlighter::Theme::ALL {
            s.theme = theme;
            h.update(&s);
            assert_eq!(h.current_line(), 0);
            assert_eq!(colors(&mut h, "{}"), vec![(0, s.colors[0]), (1, s.colors[0])]);
        }
    }

    #[test]
    fn real_editor_renders_all_three_bracket_colors_in_each_palette() {
        use iced::advanced::{
            Layout, Renderer as _, Shell, clipboard, layout, renderer, renderer::Headless, widget::Tree,
        };
        use iced::widget::text_editor;
        use iced::{Element, Event, Length, Pixels, Rectangle, Size, mouse};
        use std::future::Future as _;

        let mut new = std::pin::pin!(<iced::Renderer as Headless>::new(
            crate::theme::mono_font(),
            Pixels(18.0),
            Some("tiny-skia")
        ));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(Some(mut renderer)) = new.as_mut().poll(&mut cx) else {
            panic!("headless renderer")
        };
        let content = text_editor::Content::with_text(
            "fn review() {\n    call([1, 2, 3]);\n    // Literal braces stay comments: { [ (\n    let text = \"}])\";\n}\n",
        );
        for (i, palette) in crate::theme::PALETTES.iter().enumerate() {
            let size = Size::new(650_u32, 200_u32);
            let viewport = Rectangle::with_size(Size::new(size.width as f32, size.height as f32));
            renderer.reset(viewport);
            let mut element: Element<'_, ()> = text_editor(&content)
                .font(crate::theme::mono_font())
                .size(18)
                .height(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::None)
                .on_action(|_| ())
                .style(|_, _| text_editor::Style {
                    background: palette.bg.into(),
                    border: iced::Border::default(),
                    value: palette.fg,
                    placeholder: palette.muted,
                    selection: palette.selection,
                })
                .highlight_with::<Highlighter>(
                    Settings {
                        token: "rs".into(),
                        theme: palette.syntax,
                        colors: palette.brackets(),
                        error: palette.error_text,
                    },
                    |format, _| *format,
                )
                .into();
            let mut tree = Tree::new(&element);
            let node =
                element.as_widget_mut().layout(&mut tree, &renderer, &layout::Limits::new(Size::ZERO, viewport.size()));
            let layout = Layout::new(&node);
            let mut messages = Vec::new();
            element.as_widget_mut().update(
                &mut tree,
                &Event::Window(iced::window::Event::RedrawRequested(std::time::Instant::now())),
                layout,
                mouse::Cursor::Unavailable,
                &renderer,
                &mut clipboard::Null,
                &mut Shell::new(&mut messages),
                &viewport,
            );
            element.as_widget().draw(
                &tree,
                &mut renderer,
                &iced::Theme::Dark,
                &renderer::Style { text_color: palette.fg },
                layout,
                mouse::Cursor::Unavailable,
                &viewport,
            );
            let bytes = renderer.screenshot(size, 1.0, palette.bg);
            for color in palette.brackets() {
                let [r, g, b, _] = color.into_rgba8();
                assert!(
                    bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|p| p[0].abs_diff(r) < 8 && p[1].abs_diff(g) < 8 && p[2].abs_diff(b) < 8),
                    "{}: missing bracket color {color:?}",
                    palette.name
                );
            }
            if let Ok(dir) = std::env::var("MIRA_CODE_PREVIEW_DIR") {
                std::fs::create_dir_all(&dir).unwrap();
                image::save_buffer(
                    std::path::Path::new(&dir).join(format!("brackets-{i}.png")),
                    &bytes,
                    size.width,
                    size.height,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn editing_replays_parser_and_depth_from_the_previous_checkpoint() {
        let s = settings("rs");
        let mut h = Highlighter::new(&s);
        let mut lines: Vec<String> = (0..125).map(|_| "    call([1]);".into()).collect();
        lines[0] = "fn f() {".into();
        lines[49] = "/* a multiline comment {".into();
        lines[53] = "} */".into();
        lines[124] = "}".into();
        for line in &lines {
            let _ = h.highlight_line(line).collect::<Vec<_>>();
        }
        lines[52] = "*/ { // edited inside an existing comment".into();
        h.change_line(52);
        let mut fresh = Highlighter::new(&s);
        let restart = h.current_line();
        for (i, line) in lines.iter().enumerate() {
            let expected: Vec<_> = fresh.highlight_line(line).collect();
            if i >= restart {
                assert_eq!(h.highlight_line(line).collect::<Vec<_>>(), expected, "line {i}");
            }
        }
    }

    #[test]
    fn checkpoint_memory_is_bounded_and_distant_edits_can_replay() {
        let s = settings("txt");
        let mut h = Highlighter::new(&s);
        let _ = h.highlight_line("{").collect::<Vec<_>>();
        for _ in 1..13_000 {
            let _ = h.highlight_line("body").collect::<Vec<_>>();
        }
        assert_eq!(h.checkpoints.len(), MAX_CHECKPOINTS);
        h.change_line(12_999);
        assert_eq!(h.current_line(), (MAX_CHECKPOINTS - 1) * CHECKPOINT_LINES);
        while h.current_line() < 13_000 {
            let _ = h.highlight_line("body").collect::<Vec<_>>();
        }
        assert_eq!(colors(&mut h, "}"), vec![(0, s.colors[0])]);
    }

    #[test]
    fn matching_uses_utf8_byte_columns_and_ignores_literal_braces() {
        let source = "fn f() {\r\n  let é = \"}\";\r\n  call([1]);\r\n}";
        assert_eq!(
            matching(source, "rs", Position { line: 0, column: 7 }).unwrap(),
            Some(Position { line: 3, column: 0 })
        );
        assert_eq!(
            matching(source, "rs", Position { line: 3, column: 1 }).unwrap(),
            Some(Position { line: 0, column: 7 })
        );
        assert_eq!(matching(source, "rs", Position { line: 1, column: 12 }).unwrap(), None);
        assert_eq!(
            matching(source, "rs", Position { line: 1, column: 6 }).unwrap(),
            Some(Position { line: 3, column: 0 })
        );
        assert_eq!(
            matching("é({[]})", "rs", Position { line: 0, column: 3 }).unwrap(),
            Some(Position { line: 0, column: 6 })
        );
    }

    #[test]
    fn incomplete_or_mismatched_brackets_are_safe() {
        assert_eq!(matching("{ ([)]", "rs", Position { line: 0, column: 0 }).unwrap(), None);
        assert_eq!(matching("// {}", "rs", Position { line: 0, column: 3 }).unwrap(), None);
        let deep = "(".repeat(MAX_DEPTH + 1);
        assert!(matching(&deep, "txt", Position { line: 0, column: 0 }).is_err());
        let mut h = Highlighter::new(&settings("txt"));
        let _ = h.highlight_line(&deep).collect::<Vec<_>>();
        assert_eq!(h.state.brackets.len(), MAX_DEPTH);
        let _ = h.highlight_line(&")".repeat(MAX_DEPTH + 1)).collect::<Vec<_>>();
        assert!(h.state.brackets.is_empty());
    }
}
