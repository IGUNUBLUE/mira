//! Decorate iced's own editor through a recording renderer. It keeps the original
//! buffer, layout, input and undo behavior; geometry comes from its shaped glyphs.

use crate::{code, theme};
use iced::advanced::text::{Editor as _, Renderer as _};
use iced::advanced::{
    Clipboard, Layout, Shell, Widget, graphics, image, layout, renderer, text,
    widget::{Operation, Tree},
};
use iced::{
    Background, Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Size, Theme, Transformation, mouse,
};
use std::cell::RefCell;

type Editor = <iced::Renderer as text::Renderer>::Editor;
type Paragraph = <iced::Renderer as text::Renderer>::Paragraph;
pub type Content = iced::widget::text_editor::Content<Recording>;

/// Public-API adapter: iced 0.14 does not apply cosmic-text's horizontal offset
/// to editor drawing, caret/selection geometry or mouse hit testing.
#[derive(Default)]
pub struct ScrolledEditor(Editor);

impl text::Editor for ScrolledEditor {
    type Font = Font;
    fn with_text(value: &str) -> Self {
        Self(Editor::with_text(value))
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn cursor(&self) -> text::editor::Cursor {
        self.0.cursor()
    }
    fn selection(&self) -> text::editor::Selection {
        let offset = iced::Vector::new(-self.0.buffer().scroll().horizontal, 0.0);
        match self.0.selection() {
            text::editor::Selection::Caret(point) => text::editor::Selection::Caret(point + offset),
            text::editor::Selection::Range(rects) => {
                text::editor::Selection::Range(rects.into_iter().map(|r| r + offset).collect())
            }
        }
    }
    fn copy(&self) -> Option<String> {
        self.0.copy()
    }
    fn line(&self, index: usize) -> Option<text::editor::Line<'_>> {
        self.0.line(index)
    }
    fn line_count(&self) -> usize {
        self.0.line_count()
    }
    fn perform(&mut self, action: text::editor::Action) {
        let offset = iced::Vector::new(self.0.buffer().scroll().horizontal, 0.0);
        let action = match action {
            text::editor::Action::Click(point) => text::editor::Action::Click(point + offset),
            text::editor::Action::Drag(point) => text::editor::Action::Drag(point + offset),
            other => other,
        };
        self.0.perform(action);
    }
    fn move_to(&mut self, cursor: text::editor::Cursor) {
        self.0.move_to(cursor);
    }
    fn bounds(&self) -> Size {
        self.0.bounds()
    }
    fn min_bounds(&self) -> Size {
        self.0.min_bounds()
    }
    fn update(
        &mut self,
        bounds: Size,
        font: Font,
        size: Pixels,
        height: text::LineHeight,
        wrapping: text::Wrapping,
        highlighter: &mut impl text::Highlighter,
    ) {
        self.0.update(bounds, font, size, height, wrapping, highlighter);
    }
    fn highlight<H: text::Highlighter>(
        &mut self,
        font: Font,
        highlighter: &mut H,
        format: impl Fn(&H::Highlight) -> text::highlighter::Format<Font>,
    ) {
        self.0.highlight(font, highlighter, format);
    }
}

enum Command {
    Quad(renderer::Quad, Background),
    Editor(graphics::text::editor::Weak, Point, Color, Rectangle),
    Paragraph(graphics::text::paragraph::Weak, Point, Color, Rectangle),
    Text(text::Text, Point, Color, Rectangle),
    Layer(Rectangle),
    EndLayer,
    Transform(Transformation),
    EndTransform,
}

pub struct Recording {
    font: Font,
    size: Pixels,
    commands: Vec<Command>,
}

impl Recording {
    fn new(renderer: &iced::Renderer) -> Self {
        Self { font: renderer.default_font(), size: renderer.default_size(), commands: Vec::new() }
    }

