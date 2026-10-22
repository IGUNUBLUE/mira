//! Selection and asynchronous loading of a commit's affected files.

use super::{App, DiffSource, Msg, blocking};
use crate::git::commits::{self, Details};
use iced::Task;
use std::sync::Arc;

#[derive(Default)]
pub struct CommitView {
    pub selected: Option<SelectedCommit>,
    sequence: u64,
}

pub struct SelectedCommit {
    pub id: u64,
    pub hash: Arc<str>,
    pub result: Option<Result<Arc<Details>, String>>,
}

impl CommitView {
    fn toggle(&mut self, hash: Arc<str>) -> Option<u64> {
        if self.selected.as_ref().is_some_and(|s| s.hash == hash) {
            self.selected = None;
            return None;
        }
        self.sequence = self.sequence.wrapping_add(1);
        let id = self.sequence;
        self.selected = Some(SelectedCommit { id, hash, result: None });
        Some(id)
    }

    fn loaded(&mut self, id: u64, result: Result<Arc<Details>, String>) -> bool {
        let Some(selected) = &mut self.selected else { return false };
        if selected.id != id {
            return false;
        }
        selected.result = Some(result);
        true
    }
}

impl App {
    pub(super) fn update_commits(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::HistorySelect(hash) => {
                if !self.git.log.iter().any(|c| c.hash == hash) {
                    return Task::none();
                }
                let Some(id) = self.commit_view.toggle(Arc::clone(&hash)) else { return Task::none() };
                let (root, prefix) = (self.repo_root(), self.repo_prefix.clone());
                return self.for_root(Task::perform(
                    blocking(move || commits::inspect(&root, &hash, &prefix).map(Arc::new)),
                    move |result| Msg::HistoryLoaded(id, result),
                ));
            }
            Msg::HistoryLoaded(id, result) => {
                let error = result.as_ref().err().cloned();
                if self.commit_view.loaded(id, result)
                    && let Some(error) = error
                {
                    self.set_status(format!("Inspect commit: {error}"), true);
                }
            }
            Msg::HistoryOpen(id, index) => {
                let Some(selected) = self.commit_view.selected.as_ref().filter(|s| s.id == id) else {
                    return Task::none();
                };
                let Some(Ok(details)) = &selected.result else { return Task::none() };
                let Some(file) = details.files.get(index) else { return Task::none() };
                return self.open_diff(file.path.clone(), DiffSource::Commit { details: Arc::clone(details) });
            }
            _ => {}
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selecting_another_commit_ignores_the_previous_reply() {
        let mut view = CommitView::default();
        let first = view.toggle("abcd1234".into()).unwrap();
        let second = view.toggle("abcd5678".into()).unwrap();
        assert!(!view.loaded(first, Err("old failure".into())));
        assert!(view.selected.as_ref().unwrap().result.is_none());
        assert!(view.loaded(second, Err("current failure".into())));
        assert_eq!(view.selected.unwrap().result.unwrap().unwrap_err(), "current failure");
    }

    #[test]
    fn collapsing_and_reopening_the_same_commit_does_not_accept_old_replies() {
        let mut view = CommitView::default();
        let old = view.toggle("abcd1234".into()).unwrap();
        assert!(view.toggle("abcd1234".into()).is_none());
        assert!(view.selected.is_none());
        let new = view.toggle("abcd1234".into()).unwrap();
        assert_ne!(new, old);
        assert!(!view.loaded(old, Err("stale".into())));
        assert!(view.loaded(new, Err("fresh".into())));
    }

    #[test]
    fn resetting_the_folder_selection_drops_inflight_replies() {
        let mut view = CommitView::default();
        let old = view.toggle("abcd1234".into()).unwrap();
        view.selected = None;
        assert!(!view.loaded(old, Err("previous folder".into())));
        let new = view.toggle("abcd1234".into()).unwrap();
        assert_ne!(new, old);
    }
}
