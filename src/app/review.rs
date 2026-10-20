//! Agent runs: before/after snapshots, their files and prompts, and the review actions.

use super::{App, DiffSource, Msg, blocking};
use crate::git;
use crate::persist;
use crate::runs::{Effect, RunId};

use iced::Task;
use std::path::PathBuf;

/// Snapshot error for a folder that turned out not to be in a repository.
const NOT_A_REPO: &str = "not a git repository";

impl App {
    pub(super) fn update_review(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::RunSnapshot(root, id, after, result) => {
                self.snapshot_running = false;
                // Taken for a folder shown before: its saved state gets it, so the
                // run stays reviewable there (its files load when it's shown again).
                if root != self.root {
                    match result {
                        Ok(tree) => {
                            if let Err(e) = persist::edit_runs(&root, |runs| {
                                runs.iter_mut().find(|r| r.id == id).is_some_and(|r| r.store_tree(after, tree))
                            }) {
                                self.set_status(format!("Could not save review state: {e}"), true);
                            }
                        }
                        // Too large to snapshot: its other queued ones would time out too,
                        // each holding up the shown folder's.
                        Err(e) if e.starts_with(git::SNAPSHOT_TIMED_OUT) || e == NOT_A_REPO => {
                            self.snapshot_queue.retain(|(r, ..)| *r != root);
                        }
                        Err(_) => {}
                    }
                    return self.next_snapshot();
                }
                let tree = match result {
                    Ok(t) => t,
                    Err(e) if e.starts_with(git::SNAPSHOT_TIMED_OUT) => {
                        // Too big to track: stop instead of piling up git processes.
                        self.snapshots_off = true;
                        self.drop_snapshots_here();
                        self.set_status(format!("{e}: agent change tracking is off for this folder"), true);
                        return self.next_snapshot();
                    }
                    Err(e) if e == NOT_A_REPO => {
                        // Not a repository after all: nothing to track.
                        self.drop_snapshots_here();
                        return self.next_snapshot();
                    }
                    Err(e) => {
                        self.set_status(format!("snapshot: {e}"), true);
                        return self.next_snapshot();
                    }
                };
                let next = self.next_snapshot();
                let stored = self.runs.edit(id, |run| {
                    // The two snapshots race; whichever lands second computes the files.
                    run.store_tree(after, tree).then(|| (after || run.files.is_none()) && run.trees().is_some())
                });
                let Some(Some(load_files)) = stored else { return next };
                self.state_dirty = true;
                if load_files {
                    return Task::batch([next, self.load_run_files(id)]);
                }
                return next;
            }
            Msg::RunFiles(id, trees, Ok(files)) => {
                self.runs.edit(id, |run| {
                    // Computed for trees the run still has: a reopened and closed
                    // again run has a newer "after", and its own files coming.
                    if !run.running() && run.trees() == Some((trees.0.as_str(), trees.1.as_str())) {
                        run.files = Some(files);
                    }
                });
            }
            // Leave `files` unknown: an empty list would hide the run (and trim it first).
            Msg::RunFiles(_, _, Err(e)) => self.set_status(format!("Agent run files: {e}"), true),
            Msg::RunPrompt(id, prompt) => {
                // The first title wins: a reopened run keeps the prompt that started it.
                if prompt.is_some() && self.runs.get(id).is_some_and(|r| r.prompt.is_none()) {
                    self.runs.edit(id, |run| run.prompt = prompt);
                    self.state_dirty = true;
                }
            }
            Msg::SelectRun(id) => {
                self.selected_run = if self.selected_run == Some(id) { None } else { Some(id) };
            }
            Msg::MarkAllSeen => {
                self.runs.mark_all_seen();
                self.state_dirty = true;
            }
            Msg::GroupByRun(on) => {
                self.group_by_run = on;
                if let Err(e) = persist::config_set("changes_view", if on { "runs" } else { "files" }) {
                    self.set_status(format!("Could not save view: {e}"), true);
                }
            }
            Msg::OpenRunDiff(id, rel) => {
                self.selected_run = Some(id);
                self.mark_seen(&rel);
                if let Some(source) = self.run_source(id) {
                    return self.open_diff(rel, source);
                }
            }
            Msg::ReviewStep(step) => {
                let Some(d) = self.active_diff() else {
                    return Task::none();
                };
                // Working-tree diffs step through their section of Changes.
                if let DiffSource::Index { staged } = d.source {
                    let list = if staged { &self.git.staged } else { &self.git.unstaged };
                    let Some(i) = list.iter().position(|c| c.path == d.rel) else { return Task::none() };
                    match step_index(i, step, list.len()).and_then(|j| list.get(j)) {
                        Some(c) => {
                            let rel = c.path.clone();
                            self.mark_seen(&rel);
                            return self.open_diff(rel, DiffSource::Index { staged });
                        }
                        None => {
                            let n = list.len();
                            let edge = if step < 0 { "First change" } else { "Last change" };
                            self.set_status(format!("{edge} · {n} {}", if n == 1 { "file" } else { "files" }), false);
                            return Task::none();
                        }
                    }
                }
                let DiffSource::Trees { run: id, .. } = d.source else { return Task::none() };
                let files = self.runs.get(id).and_then(|r| r.files.as_ref());
                let Some((i, files)) = files.and_then(|f| f.iter().position(|c| c.path == d.rel).map(|i| (i, f)))
                else {
                    return Task::none();
                };
                match step_index(i, step, files.len()).and_then(|j| files.get(j)) {
                    Some(c) => return self.update(Msg::OpenRunDiff(id, c.path.clone())),
                    None => {
                        let n = files.len();
                        let edge = if step < 0 { "Start of run" } else { "End of run" };
                        self.set_status(format!("{edge} · {n} {}", if n == 1 { "file" } else { "files" }), false);
                    }
                }
            }
            Msg::RevertRun(id) => {
                if self.git_busy {
                    return Task::none();
                }
                let Some(run) = self.runs.get(id) else { return Task::none() };
                let (Some((from, _)), Some(files)) = (run.trees(), run.files.as_ref()) else { return Task::none() };
                let from = from.to_string();
                let paths: Vec<String> = files.iter().map(|c| c.path.clone()).collect();
                if !self.confirm_action(super::ConfirmAction::RevertRun(id, paths.clone(), from.clone()), "Revert this agent run?",
                    "These files will be restored to their contents before the run. All later edits to them, including changes by you or other agents, will be replaced. A snapshot of the current files is kept for Undo.",
                    paths.clone(), "Revert run") {
                    return Task::none();
                }
                for p in &paths {
                    self.mark_seen(p);
                }
                return self.git_task(move |r| {
                    // As with Discard: a snapshot first is the Undo (later edits come back too).
                    let tree = git::snapshot_tree(r)
                        .map_err(|e| format!("couldn't back the files up, nothing was reverted: {e}"))?;
                    let error = git::restore_from_tree(r, &from, &paths).err();
                    let done = format!("Reverted {} file{}", paths.len(), if paths.len() == 1 { "" } else { "s" });
                    Ok(super::GitOutcome::Discarded { tree, paths, done, close_diffs: false, error })
                });
            }
            Msg::StageRun(id) => {
                let Some(paths) = self
                    .runs
                    .get(id)
                    .and_then(|r| r.files.as_ref())
                    .map(|f| f.iter().map(|c| c.path.clone()).collect::<Vec<_>>())
                else {
                    return Task::none();
                };
                for p in &paths {
                    self.mark_seen(p);
                }
                return self.git_op(move |r| git::stage(r, &paths));
            }
            Msg::Comment(c) if !self.sending_comment => self.comment = c,
            Msg::Comment(_) => {}
            Msg::SendComment if self.sending_comment => {}
            Msg::SendComment => {
                let comment = self.comment.trim().to_string();
                let Some(d) = self.active_diff() else {
                    return Task::none();
                };
                if comment.is_empty() {
                    return Task::none();
                }
                let place = match d.anchor {
                    Some(line) => format!("{}:{line}", d.rel),
                    None => d.rel.clone(),
                };
                // A run's diff goes to the agent that made it; others to the focused pane.
                let target = match &d.source {
                    DiffSource::Trees { run, .. } => Some(*run),
                    DiffSource::Index { .. } => self.runs.origin(&d.rel).map(|r| r.id),
                };
                return self.send_to_agent(format!("{place} — {comment}"), target, true);
            }
            _ => {}
        }
        Task::none()
    }

    pub(super) fn run_effect(&mut self, effect: Effect) -> Task<Msg> {
        let (id, after) = match effect {
            Effect::Before(id) => (id, false),
            Effect::After(id) => (id, true),
        };
        let mut tasks = Vec::new();
        // Before the first snapshot of the folder it isn't known whether it
        // is a repository: queue, and `GitLoaded` starts or drops the queue.
        if !self.snapshots_off && (self.git.root.is_some() || !self.git_loaded) {
            self.snapshot_queue.push_back((self.root.clone(), id, after));
            tasks.push(self.next_snapshot());
        }
        if after
            && let Some(run) = self.runs.get(id)
            && let (Some(kind), Some(home)) = (run.kind.clone(), std::env::var_os("HOME"))
        {
            let (dir, start, end) = (run.dir.clone(), run.started, run.finished.unwrap_or(run.started));
            let home = PathBuf::from(home);
            tasks.push(self.for_root(Task::perform(
                blocking(move || crate::agentlog::last_prompt(&home, &kind, &dir, start, end)),
                move |p| Msg::RunPrompt(id, p),
            )));
        }
        Task::batch(tasks)
    }

    /// Start the next queued snapshot unless one is running. Before the first
    /// git refresh the repository is resolved in the task itself, so an agent
    /// starting right after launch still gets an accurate "before" tree. A
    /// folder shown before resolves its own repository; its reply isn't
    /// dropped with the folder (see `RunSnapshot`).
    pub(super) fn next_snapshot(&mut self) -> Task<Msg> {
        if self.snapshot_running {
            return Task::none();
        }
        // This folder's snapshots are dropped when it can't have any (see `drop_snapshots_here`).
        let Some((root, id, after)) = self.snapshot_queue.pop_front() else { return Task::none() };
        self.snapshot_running = true;
        let known = if root == self.root { self.git.root.clone() } else { None };
        let dir = root.clone();
        Task::perform(
            blocking(move || match known.or_else(|| git::repo_root(&dir)) {
                Some(repo) => git::snapshot_tree(&repo),
                None => Err(NOT_A_REPO.to_string()),
            }),
            move |r| Msg::RunSnapshot(root, id, after, r),
        )
    }

    /// Forget the queued snapshots of the folder shown (it isn't a repository,
    /// or it's too large to track); other folders' still run.
    pub(super) fn drop_snapshots_here(&mut self) {
        let root = self.root.clone();
        self.snapshot_queue.retain(|(r, ..)| *r != root);
    }

    /// Changed files of every run whose trees are known (after a restore).
    pub(super) fn load_all_run_files(&self) -> Task<Msg> {
        let ids: Vec<RunId> = self.runs.runs().iter().filter(|r| r.trees().is_some()).map(|r| r.id).collect();
        Task::batch(ids.into_iter().map(|id| self.load_run_files(id)))
    }

    fn load_run_files(&self, id: RunId) -> Task<Msg> {
        let Some((from, to)) = self.runs.get(id).and_then(|r| r.trees()) else { return Task::none() };
        let (root, from, to) = (self.repo_root(), from.to_string(), to.to_string());
        let trees = (from.clone(), to.clone());
        self.for_root(Task::perform(blocking(move || git::diff_trees(&root, &from, &to)), move |f| {
            Msg::RunFiles(id, trees, f)
        }))
    }

    pub(super) fn save_state(&mut self) {
        if !self.state_dirty {
            return;
        }
        self.state_dirty = false;
        if let Err(e) = persist::save(&self.root, self.runs.runs()) {
            self.set_status(format!("Could not save review state: {e}"), true);
        }
    }

    /// Diff source of run `id`.
    pub fn run_source(&self, id: RunId) -> Option<DiffSource> {
        let run = self.runs.get(id)?;
        let (from, to) = run.trees()?;
        Some(DiffSource::Trees { from: from.to_string(), to: to.to_string(), label: run.agent.clone(), run: id })
    }

    pub(super) fn mark_seen(&mut self, path: &str) {
        if self.runs.mark_seen(path) {
            self.state_dirty = true;
        }
    }
}

/// Index `step` files away from `i` in a list of `len`; `None` past either end.
fn step_index(i: usize, step: isize, len: usize) -> Option<usize> {
    i.checked_add_signed(step).filter(|j| *j < len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_step_stops_at_both_ends() {
        assert_eq!(step_index(0, -1, 3), None);
        assert_eq!(step_index(2, 1, 3), None);
        assert_eq!(step_index(1, 1, 3), Some(2));
        assert_eq!(step_index(1, -1, 3), Some(0));
        assert_eq!(step_index(0, 1, 1), None);
        assert_eq!(step_index(0, -1, 0), None);
    }
}