    fn replay(self, renderer: &mut iced::Renderer, index: Option<&code::Index>, indent: usize, cache: &mut GuideCache) {
        use iced::advanced::Renderer as _;
        for command in self.commands {
            match command {
                Command::Quad(quad, background) => renderer.fill_quad(quad, background),
                Command::Editor(weak, position, color, clip) => {
                    if let Some(editor) = weak.upgrade() {
                        decorate(renderer, &editor, position, clip, index, indent, cache);
                        renderer.fill_editor(
                            &editor,
                            position - iced::Vector::new(editor.buffer().scroll().horizontal, 0.0),
                            color,
                            clip,
                        );
                    }
                }
                Command::Paragraph(weak, position, color, clip) => {
                    if let Some(paragraph) = weak.upgrade() {
                        renderer.fill_paragraph(&paragraph, position, color, clip);
                    }
                }
                Command::Text(text, position, color, clip) => renderer.fill_text(text, position, color, clip),
                Command::Layer(bounds) => renderer.start_layer(bounds),
                Command::EndLayer => renderer.end_layer(),
                Command::Transform(transform) => renderer.start_transformation(transform),
                Command::EndTransform => renderer.end_transformation(),
            }
        }
    }
}

impl renderer::Renderer for Recording {
    fn start_layer(&mut self, bounds: Rectangle) {
        self.commands.push(Command::Layer(bounds));
    }
    fn end_layer(&mut self) {
        self.commands.push(Command::EndLayer);
    }
    fn start_transformation(&mut self, t: Transformation) {
        self.commands.push(Command::Transform(t));
    }
    fn end_transformation(&mut self) {
        self.commands.push(Command::EndTransform);
    }
    fn fill_quad(&mut self, quad: renderer::Quad, background: impl Into<Background>) {
        self.commands.push(Command::Quad(quad, background.into()));
    }
    fn reset(&mut self, _: Rectangle) {
        self.commands.clear();
    }
    fn allocate_image(
        &mut self,
        _: &image::Handle,
        callback: impl FnOnce(Result<image::Allocation, image::Error>) + Send + 'static,
    ) {
        callback(Err(image::Error::Unsupported));
    }
}

impl text::Renderer for Recording {
    type Font = Font;
    type Editor = ScrolledEditor;
    type Paragraph = Paragraph;
    const ICON_FONT: Font = <iced::Renderer as text::Renderer>::ICON_FONT;
    const CHECKMARK_ICON: char = <iced::Renderer as text::Renderer>::CHECKMARK_ICON;
    const ARROW_DOWN_ICON: char = <iced::Renderer as text::Renderer>::ARROW_DOWN_ICON;
    const SCROLL_UP_ICON: char = <iced::Renderer as text::Renderer>::SCROLL_UP_ICON;
    const SCROLL_DOWN_ICON: char = <iced::Renderer as text::Renderer>::SCROLL_DOWN_ICON;
    const SCROLL_LEFT_ICON: char = <iced::Renderer as text::Renderer>::SCROLL_LEFT_ICON;
    const SCROLL_RIGHT_ICON: char = <iced::Renderer as text::Renderer>::SCROLL_RIGHT_ICON;
    const ICED_LOGO: char = <iced::Renderer as text::Renderer>::ICED_LOGO;
    fn default_font(&self) -> Font {
        self.font
    }
    fn default_size(&self) -> Pixels {
        self.size
    }
    fn fill_paragraph(&mut self, paragraph: &Paragraph, position: Point, color: Color, clip: Rectangle) {
        self.commands.push(Command::Paragraph(paragraph.downgrade(), position, color, clip));
    }
    fn fill_editor(&mut self, editor: &ScrolledEditor, position: Point, color: Color, clip: Rectangle) {
        self.commands.push(Command::Editor(editor.0.downgrade(), position, color, clip));
    }
    fn fill_text(&mut self, text: text::Text, position: Point, color: Color, clip: Rectangle) {
        self.commands.push(Command::Text(text, position, color, clip));
    }
}

