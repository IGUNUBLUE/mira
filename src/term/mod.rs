//! mira's terminal: alacritty_terminal for the emulation and the PTY, and
//! mira's own widget to draw it and take input (`widget.rs`).
//!
//! Threads: alacritty's event loop reads the PTY and parses on its own
//! thread, holding the terminal lock while it does. Its [`Listener`] never
//! blocks: what the program asks of the terminal (color queries, size,
//! replies like device attributes) is answered right there, and what the UI
//! must know (new content, clipboard, exit) goes through an unbounded channel
//! that a subscription drains in batches. The UI only takes the lock to
//! snapshot the visible rows ([`frame`]) once per frame, and to apply input.

pub mod boxdraw;
pub mod frame;
pub mod input;
mod widget;

pub use widget::view;

use crate::theme;

use alacritty_terminal::event::{Event as AlacrittyEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg as LoopMsg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{self, ClipboardType, Term, TermMode, viewport_to_point};
use alacritty_terminal::tty;
use iced::futures::channel::mpsc;
use std::cell::{Cell, Ref, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// Lines of scrollback kept (herdr keeps its panes' own history).
const SCROLLBACK: usize = 10_000;

/// What to run in the terminal.
pub struct Options {
    pub program: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: PathBuf,
}

/// What the terminal tells the app.
#[derive(Debug, Clone)]
pub enum Event {
    /// New content to draw.
    Wakeup,
    /// The program asked to copy this (OSC 52); `primary`: the X selection.
    Clipboard { primary: bool, text: String },
    /// The program exited.
    Exited,
}

/// A cell of the visible grid (0-based, top-left).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

/// What the widget asks of the terminal.
#[derive(Debug, Clone)]
pub enum Action {
    /// Typed input: jumps back to the bottom of the history.
    Input(Vec<u8>),
    /// Mouse and focus reports to the program (the view stays where it is).
    Report(Vec<u8>),
    Paste(String),
    Resize {
        cols: usize,
        lines: usize,
        cell_width: f32,
        cell_height: f32,
    },
    /// Scroll the history by lines (positive: back).
    Scroll(i32),
    ScrollToBottom,
    ScrollToTop,
    Select(SelectionType, Pos, Side),
    SelectTo(Pos, Side),
    /// The drag ended: the selection goes to the primary selection.
    SelectDone,
    Copy,
    /// Ctrl over a cell: underline the link there, if any (`None`: clear).
    Hover(Option<Pos>),
    /// Ctrl+click: open the link there.
    Open(Pos),
}

/// What the app must do after an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    None,
    Copy { primary: bool, text: String },
    Open(String),
}

#[derive(Clone)]
struct Listener {
    events: mpsc::UnboundedSender<Event>,
    /// Set right after the event loop is created (it needs the listener first).
    pty: Arc<OnceLock<EventLoopSender>>,
    size: Arc<Mutex<WindowSize>>,
}

impl Listener {
    fn reply(&self, text: String) {
        if let Some(pty) = self.pty.get() {
            // Fails only once the loop has shut down: nobody to answer then.
            let _ = pty.send(LoopMsg::Input(text.into_bytes().into()));
        }
    }

    fn tell(&self, event: Event) {
        // Fails only once the app dropped the terminal.
        let _ = self.events.unbounded_send(event);
    }
}

impl EventListener for Listener {
    /// Runs on the PTY thread with the terminal locked: never block here.
    fn send_event(&self, event: AlacrittyEvent) {
        match event {
            AlacrittyEvent::PtyWrite(text) => self.reply(text),
            AlacrittyEvent::ColorRequest(index, format) => {
                self.reply(format(frame::palette_rgb(index, theme::p())));
            }
            AlacrittyEvent::TextAreaSizeRequest(format) => {
                let size = *self.size.lock().unwrap_or_else(PoisonError::into_inner);
                self.reply(format(size));
            }
            AlacrittyEvent::Wakeup => self.tell(Event::Wakeup),
            AlacrittyEvent::ClipboardStore(kind, text) => {
                self.tell(Event::Clipboard { primary: kind == ClipboardType::Selection, text });
            }
            AlacrittyEvent::Exit | AlacrittyEvent::ChildExit(_) => self.tell(Event::Exited),
            // Reading the clipboard is refused (a data leak); OSC 52 is copy-only anyway.
            // Title, bell, blinking and mouse-cursor hints are not used.
            _ => {}
        }
    }
}

pub struct Terminal {
    id: u64,
    widget_id: iced::widget::Id,
    term: Arc<FairMutex<Term<Listener>>>,
    pty: EventLoopSender,
    size: Arc<Mutex<WindowSize>>,
    events: Arc<Mutex<Option<mpsc::UnboundedReceiver<Event>>>>,
    /// The last snapshot, rebuilt on demand once something changed.
    frame: RefCell<frame::Frame>,
    dirty: Cell<bool>,
    /// The hovered link (Ctrl held), underlined.
    link: Option<Link>,
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // The loop then ends and drops the PTY, which hangs up on the child.
        let _ = self.pty.send(LoopMsg::Shutdown);
    }
}

