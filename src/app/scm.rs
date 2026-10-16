//! Source control: git status, stage/unstage, commit, push/pull and commit-message generation.

use super::{App, DiffSource, Msg, Tab, blocking, first_line};
use crate::git;

use iced::Task;
use iced::widget::text_editor::{self, Content};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A commit message being generated and streamed into the commit box.
pub struct CommitGen {
    id: u64,
    pub(super) cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The box's text before generating, restored on cancel or error.
    previous: String,
    /// The first delta arrived (and cleared the box).
    streaming: bool,
}

/// What a finished git action reports.
#[derive(Debug, Clone)]
pub enum GitOutcome {
    /// A commit was made: the commit box clears. Holds git's summary line.
    Committed(String),
    /// Output to show in the status bar (may be empty).
    Info(String),
    /// Files were discarded; `tree` is the snapshot taken just before (Undo).
    /// `whole`: the files' changes are gone (their diffs close), else one hunk.
    /// Files were thrown away (Discard, a hunk's Discard, Revert of a run). `tree`
    /// is the snapshot taken just before (Undo); `done` says what happened
    /// ("Discarded main.rs"); `close_diffs`: the files' unstaged diffs are empty
    /// now. `error`: it stopped partway; the snapshot still undoes what was done.
    Discarded { tree: String, paths: Vec<String>, done: String, close_diffs: bool, error: Option<String> },
}

/// What to do with one hunk of an index diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkOp {
    Stage,
    Unstage,
    Discard,
}

/// The git action that shows a spinner on its button while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusyOp {
    Commit,
    Push,
    Pull,
}

/// Spinner frames for the generate button.
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Not a widget: focusing it takes the keyboard away from the terminal and
/// inputs while the sign-in dialog is open (so Escape reaches it).
const DIALOG_FOCUS: &str = "mira-dialog";

