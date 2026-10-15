//! Search & replace in the opened folder.

use super::{App, Msg, blocking};
use crate::search::{self, FileMatches, Options};

use iced::Task;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Typing waits this long before searching, so a word is one search, not one per key.
const DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub replace: String,
    pub opts: Options,
    /// Bumped per search and shared with the worker, which stops once it moves on.
    pub generation: Arc<AtomicU64>,
    pub running: bool,
    pub results: Vec<FileMatches>,
    pub total: usize,
    pub error: Option<String>,
    /// Generation of `results`; clicks on a result carry it.
    pub shown: u64,
}

impl SearchState {
    fn current(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }
}

impl App {
    pub(super) fn update_find(&mut self, msg: Msg) -> Task<Msg> {
        // Any change to what would be replaced asks for the confirmation again.
        if matches!(
            msg,
            Msg::SearchQuery(_) | Msg::SearchReplace(_) | Msg::SearchToggle(_) | Msg::SearchRun | Msg::SearchDone(..)
        ) {
            self.confirm_replace = false;
        }
        match msg {
            Msg::SearchQuery(q) => {
                self.search.query = q;
                let Some(generation) = self.new_search() else { return Task::none() };
                return Task::perform(tokio::time::sleep(DEBOUNCE), move |()| Msg::SearchDebounced(generation));
            }
            Msg::SearchDebounced(generation) => {
                if generation == self.search.current() {
                    return self.start_search(generation);
                }
            }
            Msg::SearchReplace(r) => self.search.replace = r,
            Msg::SearchToggle(which) => {
                let o = &mut self.search.opts;
                match which {
                    0 => o.case_sensitive = !o.case_sensitive,
                    1 => o.whole_word = !o.whole_word,
                    _ => o.regex = !o.regex,
                }
                return self.run_search();
            }
            Msg::SearchRun => return self.run_search(),
            Msg::SearchDone(generation_id, result) => {
                if generation_id == self.search.current() {
                    self.search.running = false;
                    match result {
                        Ok((results, total)) => {
                            self.search.results = results;
                            self.search.total = total;
                            self.search.shown = generation_id;
                        }
                        Err(e) => {
                            self.search.results.clear();
                            self.search.total = 0;
                            self.search.error = Some(e);
                        }
                    }
                }
            }
            Msg::SearchOpen(shown, file, line) => {
                if let Some(f) = self.search.results.get(file).filter(|_| shown == self.search.shown) {
                    let line = line.and_then(|i| f.lines.get(i)).map(|m| m.line);
                    return self.update(Msg::OpenFile(f.path.clone(), line));
                }
            }
            Msg::SearchSend(shown, file, line) => {
                if let Some(f) = self.search.results.get(file).filter(|_| shown == self.search.shown)
                    && let Some(m) = f.lines.get(line)
                {
                    return self.update(Msg::SendToAgent(format!("{}:{} ", f.rel, m.line + 1)));
                }
            }
            Msg::ReplaceAll => {
                // Results of an older query would get the new pattern.
                if self.search.running || self.search.results.is_empty() {
                    return Task::none();
                }
                // Files not open in a tab can't be undone: the first press asks.
                if !self.confirm_replace {
                    self.confirm_replace = true;
                    self.set_confirm("Click Replace again to rewrite these files on disk · Esc cancels");
                    return Task::none();
                }
                self.confirm_replace = false;
                let capped = self.search.total >= search::MAX_MATCHES;
                let files: Vec<PathBuf> = self.search.results.iter().map(|f| f.path.clone()).collect();
                let (query, replacement, opts) =
                    (self.search.query.clone(), self.search.replace.clone(), self.search.opts.clone());
                return Task::perform(
                    blocking(move || search::replace_all(&files, &query, &replacement, &opts)),
                    move |r| Msg::ReplaceDone(capped, r),
                );
            }
            Msg::ReplaceDone(capped, result) => {
                match result {
                    Ok(n) if capped => self.set_status(
                        format!(
                            "Replaced in {n} file(s); results stop at {} matches, so run Replace all again for the rest",
                            search::MAX_MATCHES
                        ),
                        true,
                    ),
                    Ok(n) => self.set_status(format!("Replaced in {n} file(s)"), false),
                    Err(e) => self.set_status(e, true),
                }
                self.reload_changed_files();
                return self.run_search();
            }
            _ => {}
        }
        Task::none()
    }

    /// Search now (Enter, an option toggled, after a replace).
    fn run_search(&mut self) -> Task<Msg> {
        match self.new_search() {
            Some(generation) => self.start_search(generation),
            None => Task::none(),
        }
    }

    /// Stop the running search; `None` when the query is empty (results cleared).
    fn new_search(&mut self) -> Option<u64> {
        let generation = self.search.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.search.error = None;
        if self.search.query.is_empty() {
            self.search.results.clear();
            self.search.total = 0;
            self.search.running = false;
            return None;
        }
        self.search.running = true;
        Some(generation)
    }

    fn start_search(&self, generation: u64) -> Task<Msg> {
        let (current, root, query, opts) = (
            Arc::clone(&self.search.generation),
            self.root.clone(),
            self.search.query.clone(),
            self.search.opts.clone(),
        );
        Task::perform(
            blocking(move || search::search(&root, &query, &opts, || current.load(Ordering::Relaxed) != generation)),
            move |r| Msg::SearchDone(generation, r),
        )
    }
}
