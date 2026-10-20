//! A small spinner that can animate inside cached diff rows without rebuilding them.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::text::Renderer as _;
use iced::advanced::widget::{Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget};
use iced::{Element, Event, Length, Rectangle, Size, mouse};
use std::time::{Duration, Instant};

use crate::{app::SPINNER, icons, theme};

const STEP: Duration = Duration::from_millis(80);

#[derive(Default)]
struct State {
    frame: usize,
    last: Option<Instant>,
}

impl State {
    fn advance(&mut self, now: Instant) -> Instant {
        if let Some(last) = self.last {
            let steps = now.saturating_duration_since(last).as_millis() / STEP.as_millis();
            self.frame = (self.frame + (steps % SPINNER.len() as u128) as usize) % SPINNER.len();
            self.last = Some(last + STEP * steps.min(u128::from(u32::MAX)) as u32);
        } else {
            self.last = Some(now);
        }
        self.last.unwrap_or(now) + STEP
    }
}

struct Spinner(u32);

pub fn view<'a, M: 'a>(size: u32) -> Element<'a, M> {
    Element::new(Spinner(size))
}

impl<M> Widget<M, iced::Theme, iced::Renderer> for Spinner {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.0 as f32), Length::Fixed(self.0 as f32))
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.resolve(Length::Shrink, Length::Shrink, Size::new(self.0 as f32, self.0 as f32)))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        if let Event::Window(iced::window::Event::RedrawRequested(now)) = event
            && layout.bounds().intersects(viewport)
        {
            shell.request_redraw_at(tree.state.downcast_mut::<State>().advance(*now));
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        renderer.fill_text(
            iced::advanced::Text {
                content: SPINNER[tree.state.downcast_ref::<State>().frame].to_string(),
                bounds: bounds.size(),
                size: (self.0 as f32).into(),
                line_height: iced::advanced::text::LineHeight::Relative(1.0),
                font: icons::font().unwrap_or_else(theme::mono_font),
                align_x: iced::advanced::text::Alignment::Center,
                align_y: iced::alignment::Vertical::Center,
                shaping: iced::advanced::text::Shaping::Basic,
                wrapping: iced::advanced::text::Wrapping::None,
            },
            bounds.center(),
            theme::p().accent,
            *viewport,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_widget_repaints_without_publishing_messages_and_sleeps_offscreen() {
        use iced::advanced::renderer::Headless;
        use std::future::Future as _;
        let mut new = std::pin::pin!(<iced::Renderer as Headless>::new(
            iced::Font::MONOSPACE,
            iced::Pixels(14.0),
            Some("tiny-skia")
        ));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(Some(mut renderer)) = new.as_mut().poll(&mut cx) else {
            panic!("headless renderer")
        };
        let mut el: Element<'_, ()> = view(24);
        let mut tree = Tree::new(&el);
        let node =
            el.as_widget_mut().layout(&mut tree, &renderer, &layout::Limits::new(Size::ZERO, Size::new(24.0, 24.0)));
        let layout = Layout::new(&node);
        let start = Instant::now();
        let mut messages = Vec::new();
        let mut clip = iced::advanced::clipboard::Null;
        let mut screenshots = Vec::new();
        for now in [start, start + STEP] {
            let mut shell = Shell::new(&mut messages);
            el.as_widget_mut().update(
                &mut tree,
                &Event::Window(iced::window::Event::RedrawRequested(now)),
                layout,
                mouse::Cursor::Unavailable,
                &renderer,
                &mut clip,
                &mut shell,
                &layout.bounds(),
            );
            assert_eq!(shell.redraw_request(), iced::window::RedrawRequest::At(now + STEP));
            el.as_widget().draw(
                &tree,
                &mut renderer,
                &iced::Theme::Dark,
                &renderer::Style::default(),
                layout,
                mouse::Cursor::Unavailable,
                &layout.bounds(),
            );
            screenshots.push(renderer.screenshot(Size::new(24, 24), 1.0, theme::p().bg));
        }
        assert_ne!(screenshots[0], screenshots[1], "frames paint different pixels without rebuilding the widget");
        assert!(messages.is_empty());
        let mut shell = Shell::new(&mut messages);
        el.as_widget_mut().update(
            &mut tree,
            &Event::Window(iced::window::Event::RedrawRequested(start + STEP * 2)),
            layout,
            mouse::Cursor::Unavailable,
            &renderer,
            &mut clip,
            &mut shell,
            &Rectangle::new(iced::Point::new(100.0, 100.0), Size::new(24.0, 24.0)),
        );
        assert_eq!(shell.redraw_request(), iced::window::RedrawRequest::Wait);
    }

    #[test]
    fn cached_spinner_advances_on_redraws_and_wraps_without_app_messages() {
        let start = Instant::now();
        let mut state = State::default();
        assert_eq!(state.advance(start), start + STEP);
        state.advance(start + STEP / 2);
        assert_eq!(state.frame, 0);
        assert_eq!(state.advance(start + STEP), start + STEP * 2);
        assert_eq!(state.frame, 1);
        state.advance(start + STEP * 12);
        assert_eq!(state.frame, 2);
    }
}