struct Decorated<'a, M> {
    inner: Element<'a, M, Theme, Recording>,
    index: Option<&'a code::Index>,
    indent: usize,
}

pub fn editor<'a, M: 'a>(
    inner: impl Into<Element<'a, M, Theme, Recording>>,
    index: Option<&'a code::Index>,
    indent: usize,
) -> Element<'a, M> {
    Element::new(Decorated { inner: inner.into(), index, indent })
}

impl<M> Widget<M, Theme, iced::Renderer> for Decorated<'_, M> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<RefCell<GuideCache>>()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(RefCell::new(GuideCache::default()))
    }
    fn size(&self) -> Size<Length> {
        self.inner.as_widget().size()
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.inner)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.inner));
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        self.inner.as_widget_mut().layout(&mut tree.children[0], &Recording::new(renderer), limits)
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        self.inner.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            &Recording::new(renderer),
            clipboard,
            shell,
            viewport,
        );
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.inner.as_widget_mut().operate(&mut tree.children[0], layout, &Recording::new(renderer), operation);
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.inner.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, &Recording::new(renderer))
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let mut recording = Recording::new(renderer);
        self.inner.as_widget().draw(&tree.children[0], &mut recording, theme, style, layout, cursor, viewport);
        recording.replay(
            renderer,
            self.index,
            self.indent,
            &mut tree.state.downcast_ref::<RefCell<GuideCache>>().borrow_mut(),
        );
    }
}

#[derive(Default)]
struct GuideCache {
    prefix: Option<String>,
    x: f32,
}

impl GuideCache {
    fn measure(&mut self, prefix: &str, row: f32) -> Option<f32> {
        if prefix.len() > 4096 {
            return None;
        }
        if self.prefix.as_deref() != Some(prefix) {
            let mut measure = Editor::with_text(&format!("{prefix}|"));
            measure.update(
                Size::new(100_000.0, row),
                theme::mono_font(),
                Pixels(crate::app::CODE_SIZE as f32),
                text::LineHeight::Absolute(Pixels(row)),
                text::Wrapping::None,
                &mut text::highlighter::PlainText,
            );
            self.x = measure.buffer().layout_runs().next().and_then(|r| r.glyphs.last()).map_or(0.0, |g| g.x);
            self.prefix = Some(prefix.to_owned());
        }
        Some(self.x)
    }
}