/// The ChatGPT sign-in dialog. mira explains the flow and asks before it
/// opens the browser; then it waits for the browser to come back.
pub struct SignIn {
    id: u64,
    pub stage: SignInStage,
    cancel: Arc<AtomicBool>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SignInStage {
    /// Explaining what will happen; nothing started yet.
    Ask,
    /// Waiting for the browser. `url` is the sign-in link once known;
    /// `opened` says whether a browser could be started for it.
    Waiting { url: Option<String>, opened: bool },
}

/// Progress of a sign-in, from its thread.
#[derive(Debug, Clone)]
pub enum SignInEvent {
    Url(String, bool),
    /// The account's email (maybe empty), or why it failed.
    Done(Result<String, String>),
}

impl App {
    pub(super) fn update_scm(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::GitLoaded(generation, result) => {
                // A newer refresh started after this one (e.g. after a commit): drop it.
                if generation != self.git_gen {
                    return Task::none();
                }
                self.refreshing = false;
                match result {
                    Ok(mut snap) => {
                        self.git_loaded = true;
                        let mut tasks = Vec::new();
                        // Ignored files weren't listed (Files is hidden): keep the
                        // last list, and leave it out of the comparison.
                        let kept = (!self.git_with_ignored).then(|| std::mem::take(&mut self.git.ignored));
                        // Only a real change re-renders the panel and recomputes the diff.
                        if snap != self.git {
                            if let Some(kept) = kept {
                                snap.ignored = kept;
                            }
                            self.repo_prefix =
                                snap.root.as_deref().map(|r| prefix_in(&self.root, r)).unwrap_or_default();
                            self.git = snap;
                            self.explorer.set_marks(crate::explorer::Marks::new(&self.git, &self.repo_prefix));
                            if self.active_diff_is_index() {
                                tasks.push(self.reload_active_diff());
                            }
                        } else if let Some(kept) = kept {
                            self.git.ignored = kept;
                        }
                        // Snapshots queued before it was known whether this is a repository
                        // (another folder's queued ones go on either way).
                        if self.git.root.is_none() {
                            self.drop_snapshots_here();
                        }
                        tasks.push(self.next_snapshot());
                        return Task::batch(tasks);
                    }
                    Err(e) => {
                        // Loaded, if unhappily: don't keep showing "Loading changes…".
                        self.git_loaded = true;
                        if self.git.root.is_none() {
                            self.drop_snapshots_here();
                        }
                        self.set_error_detail(git::error_summary(&e), e);
                    }
                }
            }
            Msg::OpenDiff(rel, staged) => {
                self.mark_seen(&rel);
                return self.open_diff(rel, DiffSource::Index { staged });
            }
            Msg::Stage(paths) => return self.git_op(move |r| git::stage(r, &paths)),
            Msg::Discard(paths) if !self.git_busy => {
                // Only unstaged, non-conflicted changes; the status letter says how.
                let files: Vec<(String, char)> = paths
                    .iter()
                    .filter_map(|p| self.git.unstaged.iter().find(|c| &c.path == p && c.code != '!'))
                    .map(|c| (c.path.clone(), c.code))
                    .collect();
                if files.is_empty() {
                    return Task::none();
                }
                self.confirm_discard_all = false;
                return self.git_task(move |r| {
                    // The snapshot is the Undo: without it, nothing is discarded.
                    let tree = git::snapshot_tree(r)
                        .map_err(|e| format!("couldn't back the files up, nothing was discarded: {e}"))?;
                    // From here on, a failure still hands back the snapshot: some files may be discarded.
                    let error = git::discard(r, &files).err();
                    let paths: Vec<String> = files.into_iter().map(|(p, _)| p).collect();
                    let done = match paths.as_slice() {
                        [one] => format!("Discarded {}", one.rsplit('/').next().unwrap_or(one)),
                        many => format!("Discarded {} files", many.len()),
                    };
                    Ok(GitOutcome::Discarded { tree, paths, done, close_diffs: true, error })
                });
            }
            Msg::Hunk(op, id, i) if !self.git_busy => {
                let Some(d) = self.active_diff() else { return Task::none() };
                let rel = d.rel.clone();
                let patch = d.diff.as_ref().filter(|diff| diff.id == id).and_then(|diff| diff.patch.as_ref());
                let Some(text) = patch.and_then(|p| p.single(i)) else { return Task::none() };
                let name = rel.rsplit('/').next().unwrap_or("").to_string();
                return match op {
                    HunkOp::Stage => self.git_op(move |r| {
                        git::apply(r, &text, &["--cached"]).map(|_| format!("Staged a change in {name}"))
                    }),
                    HunkOp::Unstage => self.git_op(move |r| {
                        git::apply(r, &text, &["--cached", "-R"]).map(|_| format!("Unstaged a change in {name}"))
                    }),
                    HunkOp::Discard => self.git_task(move |r| {
                        let tree = git::snapshot_tree(r)
                            .map_err(|e| format!("couldn't back the file up, nothing was discarded: {e}"))?;
                        git::apply(r, &text, &["-R"])?;
                        let done = format!("Discarded a change in {name}");
                        Ok(GitOutcome::Discarded { tree, paths: vec![rel], done, close_diffs: false, error: None })
                    }),
                };
            }
            Msg::Hunk(..) => {}
            Msg::DiscardAll if !self.git_busy => {
                let paths: Vec<String> =
                    self.git.unstaged.iter().filter(|c| c.code != '!').map(|c| c.path.clone()).collect();
                if paths.is_empty() {
                    return Task::none();
                }
                if self.git.truncated {
                    self.set_status(
                        format!("Over {} changes, not all listed: discard them in a terminal", git::MAX_CHANGES),
                        true,
                    );
                    return Task::none();
                }
                if !self.confirm_discard_all {
                    let untracked = self.git.unstaged.iter().filter(|c| c.code == 'U').count();
                    let deleted =
                        if untracked > 0 { format!(" ({untracked} untracked deleted)") } else { String::new() };
                    self.clear_confirms();
                    self.confirm_discard_all = true;
                    self.set_confirm(&format!(
                        "Click again to discard {} changes{deleted} · Undo stays available · Esc cancels",
                        paths.len()
                    ));
                    return Task::none();
                }
                return self.update(Msg::Discard(paths));
            }
            Msg::Discard(_) | Msg::DiscardAll => {}
            Msg::UndoDiscard if !self.git_busy => {
                let Some((tree, paths)) = self.undo_discard.take() else { return Task::none() };
                self.clear_status();
                return self.git_op(move |r| {
                    git::restore_from_tree(r, &tree, &paths)
                        .map(|_| format!("Restored {} file{}", paths.len(), if paths.len() == 1 { "" } else { "s" }))
                });
            }
            Msg::UndoDiscard => {}
            Msg::OpenChanges(staged) => {
                let list = if staged { &self.git.staged } else { &self.git.unstaged };
                if let Some(first) = list.first() {
                    let rel = first.path.clone();
                    self.mark_seen(&rel);
                    return self.open_diff(rel, DiffSource::Index { staged });
                }
            }
            Msg::OpenChangedFile(rel) => {
                let path = self.repo_root().join(&rel);
                self.mark_seen(&rel);
                return self.update(Msg::OpenFile(path, None));
            }
            // A late "left" from the previous row must not hide the new one's buttons.
            Msg::HoverChange(key, true) => self.hovered_change = Some(key),
            Msg::HoverChange(key, false) if self.hovered_change == Some(key) => self.hovered_change = None,
            Msg::HoverChange(..) => {}
            Msg::Unstage(mut paths) => {
                // A staged rename also staged its old path's deletion: both go back.
                let origs: Vec<String> =
                    self.git.staged.iter().filter(|c| paths.contains(&c.path)).filter_map(|c| c.orig.clone()).collect();
                paths.extend(origs);
                return self.git_op(move |r| git::unstage(r, &paths));
            }
            // The box is the stream's output while generating: allow selecting, not editing.
            Msg::CommitEdit(action) if self.commit_gen.is_some() && action.is_edit() => {}
            Msg::CommitEdit(action) => self.commit_history.perform(&mut self.commit_msg, action),
            Msg::CommitUndo if self.commit_gen.is_none() => {
                self.commit_history.undo(&mut self.commit_msg);
            }
            Msg::CommitRedo if self.commit_gen.is_none() => {
                self.commit_history.redo(&mut self.commit_msg);
            }
            Msg::CommitUndo | Msg::CommitRedo => {}
            Msg::Commit => {
                if self.commit_gen.is_some() {
                    self.set_status("Wait for the message to finish (or click ✦ to stop)", true);
                    return Task::none();
                }
                let message = self.commit_msg.text().trim().to_string();
                if message.is_empty() {
                    self.set_status("Write a commit message first", true);
                    return Task::none();
                }
                // git refuses to commit unmerged paths; staging them would mark
                // files with conflict markers as resolved.
                let conflicts = self.git.conflicts();
                if conflicts > 0 {
                    self.set_status(
                        format!("Resolve {conflicts} merge conflict(s) first: fix the files, then stage them"),
                        true,
                    );
                    return Task::none();
                }
                // Nothing staged: "Commit All" stages the listed changes of this folder.
                let commit_all = self.git.staged.is_empty();
                if commit_all && self.git.truncated {
                    self.set_status(
                        format!("Over {} changes, not all listed: stage what to commit first", git::MAX_CHANGES),
                        true,
                    );
                    return Task::none();
                }
                let stage_all: Vec<(String, bool)> = if commit_all {
                    let listed = self.git.unstaged.iter().filter(|c| c.code != '!');
                    // A worktree rename stages its old path's deletion too.
                    listed
                        .flat_map(|c| {
                            let orig = c.orig.clone().map(|o| (o, false));
                            std::iter::once((c.path.clone(), c.code == 'U')).chain(orig)
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                if commit_all && stage_all.is_empty() {
                    self.set_status("Nothing to commit", true);
                    return Task::none();
                }
                let prefix = self.repo_prefix.clone();
                return self
                    .spinning(BusyOp::Commit, |a| a.git_task(move |r| commit_in(r, &message, &prefix, stage_all)));
            }
            Msg::GitInit => return self.git_op(git::init),
            // The dialog asks first; ✦ does nothing while it is open.
            Msg::GenerateCommit if self.signin.is_some() => {}
            Msg::GenerateCommit => {
                if let Some(job) = self.commit_gen.take() {
                    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    self.set_commit_text(&job.previous);
                    self.set_status("Commit message generation cancelled", false);
                    return Task::none();
                }
                if self.git.root.is_none() {
                    return Task::none();
                }
                let staged = !self.git.staged.is_empty();
                if !staged && self.git.unstaged.is_empty() {
                    self.set_status("Nothing to commit", true);
                    return Task::none();
                }
                if crate::commitgen::needs_sign_in() {
                    self.signin_seq += 1;
                    self.signin = Some(SignIn {
                        id: self.signin_seq,
                        stage: SignInStage::Ask,
                        cancel: Arc::new(AtomicBool::new(false)),
                    });
                    return iced::widget::operation::focus(DIALOG_FOCUS);
                }
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                // Close the open typing group first, so the history's last
                // state is `previous` and the streamed text is never recorded.
                self.commit_history.flush(&self.commit_msg);
                self.commit_gen_seq += 1;
                let id = self.commit_gen_seq;
                self.commit_gen =
                    Some(CommitGen { id, cancel: cancel.clone(), previous: self.commit_msg.text(), streaming: false });
                let what = if staged { "staged changes" } else { "all changes" };
                self.set_status(format!("Generating a commit message for {what}…"), false);
                let (root, prefix) = (self.repo_root(), self.repo_prefix.clone());
                let (tx, rx) = iced::futures::channel::mpsc::unbounded();
                std::thread::spawn(move || {
                    let mut emit = |e| {
                        let _ = tx.unbounded_send(e);
                    };
                    let result = crate::commitgen::generate(&root, staged, &prefix, &cancel, &mut emit);
                    emit(crate::commitgen::Event::Done(result));
                });
                return Task::run(rx, move |e| Msg::CommitGen(id, e));
            }
            Msg::CommitGen(id, event) => {
                use crate::commitgen::Event;
                if let Event::Account(account) = event {
                    self.chatgpt_account = account;
                    return Task::none();
                }
                // Events of a cancelled run arrive after `commit_gen` changed; drop them.
                let Some(job) = self.commit_gen.as_mut().filter(|g| g.id == id) else {
                    return Task::none();
                };
                match event {
                    Event::Delta(d) => {
                        // Append at the end instead of rebuilding the box from the
                        // whole text per delta (quadratic, and every line reshaped).
                        if !std::mem::replace(&mut job.streaming, true) {
                            self.commit_msg = Content::new();
                        }
                        append(&mut self.commit_msg, d);
                    }
                    Event::Done(result) => {
                        let previous = self.commit_gen.take().map(|g| g.previous).unwrap_or_default();
                        match result {
                            Ok(message) => {
                                self.set_commit_text(&message);
                                // One step: Ctrl+Z brings back what the box had before.
                                let start = text_editor::Cursor {
                                    position: text_editor::Position { line: 0, column: 0 },
                                    selection: None,
                                };
                                self.commit_history.record(&previous, &message, start, self.commit_msg.cursor());
                                self.set_status("Commit message ready — review it, then Commit", false);
                            }
                            Err(e) => {
                                self.set_commit_text(&previous);
                                self.set_status(format!("Commit message: {e}"), true);
                            }
                        }
                    }
                    Event::Account(_) => {}
                }
            }
            Msg::Spin => self.spin = (self.spin + 1) % SPINNER.len(),
            Msg::ChatGptSignOut if !self.confirm_signout => {
                self.confirm_signout = true;
                self.set_confirm("Click again to sign out of ChatGPT · Esc cancels");
            }
            Msg::ChatGptSignOut => {
                self.confirm_signout = false;
                match crate::openai::sign_out() {
                    Ok(()) => {
                        self.chatgpt_account = None;
                        self.set_status("Signed out of ChatGPT", false);
                    }
                    Err(e) => self.set_status(format!("Sign out: {e}"), true),
                }
            }
            Msg::SignInStart => {
                let Some(signin) = self.signin.as_mut().filter(|s| s.stage == SignInStage::Ask) else {
                    return Task::none();
                };
                signin.stage = SignInStage::Waiting { url: None, opened: false };
                let (id, cancel) = (signin.id, Arc::clone(&signin.cancel));
                let (tx, rx) = iced::futures::channel::mpsc::unbounded();
                std::thread::spawn(move || {
                    let result = crate::commitgen::sign_in(&cancel, &mut |url, opened| {
                        let _ = tx.unbounded_send(SignInEvent::Url(url.to_string(), opened));
                    });
                    let _ = tx.unbounded_send(SignInEvent::Done(result));
                });
                return Task::run(rx, move |e| Msg::SignInEvent(id, e));
            }
            Msg::SignInCancel => {
                if let Some(signin) = self.signin.take() {
                    signin.cancel.store(true, Ordering::Relaxed);
                    self.set_status("Sign-in cancelled", false);
                }
            }
            Msg::SignInCopyLink => {
                if let Some(SignIn { stage: SignInStage::Waiting { url: Some(url), .. }, .. }) = &self.signin {
                    let url = url.clone();
                    self.set_status("Sign-in link copied: open it in your browser", false);
                    return iced::clipboard::write(url);
                }
            }
            Msg::SignInEvent(id, event) => {
                // Events of a dialog that was cancelled (or replaced) are dropped.
                let Some(signin) = self.signin.as_mut().filter(|s| s.id == id) else {
                    return Task::none();
                };
                match event {
                    SignInEvent::Url(url, opened) => signin.stage = SignInStage::Waiting { url: Some(url), opened },
                    SignInEvent::Done(result) => {
                        self.signin = None;
                        match result {
                            Ok(email) => {
                                let who = if email.is_empty() { String::new() } else { format!(" as {email}") };
                                self.set_status(format!("Signed in to ChatGPT{who}"), false);
                                self.chatgpt_account = Some(email);
                                // What the user asked for in the first place.
                                return self.update(Msg::GenerateCommit);
                            }
                            Err(e) if e == "Cancelled" => {}
                            Err(e) => self.set_status(format!("ChatGPT sign-in: {e}"), true),
                        }
                    }
                }
            }
            // Said only once the action really starts (one git action at a time).
            Msg::Push if !self.git_busy => {
                self.set_status("Pushing…", false);
                let task = self.spinning(BusyOp::Push, |a| a.git_op(|r| git::push(r).map(|_| "Pushed".into())));
                return task;
            }
            Msg::Pull if !self.git_busy => {
                self.set_status("Pulling…", false);
                return self.spinning(BusyOp::Pull, |a| {
                    a.git_op(|r| git::pull(r).map(|o| first_line(&o).unwrap_or("Pulled").into()))
                });
            }
            Msg::Push | Msg::Pull => {}
            Msg::GitDone(epoch, result) => {
                self.git_busy = false;
                self.busy_op = None;
                // Started in a folder mira has left: its commit box and status
                // belong to the old folder, and the new one was refreshed on switch.
                if epoch != self.root_epoch {
                    // Still say when it failed (a rejected push, a failing hook).
                    if let Err(e) = result {
                        self.set_error_detail(format!("Previous folder: {}", git::error_summary(&e)), e);
                    }
                    return Task::none();
                }
                match result {
                    Ok(GitOutcome::Committed(summary)) => {
                        self.commit_msg = Content::new();
                        self.commit_history = crate::history::History::default();
                        self.set_status(summary, false);
                    }
                    Ok(GitOutcome::Info(out)) => {
                        if let Some(line) = first_line(&out) {
                            self.set_status(line.to_string(), false);
                        }
                    }
                    Ok(GitOutcome::Discarded { tree, paths, done, close_diffs, error }) => {
                        // Their diffs would only say "No changes" now.
                        for i in (0..self.tabs.len()).rev() {
                            if close_diffs
                                && matches!(&self.tabs[i], Tab::Diff(d) if matches!(d.source, DiffSource::Index { staged: false }) && paths.contains(&d.rel))
                            {
                                self.close_tab(i);
                            }
                        }
                        self.undo_discard = Some((tree, paths));
                        let msg = match error {
                            None => done,
                            Some(e) => {
                                format!("Stopped partway: {} — Undo puts back what was changed", git::error_summary(&e))
                            }
                        };
                        self.set_status_action(msg, "Undo", Msg::UndoDiscard);
                    }
                    Err(e) => self.set_error_detail(git::error_summary(&e), e),
                }
                // Start a fresh refresh even if one is running: that one may
                // predate this action, and its reply is dropped (`git_gen`).
                self.refreshing = false;
                self.reload_changed_files();
                return Task::batch([self.refresh_git(), self.reload_active_diff()]);
            }
            _ => {}
        }
        Task::none()
    }

    pub(super) fn refresh_git(&mut self) -> Task<Msg> {
        if self.refreshing {
            return Task::none();
        }
        self.refreshing = true;
        self.git_gen += 1;
        self.git_with_ignored = self.files_visible();
        let (root, generation, with_ignored) = (self.root.clone(), self.git_gen, self.git_with_ignored);
        let cache = Arc::clone(&self.git_cache);
        self.for_root(Task::perform(
            blocking(move || {
                let mut cache = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                git::snapshot(&root, &mut cache, with_ignored)
            }),
            move |r| Msg::GitLoaded(generation, r),
        ))
    }

    /// Run a git action whose output is just informative (stage, push, …).
    pub(super) fn git_op(&mut self, op: impl FnOnce(&Path) -> Result<String, String> + Send + 'static) -> Task<Msg> {
        self.git_task(move |r| op(r).map(GitOutcome::Info))
    }

    /// Start a git action with `start` (through `git_task`); its button spins
    /// until `GitDone`. Nothing starts while another action runs.
    fn spinning(&mut self, op: BusyOp, start: impl FnOnce(&mut Self) -> Task<Msg>) -> Task<Msg> {
        if self.git_busy {
            return Task::none();
        }
        let task = start(self);
        if self.git_busy {
            self.busy_op = Some(op);
        }
        task
    }

    /// Run one git action at a time in the repository, off the UI thread.
    pub(super) fn git_task(
        &mut self,
        op: impl FnOnce(&Path) -> Result<GitOutcome, String> + Send + 'static,
    ) -> Task<Msg> {
        if self.git_busy {
            return Task::none();
        }
        self.git_busy = true;
        // Before the folder's first snapshot the repository root isn't known yet:
        // ask git, or paths relative to it would resolve in a subfolder.
        let (known, root, epoch) = (self.git.root.clone(), self.root.clone(), self.root_epoch);
        let run = move || {
            let repo = known.or_else(|| git::repo_root(&root)).unwrap_or(root);
            op(&repo)
        };
        Task::perform(blocking(run), move |r| Msg::GitDone(epoch, r))
    }

    /// Replace the commit box's text, cursor at the end (where streaming writes).
    fn set_commit_text(&mut self, text: &str) {
        self.commit_msg = Content::with_text(text);
        self.commit_msg.perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
    }
}

/// Add `text` at the end of `content` (where streaming writes).
fn append(content: &mut Content, text: String) {
    content.perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
    content.perform(text_editor::Action::Edit(text_editor::Edit::Paste(Arc::new(text))));
}

/// Commit in the repository at `repo`: what's staged, or with `stage_all`
/// (Commit All: path, untracked) those changes staged first. `prefix` is the
/// opened folder inside the repository.
fn commit_in(repo: &Path, message: &str, prefix: &str, stage_all: Vec<(String, bool)>) -> Result<GitOutcome, String> {
    // `git commit` takes the whole index: files staged outside
    // this folder are not on screen and must not ride along.
    let outside = git::staged_outside(repo, prefix)?;
    if let Some(first) = outside.first() {
        return Err(format!(
            "{} file(s) outside this folder are staged ({first}…): commit or unstage them first",
            outside.len()
        ));
    }
    // The list is up to a refresh old: an untracked file deleted
    // since would fail `git add` and with it the whole commit.
    let stage_all = still_there(repo, stage_all);
    if !stage_all.is_empty() {
        git::stage(repo, &stage_all)?;
    }
    match git::commit(repo, message) {
        Ok(out) => Ok(GitOutcome::Committed(first_line(&out).unwrap_or("Committed").to_string())),
        Err(e) => {
            // A rejected Commit All (e.g. by a hook) leaves the index as it was:
            // left staged, the next Commit would commit only those, a different one.
            if !stage_all.is_empty() {
                let _ = git::unstage(repo, &stage_all);
            }
            Err(e)
        }
    }
}

/// The paths of `listed` (path, untracked) still to stage: an untracked file
/// that's gone has nothing to add (a tracked one's deletion is staged).
fn still_there(repo: &Path, listed: Vec<(String, bool)>) -> Vec<String> {
    listed
        .into_iter()
        .filter(|(p, untracked)| !untracked || repo.join(p).symlink_metadata().is_ok())
        .map(|(p, _)| p)
        .collect()
}

/// `root` relative to the repository at `repo`, with a trailing slash ("" at
/// the top, or when `root` is not lexically inside `repo`).
fn prefix_in(root: &Path, repo: &Path) -> String {
    match root.strip_prefix(repo).map(|p| p.to_string_lossy().into_owned()) {
        Ok(p) if !p.is_empty() => format!("{p}/"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_deltas_append_at_the_end() {
        let mut c = Content::new();
        for d in ["feat: a", "dd x\n", "\nbody", " é"] {
            append(&mut c, d.to_string());
        }
        // A click elsewhere while streaming doesn't move where text lands.
        c.perform(text_editor::Action::Move(text_editor::Motion::DocumentStart));
        append(&mut c, "!".into());
        assert_eq!(c.text(), "feat: add x\n\nbody é!");
    }

    #[test]
    fn prefix_is_the_folder_inside_its_repo() {
        assert_eq!(prefix_in(Path::new("/r"), Path::new("/r")), "");
        assert_eq!(prefix_in(Path::new("/r/a/b"), Path::new("/r")), "a/b/");
        assert_eq!(prefix_in(Path::new("/elsewhere"), Path::new("/r")), "");
    }

    /// A repository with one commit of `a.txt`; commits as a local test identity.
    fn repo(tag: &str) -> std::path::PathBuf {
        let dir = crate::files::test_dir(tag);
        for args in [
            &["init", "-q"][..],
            &["config", "user.name", "t"],
            &["config", "user.email", "t@t"],
            &["config", "commit.gpgsign", "false"],
        ] {
            git::git(&dir, args).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        git::git(&dir, &["add", "-A"]).unwrap();
        git::git(&dir, &["commit", "-qm", "base"]).unwrap();
        dir
    }

    fn staged(dir: &Path) -> String {
        git::git(dir, &["diff", "--cached", "--name-only"]).unwrap().trim().to_string()
    }

    #[test]
    fn commit_all_skips_untracked_files_deleted_since_the_list() {
        let dir = repo("commit-gone");
        std::fs::write(dir.join("a.txt"), "edited\n").unwrap();
        // Listed as untracked, then deleted by an agent before the click.
        let listed = vec![("a.txt".to_string(), false), ("gone.txt".to_string(), true)];
        assert!(matches!(commit_in(&dir, "edit", "", listed), Ok(GitOutcome::Committed(_))));
        assert_eq!(git::git(&dir, &["log", "-1", "--format=%s"]).unwrap().trim(), "edit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rejected_commit_all_unstages_what_it_staged() {
        let dir = repo("commit-hook");
        let hook = dir.join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("a.txt"), "edited\n").unwrap();
        std::fs::write(dir.join("new.txt"), "new\n").unwrap();
        let listed = vec![("a.txt".to_string(), false), ("new.txt".to_string(), true)];
        assert!(commit_in(&dir, "edit", "", listed).is_err());
        assert_eq!(staged(&dir), "", "the index is as it was");
        // A plain Commit leaves what you staged yourself alone.
        git::stage(&dir, &["a.txt".to_string()]).unwrap();
        assert!(commit_in(&dir, "edit", "", Vec::new()).is_err());
        assert_eq!(staged(&dir), "a.txt");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