impl Terminal {
    /// Start `options.program` in a new PTY.
    pub fn spawn(id: u64, options: Options) -> std::io::Result<Self> {
        let size = WindowSize { num_lines: 24, num_cols: 80, cell_width: 8, cell_height: 16 };
        let pty = tty::new(
            &tty::Options {
                shell: Some(tty::Shell::new(options.program, options.args)),
                working_directory: Some(options.cwd),
                drain_on_exit: false,
                env: options.env,
            },
            size,
            id,
        )?;
        let (tx, rx) = mpsc::unbounded();
        let shared_size = Arc::new(Mutex::new(size));
        let listener = Listener { events: tx, pty: Arc::new(OnceLock::new()), size: Arc::clone(&shared_size) };
        let config = term::Config { scrolling_history: SCROLLBACK, ..term::Config::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &TermSize::new(80, 24), listener.clone())));
        let event_loop = EventLoop::new(Arc::clone(&term), listener.clone(), pty, false, false)?;
        let sender = event_loop.channel();
        // Just created: nothing else can have set it.
        let _ = listener.pty.set(event_loop.channel());
        // Detached: it ends on `Shutdown` or when the program exits.
        drop(event_loop.spawn());
        Ok(Self {
            id,
            widget_id: iced::widget::Id::unique(),
            term,
            pty: sender,
            size: shared_size,
            events: Arc::new(Mutex::new(Some(rx))),
            frame: RefCell::new(frame::Frame::default()),
            dirty: Cell::new(true),
            link: None,
        })
    }

    pub fn widget_id(&self) -> &iced::widget::Id {
        &self.widget_id
    }

    /// The terminal's events, in batches (a burst of output is one message).
    pub fn subscription(&self) -> iced::Subscription<Vec<Event>> {
        iced::Subscription::run_with(EventSource { id: self.id, rx: Arc::clone(&self.events) }, EventSource::stream)
    }

    /// The visible screen, snapshotted again if it changed since last asked.
    pub fn frame(&self) -> Ref<'_, frame::Frame> {
        if self.dirty.replace(false) {
            let links: Vec<_> = self.link.iter().map(|l| (l.line, l.cols.clone())).collect();
            let snapshot = frame::build(&self.term.lock(), theme::p(), &links);
            *self.frame.borrow_mut() = snapshot;
        }
        self.frame.borrow()
    }

    /// Draw again (new content, or mira's palette changed).
    pub fn redraw(&self) {
        self.dirty.set(true);
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    /// Bytes for the program, as if typed.
    pub fn write(&self, bytes: Vec<u8>) {
        // Fails only once the loop has shut down (the program exited).
        let _ = self.pty.send(LoopMsg::Input(bytes.into()));
    }

    /// Tell the program the window gained or lost focus, if it asked (DECSET 1004).
    pub fn focus_changed(&self, focused: bool) {
        if self.mode().contains(TermMode::FOCUS_IN_OUT) {
            self.write(if focused { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
        }
    }

    pub fn on_event(&mut self, event: &Event) {
        if matches!(event, Event::Wakeup) {
            self.redraw();
        }
    }

    pub fn apply(&mut self, action: Action) -> Effect {
        let mut effect = Effect::None;
        {
            let mut term = self.term.lock();
            match action {
                Action::Input(bytes) => {
                    term.scroll_display(Scroll::Bottom);
                    self.write(bytes);
                }
                Action::Report(bytes) => self.write(bytes),
                Action::Paste(text) => {
                    term.scroll_display(Scroll::Bottom);
                    self.write(input::paste(&text, *term.mode()));
                }
                Action::Resize { cols, lines, cell_width, cell_height } => {
                    let size = WindowSize {
                        num_lines: u16::try_from(lines).unwrap_or(u16::MAX).max(1),
                        num_cols: u16::try_from(cols).unwrap_or(u16::MAX).max(1),
                        cell_width: cell_width.round().clamp(1.0, f32::from(u16::MAX)) as u16,
                        cell_height: cell_height.round().clamp(1.0, f32::from(u16::MAX)) as u16,
                    };
                    *self.size.lock().unwrap_or_else(PoisonError::into_inner) = size;
                    term.resize(TermSize::new(usize::from(size.num_cols), usize::from(size.num_lines)));
                    let _ = self.pty.send(LoopMsg::Resize(size));
                }
                Action::Scroll(lines) => term.scroll_display(Scroll::Delta(lines)),
                Action::ScrollToBottom => term.scroll_display(Scroll::Bottom),
                Action::ScrollToTop => term.scroll_display(Scroll::Top),
                Action::Select(kind, pos, side) => {
                    let point = grid_point(&term, pos);
                    term.selection = Some(Selection::new(kind, point, side));
                }
                Action::SelectTo(pos, side) => {
                    let point = grid_point(&term, pos);
                    if let Some(selection) = &mut term.selection {
                        selection.update(point, side);
                    }
                }
                Action::SelectDone => {
                    if let Some(text) = term.selection_to_string().filter(|t| !t.is_empty()) {
                        effect = Effect::Copy { primary: true, text };
                    }
                }
                Action::Copy => {
                    if let Some(text) = term.selection_to_string().filter(|t| !t.is_empty()) {
                        effect = Effect::Copy { primary: false, text };
                    }
                }
                Action::Hover(pos) => {
                    self.link = pos.and_then(|p| link_at(&term, p));
                }
                Action::Open(pos) => {
                    if let Some(link) = link_at(&term, pos) {
                        effect = Effect::Open(link.uri);
                    }
                }
            }
        }
        self.redraw();
        effect
    }

    /// The link under the pointer (Ctrl held), for the pointer shape.
    pub fn hovered_link(&self) -> bool {
        self.link.is_some()
    }
}

fn grid_point<T>(term: &Term<T>, pos: Pos) -> Point {
    let line = pos.line.min(term.screen_lines().saturating_sub(1));
    let col = pos.col.min(term.columns().saturating_sub(1));
    viewport_to_point(term.grid().display_offset(), Point::new(line, Column(col)))
}

/// A link on screen: its columns on one line, and its target.
struct Link {
    line: usize,
    cols: Range<usize>,
    uri: String,
}

/// The link at `pos`: an OSC 8 hyperlink, else a URL in the text of that row.
fn link_at<T>(term: &Term<T>, pos: Pos) -> Option<Link> {
    let point = grid_point(term, pos);
    let row = &term.grid()[point.line];
    let cols = term.columns();
    if let Some(link) = row[point.column].hyperlink() {
        let same = |c: usize| row[Column(c)].hyperlink().is_some_and(|h| h.id() == link.id());
        let start = (0..=point.column.0).rev().take_while(|&c| same(c)).last().unwrap_or(point.column.0);
        let end = (point.column.0..cols).take_while(|&c| same(c)).last().unwrap_or(point.column.0) + 1;
        return Some(Link { line: pos.line, cols: start..end, uri: link.uri().to_string() });
    }
    let text: Vec<char> = (0..cols).map(|c| row[Column(c)].c).collect();
    let (cols, uri) = find_url(&text, pos.col)?;
    Some(Link { line: pos.line, cols, uri })
}

/// A URL in `row` (one char per column) that covers column `col`.
fn find_url(row: &[char], col: usize) -> Option<(Range<usize>, String)> {
    const SCHEMES: [&str; 5] = ["https://", "http://", "file://", "ssh://", "ftp://"];
    let line: String = row.iter().collect();
    // Byte offset → column, since each char is one column here.
    let col_of = |byte: usize| line[..byte].chars().count();
    for scheme in SCHEMES {
        for (start, _) in line.match_indices(scheme) {
            let rest = &line[start..];
            let len = rest
                .find(|c: char| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"' | '\'' | '`'))
                .unwrap_or(rest.len());
            // Trailing punctuation closes a sentence, not the URL.
            let url = rest[..len].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}']);
            if url.len() <= scheme.len() {
                continue;
            }
            let range = col_of(start)..col_of(start + url.len());
            if range.contains(&col) {
                return Some((range, url.to_string()));
            }
        }
    }
    None
}

/// The subscription's source: one per terminal, identified by its id.
struct EventSource {
    id: u64,
    rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<Event>>>>,
}

impl std::hash::Hash for EventSource {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl EventSource {
    fn stream(&self) -> impl iced::futures::Stream<Item = Vec<Event>> + use<> {
        use iced::futures::StreamExt;
        let rx = self.rx.lock().unwrap_or_else(PoisonError::into_inner).take();
        // Taken once: iced keeps one stream per id alive for the terminal's life.
        iced::futures::stream::iter(rx).flatten().ready_chunks(256).map(|mut batch| {
            // A burst of output is one redraw.
            let mut seen = false;
            batch.retain(|e| !matches!(e, Event::Wakeup) || !std::mem::replace(&mut seen, true));
            batch
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn finds_the_url_under_the_pointer() {
        let r = row("see https://example.org/a?b=1. ok http://x.y");
        assert_eq!(find_url(&r, 10), Some((4..29, "https://example.org/a?b=1".into())));
        assert_eq!(find_url(&r, 2), None);
        assert_eq!(find_url(&r, 29), None, "the period ends the sentence");
        assert_eq!(find_url(&r, 40).map(|(_, u)| u), Some("http://x.y".into()));
        assert_eq!(find_url(&row("(https://a.b/c)"), 3), Some((1..14, "https://a.b/c".into())));
        assert_eq!(find_url(&row("https:// nothing"), 2), None);
    }

    #[test]
    fn runs_a_program_and_reads_its_output() {
        let mut t = Terminal::spawn(
            1,
            Options {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), "printf 'hello\\033[31m red'; sleep 5".into()],
                env: HashMap::new(),
                cwd: std::env::temp_dir(),
            },
        )
        .expect("spawn");
        t.apply(Action::Resize { cols: 20, lines: 4, cell_width: 8.0, cell_height: 16.0 });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            t.redraw();
            let text: String = t.frame().row(0).iter().map(|c| c.c).collect();
            if text.starts_with("hello red") {
                assert_eq!(t.frame().row(0)[6].fg, frame::palette_rgb(1, theme::p()));
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no output: {text:?}");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!((t.frame().cols, t.frame().lines), (20, 4));
        // Typed input reaches the program; selection copies the text.
        t.apply(Action::Select(SelectionType::Simple, Pos { line: 0, col: 0 }, Side::Left));
        t.apply(Action::SelectTo(Pos { line: 0, col: 4 }, Side::Right));
        assert_eq!(t.apply(Action::Copy), Effect::Copy { primary: false, text: "hello".into() });
    }

    /// Run `script` in a 40×6 terminal and wait until `ready` holds.
    fn run(script: &str, ready: impl Fn(&Terminal) -> bool) -> Terminal {
        let mut t = Terminal::spawn(
            3,
            Options {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                env: HashMap::new(),
                cwd: std::env::temp_dir(),
            },
        )
        .expect("spawn");
        t.apply(Action::Resize { cols: 40, lines: 6, cell_width: 8.0, cell_height: 16.0 });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !ready(&t) {
            assert!(std::time::Instant::now() < deadline, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(20));
            t.redraw();
        }
        t
    }

    fn line(t: &Terminal, n: usize) -> String {
        t.frame().row(n).iter().map(|c| c.c).collect::<String>().trim_end().to_string()
    }

    #[test]
    fn osc8_links_hover_and_open_and_modes_are_seen() {
        let script = "printf 'see \\033]8;;https://a.b/c\\033\\\\the docs\\033]8;;\\033\\\\ ok\\033[?2004h\\033[?1004h'; sleep 5";
        let mut t = run(script, |t| line(t, 0) == "see the docs ok" && t.mode().contains(TermMode::BRACKETED_PASTE));
        assert!(t.mode().contains(TermMode::FOCUS_IN_OUT));
        assert!(!t.hovered_link());
        t.apply(Action::Hover(Some(Pos { line: 0, col: 6 })));
        assert!(t.hovered_link());
        // The whole link is underlined, nothing else.
        let underlined: Vec<usize> = t
            .frame()
            .row(0)
            .iter()
            .enumerate()
            .filter(|(_, c)| c.underline != frame::Underline::None)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(underlined, (4..12).collect::<Vec<_>>());
        assert_eq!(t.apply(Action::Open(Pos { line: 0, col: 4 })), Effect::Open("https://a.b/c".into()));
        assert_eq!(t.apply(Action::Open(Pos { line: 0, col: 1 })), Effect::None);
        t.apply(Action::Hover(None));
        assert!(!t.hovered_link());
        // The modes the program set decide what is written for focus and paste.
        t.focus_changed(true);
        t.focus_changed(false);
        t.on_event(&Event::Exited);
        assert_eq!(t.apply(Action::Paste("x".into())), Effect::None);
        assert_eq!(t.apply(Action::Report(b"\x1b[<0;1;1M".to_vec())), Effect::None);
        t.on_event(&Event::Wakeup);
        assert!(t.dirty.get());
        let _ = t.subscription();
        assert_eq!(t.widget_id(), t.widget_id());
    }

    #[test]
    fn a_program_that_cannot_start_is_an_error() {
        let options = Options {
            program: "/nonexistent/mira-test".into(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: std::env::temp_dir(),
        };
        assert!(Terminal::spawn(11, options).is_err());
    }

    #[test]
    fn plain_urls_are_links_too() {
        let mut t = run("printf 'go to https://example.org/x now'; sleep 5", |t| line(t, 0).ends_with("now"));
        t.apply(Action::Hover(Some(Pos { line: 0, col: 10 })));
        assert!(t.hovered_link());
        assert_eq!(t.apply(Action::Open(Pos { line: 0, col: 10 })), Effect::Open("https://example.org/x".into()));
        // A terminal that is told nothing doesn't report focus.
        t.focus_changed(false);
    }

    #[test]
    fn the_listener_answers_and_forwards_without_blocking() {
        use alacritty_terminal::event::EventListener as _;
        let (tx, mut rx) = mpsc::unbounded();
        let size = Arc::new(Mutex::new(WindowSize { num_lines: 1, num_cols: 1, cell_width: 1, cell_height: 1 }));
        // No event loop yet: replies are dropped instead of blocking.
        let l = Listener { events: tx, pty: Arc::new(OnceLock::new()), size };
        l.send_event(AlacrittyEvent::PtyWrite("x".into()));
        l.send_event(AlacrittyEvent::TextAreaSizeRequest(Arc::new(|s| format!("{}x{}", s.num_cols, s.num_lines))));
        l.send_event(AlacrittyEvent::ColorRequest(1, Arc::new(|c| format!("{c:?}"))));
        l.send_event(AlacrittyEvent::ClipboardStore(ClipboardType::Selection, "sel".into()));
        l.send_event(AlacrittyEvent::ClipboardStore(ClipboardType::Clipboard, "clip".into()));
        l.send_event(AlacrittyEvent::Bell);
        l.send_event(AlacrittyEvent::Exit);
        let got: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok()).map(|e| format!("{e:?}")).collect();
        assert_eq!(
            got,
            ["Clipboard { primary: true, text: \"sel\" }", "Clipboard { primary: false, text: \"clip\" }", "Exited"]
        );
    }

    #[test]
    fn events_arrive_in_batches_with_one_wakeup() {
        use iced::futures::StreamExt;
        use std::hash::{Hash, Hasher};
        let (tx, rx) = mpsc::unbounded();
        for e in [Event::Wakeup, Event::Wakeup, Event::Clipboard { primary: false, text: "a".into() }, Event::Wakeup] {
            tx.unbounded_send(e).unwrap();
        }
        let source = EventSource { id: 7, rx: Arc::new(Mutex::new(Some(rx))) };
        let mut stream = std::pin::pin!(source.stream());
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(Some(batch)) = stream.poll_next_unpin(&mut cx) else { panic!("a batch") };
        assert_eq!(batch.iter().filter(|e| matches!(e, Event::Wakeup)).count(), 1);
        assert_eq!(batch.len(), 2);
        // Same id, same subscription.
        let hash = |s: &EventSource| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash(&source), hash(&EventSource { id: 7, rx: Arc::new(Mutex::new(None)) }));
    }

    #[test]
    fn the_history_scrolls_and_input_returns_to_the_bottom() {
        let mut t = run("seq 1 30; sleep 5", |t| line(t, 4) == "30");
        t.apply(Action::Scroll(3));
        assert_eq!((t.frame().display_offset, line(&t, 0)), (3, "23".to_string()));
        t.apply(Action::ScrollToTop);
        assert_eq!(line(&t, 0), "1");
        // A selection in the history copies what is shown there.
        t.apply(Action::Select(SelectionType::Lines, Pos { line: 1, col: 0 }, Side::Left));
        t.apply(Action::SelectDone);
        assert_eq!(t.apply(Action::Copy), Effect::Copy { primary: false, text: "2\n".into() });
        t.apply(Action::Input(Vec::new()));
        assert_eq!((t.frame().display_offset, line(&t, 4)), (0, "30".to_string()));
        t.apply(Action::Scroll(2));
        t.apply(Action::ScrollToBottom);
        assert_eq!(t.frame().display_offset, 0);
    }

    #[test]
    fn answers_color_queries_on_the_pty_thread() {
        // The program asks for color 1 and the background, then prints the replies' length.
        let script = "stty -echo raw; printf '\\033]4;1;?\\007\\033]11;?\\007'; \
                      dd bs=1 count=40 2>/dev/null | od -c | head -3; sleep 5";
        let mut t = Terminal::spawn(
            2,
            Options {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                env: HashMap::new(),
                cwd: std::env::temp_dir(),
            },
        )
        .expect("spawn");
        t.apply(Action::Resize { cols: 80, lines: 6, cell_width: 8.0, cell_height: 16.0 });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            t.redraw();
            let f = t.frame();
            let screen: String = (0..f.lines).flat_map(|l| f.row(l).iter().map(|c| c.c)).collect();
            // `od` shows the replies the terminal typed: OSC 4 and OSC 11 with rgb values.
            if screen.contains("r   g   b") || screen.contains("]   4   ;") {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no reply: {screen:?}");
            drop(f);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