fn decorate(
    renderer: &mut iced::Renderer,
    editor: &Editor,
    position: Point,
    clip: Rectangle,
    index: Option<&code::Index>,
    indent: usize,
    cache: &mut GuideCache,
) {
    use iced::advanced::Renderer as _;
    let buffer = editor.buffer();
    let scroll = buffer.scroll();
    let row = buffer.metrics().line_height;
    let active = index.and_then(|i| i.active(editor.cursor()));
    renderer.with_layer(clip, |renderer| {
        // Whitespace glyphs give exact guide positions, including tabs and horizontal scroll.
        for run in buffer.layout_runs() {
            let mut columns = 0;
            // Bound decoration work even for pathological whitespace-only lines.
            for glyph in run.glyphs.iter().take(4096) {
                let Some(piece) = run.text.get(glyph.start..glyph.end) else { continue };
                if !piece.bytes().all(|b| b == b' ' || b == b'\t') {
                    break;
                }
                let tab = usize::from(buffer.tab_width()).max(1);
                let advance = if piece == "\t" { tab - columns % tab } else { piece.len() };
                for column in (columns..columns + advance).filter(|c| c.is_multiple_of(indent.max(1))) {
                    let x =
                        position.x + glyph.x + glyph.w * (column - columns) as f32 / advance as f32 - scroll.horizontal;
                    if x < clip.x || x > clip.x + clip.width {
                        continue;
                    }
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(
                                Point::new(x, position.y + run.line_top),
                                Size::new(1.0, run.line_height),
                            ),
                            ..Default::default()
                        },
                        theme::p().border,
                    );
                }
                columns += advance;
            }
        }
        let Some(pair) = active else { return };
        let Some(close) = pair.close else { return };
        let color = theme::p().brackets()[pair.depth % 3];
        let mut open_rect = None;
        let mut close_rect = None;
        for run in buffer.layout_runs() {
            for target in [pair.open, close] {
                if run.line_i != target.line {
                    continue;
                }
                if let Some(glyph) = run.glyphs.iter().find(|g| g.start <= target.column && target.column < g.end) {
                    let rect = Rectangle::new(
                        Point::new(position.x + glyph.x - scroll.horizontal - 1.0, position.y + run.line_top),
                        Size::new(glyph.w + 2.0, run.line_height),
                    );
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: rect,
                            border: iced::Border { color, width: 1.5, radius: 2.0.into() },
                            ..Default::default()
                        },
                        color.scale_alpha(0.12),
                    );
                    if target == pair.open {
                        open_rect = Some(rect);
                    } else {
                        close_rect = Some(rect);
                    }
                }
            }
        }
        if pair.open.line == close.line {
            return;
        }
        // Shape only the closing line's indentation when its glyph is offscreen.
        // This tiny editor shares iced's font system and tab policy, not the file buffer.
        let line = buffer.lines.get(close.line).map_or("", |l| l.text());
        let indent = line.bytes().take_while(|&b| b == b' ' || b == b'\t').count().min(close.column);
        let Some(guide_x) = cache.measure(&line[..indent], row).map(|x| position.x + x - scroll.horizontal) else {
            return;
        };
        let y = |line: usize| position.y + (line as f32 - scroll.line as f32) * row - scroll.vertical;
        let top = open_rect.map_or_else(|| y(pair.open.line) + row, |r| r.y + r.height);
        let bottom = close_rect.map_or_else(|| y(close.line), |r| r.y);
        if bottom > top {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle::new(Point::new(guide_x, top), Size::new(1.5, bottom - top)),
                    ..Default::default()
                },
                color,
            );
        }
        for (rect, at_bottom) in [(open_rect, true), (close_rect, false)] {
            if let Some(rect) = rect {
                let end = rect.x + rect.width;
                if end > guide_x {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(
                                Point::new(guide_x, if at_bottom { rect.y + rect.height - 1.0 } else { rect.y }),
                                Size::new(end - guide_x, 1.0),
                            ),
                            ..Default::default()
                        },
                        color,
                    );
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::{Renderer as _, renderer::Headless};
    use iced::widget::text_editor::{self, Action, Cursor, Position};
    use std::future::Future as _;
    use std::sync::atomic::AtomicBool;

    fn renderer() -> iced::Renderer {
        let mut new =
            std::pin::pin!(<iced::Renderer as Headless>::new(theme::mono_font(), Pixels(14.0), Some("tiny-skia")));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(Some(renderer)) = new.as_mut().poll(&mut cx) else { panic!("headless renderer") };
        renderer
    }

    fn paint(renderer: &mut iced::Renderer, content: &Content, index: &code::Index, size: Size<u32>) -> Vec<u8> {
        let bounds = Rectangle::with_size(Size::new(size.width as f32, size.height as f32));
        renderer.reset(bounds);
        let settings = code::Settings {
            token: "rs".into(),
            theme: theme::p().syntax,
            colors: theme::p().brackets(),
            error: theme::p().error_text,
        };
        let inner = iced::widget::text_editor(content)
            .on_action(|a| a)
            .font(theme::mono_font())
            .size(14)
            .height(Length::Fill)
            .wrapping(text::Wrapping::None)
            .padding(iced::Padding { top: 6.0, bottom: 6.0, left: 8.0, right: 12.0 })
            .style(|_, _| text_editor::Style {
                background: theme::p().bg.into(),
                border: Default::default(),
                placeholder: theme::p().muted,
                value: theme::p().fg,
                selection: theme::p().selection,
            })
            .highlight_with::<code::Highlighter>(settings, |f, _| *f);
        let mut element = editor(inner, Some(index), index.indent);
        let mut tree = Tree::new(&element);
        let node = element.as_widget_mut().layout(&mut tree, renderer, &layout::Limits::new(Size::ZERO, bounds.size()));
        let mut messages = Vec::new();
        let layout = Layout::new(&node);
        element.as_widget_mut().update(
            &mut tree,
            &Event::Window(iced::window::Event::RedrawRequested(std::time::Instant::now())),
            layout,
            mouse::Cursor::Unavailable,
            renderer,
            &mut iced::advanced::clipboard::Null,
            &mut Shell::new(&mut messages),
            &bounds,
        );
        element.as_widget().draw(
            &tree,
            renderer,
            &theme::theme(),
            &renderer::Style { text_color: theme::p().fg },
            layout,
            mouse::Cursor::Unavailable,
            &bounds,
        );
        renderer.screenshot(size, 1.0, theme::p().bg)
    }

    fn colored(bytes: &[u8], width: u32, rect: Rectangle, color: Color) -> bool {
        let [r, g, b, _] = color.into_rgba8();
        (rect.y.max(0.0) as u32..(rect.y + rect.height) as u32).any(|y| {
            (rect.x.max(0.0) as u32..(rect.x + rect.width) as u32).any(|x| {
                bytes.get(((y * width + x) * 4) as usize..).is_some_and(|p| {
                    p.len() >= 4 && p[0].abs_diff(r) < 8 && p[1].abs_diff(g) < 8 && p[2].abs_diff(b) < 8
                })
            })
        })
    }

    #[test]
    fn moving_the_cursor_highlights_both_brackets_and_the_connecting_guide_without_selecting() {
        let source = "fn review() {\n    if ready {\n        call([1, 2]);\n    }\n}\n";
        let index = code::Index::parse(source, "rs", &AtomicBool::new(false)).unwrap().unwrap();
        let mut content = Content::with_text(source);
        let mut renderer = renderer();
        let idle = paint(&mut renderer, &content, &index, Size::new(600, 160));
        content.move_to(Cursor { position: Position { line: 2, column: 8 }, selection: None });
        let active = paint(&mut renderer, &content, &index, Size::new(600, 160));
        let row = crate::app::FileTab::row();
        let mut cache = GuideCache::default();
        let x = 8.0 + cache.measure("    ", row).unwrap();
        let guide = Rectangle::new(Point::new(x - 1.0, 6.0 + 2.0 * row + 3.0), Size::new(4.0, row - 6.0));
        let color = theme::p().brackets()[1];
        assert!(!colored(&idle, 600, guide, color), "no active scope before the first bracket");
        assert!(colored(&active, 600, guide, color), "active bracket guide must join both lines");
        assert!(
            colored(&active, 600, Rectangle::new(Point::new(x - 2.0, 6.0 + 3.0 * row), Size::new(12.0, row)), color),
            "closing bracket highlight"
        );
        assert!(
            colored(
                &active,
                600,
                Rectangle::new(
                    Point::new(8.0 + cache.measure(&" ".repeat(13), row).unwrap() - 2.0, 6.0 + row),
                    Size::new(12.0, row)
                ),
                color
            ),
            "opening bracket highlight"
        );
        assert_eq!(content.text(), source);
        assert!(content.selection().is_none(), "pair highlights must not change the user's selection");
        if let Ok(dir) = std::env::var("MIRA_CODE_GUIDES_PREVIEW_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            ::image::save_buffer(
                std::path::Path::new(&dir).join("active-pair.png"),
                &active,
                600,
                160,
                ::image::ColorType::Rgba8,
            )
            .unwrap();
        }
    }

    #[test]
    fn horizontal_scroll_keeps_caret_mouse_hits_and_selections_in_the_same_coordinates() {
        let source = format!("{}{{ value }}", "a".repeat(120));
        let mut editor = ScrolledEditor::with_text(&source);
        let mut highlighter = text::highlighter::PlainText;
        let update = |e: &mut ScrolledEditor, h: &mut text::highlighter::PlainText| {
            e.update(
                Size::new(160.0, 40.0),
                theme::mono_font(),
                Pixels(14.0),
                text::LineHeight::Relative(1.3),
                text::Wrapping::None,
                h,
            )
        };
        update(&mut editor, &mut highlighter);
        editor.move_to(Cursor { position: Position { line: 0, column: 121 }, selection: None });
        update(&mut editor, &mut highlighter);
        assert!(editor.0.buffer().scroll().horizontal > 0.0);
        let text::editor::Selection::Caret(caret) = editor.selection() else { panic!("caret") };
        assert!((0.0..=160.0).contains(&caret.x));
        editor.perform(Action::Click(Point::new(caret.x, 9.0)));
        assert!(
            editor.cursor().position.column.abs_diff(121) <= 1,
            "click must hit the visible brace, not the beginning of the line"
        );
        editor.move_to(Cursor {
            position: Position { line: 0, column: 122 },
            selection: Some(Position { line: 0, column: 120 }),
        });
        update(&mut editor, &mut highlighter);
        let text::editor::Selection::Range(rects) = editor.selection() else { panic!("selection") };
        assert!(rects.iter().all(|r| r.x < 160.0 && r.x + r.width >= 0.0));
        assert_eq!(editor.copy().as_deref(), Some("{ "));
    }

    #[test]
    fn the_active_guide_survives_scrolling_with_both_brackets_offscreen() {
        let source = format!("fn review() {{\n{}}}\n", "    body;\n".repeat(50));
        let index = code::Index::parse(&source, "rs", &AtomicBool::new(false)).unwrap().unwrap();
        let mut content = Content::with_text(&source);
        content.move_to(Cursor { position: Position { line: 25, column: 8 }, selection: None });
        let mut renderer = renderer();
        for scroll in [0, 5, -10] {
            content.perform(Action::Scroll { lines: scroll });
            let pixels = paint(&mut renderer, &content, &index, Size::new(400, 80));
            assert!(colored(
                &pixels,
                400,
                Rectangle::new(Point::new(7.0, 20.0), Size::new(4.0, 30.0)),
                theme::p().brackets()[0]
            ));
        }
        assert_eq!(content.text(), source);
    }

    #[test]
    fn bracket_outlines_use_shaped_glyphs_after_tabs_and_multibyte_text() {
        let source = "\tlet café = { value: 1 };";
        let index = code::Index::parse(source, "rs", &AtomicBool::new(false)).unwrap().unwrap();
        let column = source.find('{').unwrap();
        let mut model = Editor::with_text(source);
        model.update(
            Size::new(600.0, 80.0),
            theme::mono_font(),
            Pixels(14.0),
            text::LineHeight::Relative(1.3),
            text::Wrapping::None,
            &mut text::highlighter::PlainText,
        );
        let run = model.buffer().layout_runs().next().unwrap();
        let glyph = run.glyphs.iter().find(|g| g.start == column).unwrap();
        let border = Rectangle::new(Point::new(8.0 + glyph.x + glyph.w / 3.0, 6.0), Size::new(2.0, 2.0));
        let mut content = Content::with_text(source);
        let mut renderer = renderer();
        let idle = paint(&mut renderer, &content, &index, Size::new(600, 80));
        content.move_to(Cursor { position: Position { line: 0, column: column + 1 }, selection: None });
        let active = paint(&mut renderer, &content, &index, Size::new(600, 80));
        assert!(!colored(&idle, 600, border, theme::p().brackets()[0]));
        assert!(colored(&active, 600, border, theme::p().brackets()[0]));
    }
}
