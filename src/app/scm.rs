//! Source control: git status, stage/unstage, commit, push/pull and commit-message generation.

use super::{App, ConfirmAction, DiffSource, Msg, Tab, blocking, first_line};
use crate::git;

use iced::Task;
use iced::widget::text_editor::{self, Content};
use std::path::{Path, PathBuf};
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
    Switched(String),
    Undone {
        message: String,
        summary: String,
    },
    /// Output to show in the status bar (may be empty).
    Info(String),
    /// Files were discarded; `tree` is the snapshot taken just before (Undo).
    /// `whole`: the files' changes are gone (their diffs close), else one hunk.
    /// Files were thrown away (Discard, a hunk's Discard, Revert of a run). `tree`
    /// is the snapshot taken just before (Undo); `done` says what happened
    /// ("Discarded main.rs"); `close_diffs`: the files' unstaged diffs are empty
    /// now. `error`: it stopped partway; the snapshot still undoes what was done.
    Discarded {
        tree: String,
        paths: Vec<String>,
        done: String,
        close_diffs: bool,
        error: Option<String>,
    },
}

/// What to do with one hunk of an index diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HunkOp {
    Stage,
    Unstage,
    Discard,
}

/// The running action, including its target so unrelated buttons stay still.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BusyOp {
    Commit,
    Branches,
    SwitchBranch(String),
    UndoCommit,
    Push,
    Pull,
    Init,
    Stage(Vec<String>),
    Unstage(Vec<String>),
    Discard(Vec<String>),
    DiscardAll,
    Undo,
    StageRun(crate::runs::RunId),
    RevertRun(crate::runs::RunId),
    Hunk(HunkOp, u64, usize),
}

impl BusyOp {
    pub(super) fn from_message(msg: &Msg) -> Option<Self> {
        Some(match msg {
            Msg::BranchesOpen => Self::Branches,
            Msg::BranchPick(_, branch) => Self::SwitchBranch(branch.clone()),
            Msg::UndoLastCommit | Msg::UndoCommitApply(_) => Self::UndoCommit,
            Msg::Commit => Self::Commit,
            Msg::Push => Self::Push,
            Msg::Pull => Self::Pull,
            Msg::GitInit => Self::Init,
            Msg::Stage(paths) => Self::Stage(paths.clone()),
            Msg::Unstage(paths) => Self::Unstage(paths.clone()),
            Msg::Discard(paths) => Self::Discard(paths.clone()),
            Msg::DiscardAll => Self::DiscardAll,
            Msg::UndoDiscard => Self::Undo,
            Msg::StageRun(id) => Self::StageRun(*id),
            Msg::RevertRun(id) => Self::RevertRun(*id),
            Msg::Hunk(op, diff, i) => Self::Hunk(*op, *diff, *i),
            _ => return None,
        })
    }

    /// Compare without cloning file lists during view construction.
    pub fn matches(&self, msg: &Msg) -> bool {
        match (self, msg) {
            (Self::Branches, Msg::BranchesOpen) | (Self::UndoCommit, Msg::UndoLastCommit | Msg::UndoCommitApply(_)) => {
                true
            }
            (Self::SwitchBranch(a), Msg::BranchPick(_, b)) => a == b,
            (Self::Commit, Msg::Commit)
            | (Self::Push, Msg::Push)
            | (Self::Pull, Msg::Pull)
            | (Self::Init, Msg::GitInit)
            | (Self::DiscardAll, Msg::DiscardAll)
            | (Self::Undo, Msg::UndoDiscard) => true,
            (Self::Stage(a), Msg::Stage(b))
            | (Self::Unstage(a), Msg::Unstage(b))
            | (Self::Discard(a), Msg::Discard(b)) => a == b,
            (Self::StageRun(a), Msg::StageRun(b)) | (Self::RevertRun(a), Msg::RevertRun(b)) => a == b,
            (Self::Hunk(a, d, i), Msg::Hunk(b, e, j)) => a == b && d == e && i == j,
            _ => false,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Branches => "Loading branches…",
            Self::SwitchBranch(_) => "Switching branch…",
            Self::UndoCommit => "Undoing last commit…",
            Self::Commit => "Committing…",
            Self::Push => "Pushing…",
            Self::Pull => "Pulling…",
            Self::Init => "Initializing repository…",
            Self::Stage(_) | Self::StageRun(_) | Self::Hunk(HunkOp::Stage, ..) => "Staging…",
            Self::Unstage(_) | Self::Hunk(HunkOp::Unstage, ..) => "Unstaging…",
            Self::Discard(_) | Self::DiscardAll | Self::Hunk(HunkOp::Discard, ..) => "Discarding…",
            Self::Undo => "Restoring…",
            Self::RevertRun(_) => "Reverting…",
        }
    }
}

/// Shared spinner frames for user actions.
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
                let manual = std::mem::take(&mut self.manual_refresh);
                match result {
                    Ok(mut snap) => {
                        self.git_loaded = true;
                        if manual {
                            self.set_status("Changes refreshed", false);
                        }
                        let mut tasks = Vec::new();
                        // Ignored files weren't listed (Files is hidden): keep the
                        // last list, and leave it out of the comparison.
                        let kept = (!self.git_with_ignored).then(|| std::mem::take(&mut self.git.ignored));
                        // Only a real change re-renders the panel and recomputes the diff.
                        if snap != self.git {
                            if self
                                .commit_view
                                .selected
                                .as_ref()
                                .is_some_and(|selected| !snap.log.iter().any(|commit| commit.hash == selected.hash))
                            {
                                self.commit_view.selected = None;
                            }
                            if let Some(kept) = kept {
                                snap.ignored = kept;
                            }
                            self.repo_prefix =
                                snap.root.as_deref().map(|r| prefix_in(&self.root, r)).unwrap_or_default();
                            self.git = snap;
                            self.git_rev += 1;
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
            Msg::RefreshChanges => {
                if self.git_busy || self.manual_refresh {
                    return Task::none();
                }
                self.manual_refresh = true;
                self.reload_changed_files();
                return self.refresh_git();
            }
            Msg::OpenDiff(rel, staged) => {
                self.mark_seen(&rel);
                return self.open_diff(rel, DiffSource::Index { staged });
            }
            Msg::Stage(paths) => return self.git_op(move |r| git::stage(r, &paths)),
            Msg::Discard(paths) if !self.git_busy => {
                let requested: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
                let changes: Vec<git::Change> = self
                    .git
                    .unstaged
                    .iter()
                    .filter(|c| c.code != '!' && requested.contains(c.path.as_str()))
                    .cloned()
                    .collect();
                if changes.is_empty() {
                    return Task::none();
                }
                let paths: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
                let new_files = changes.iter().filter(|c| c.code == 'U').count();
                let detail = format!(
                    "All unstaged edits in the listed files will be discarded. {new_files} new file(s) will be deleted. Staged changes stay. A snapshot is kept for Undo, which can restore the discarded files."
                );
                if !self.confirm_action(
                    ConfirmAction::Discard(changes),
                    "Discard file changes?",
                    detail,
                    paths.clone(),
                    "Discard changes",
                ) {
                    return Task::none();
                }
                return self.discard_paths(paths);
            }
            Msg::Hunk(op, id, i) if !self.git_busy => {
                let Some(d) = self.active_diff() else { return Task::none() };
                let rel = d.rel.clone();
                let patch = d.diff.as_ref().filter(|diff| diff.id == id).and_then(|diff| diff.patch.as_ref());
                let Some(text) = patch.and_then(|p| p.single(i)) else { return Task::none() };
                if op == HunkOp::Discard && !self.confirm_action(
                    ConfirmAction::Hunk(id, i), "Discard this change?",
                    "This hunk's unstaged edits will be removed from the file. Other hunks and staged changes stay. A snapshot of the file is kept for Undo; restoring it also replaces any later edits to that file.",
                    vec![rel.clone()], "Discard change") {
                    return Task::none();
                }
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
                let untracked = self.git.unstaged.iter().filter(|c| c.code == 'U').count();
                if !self.confirm_action(ConfirmAction::DiscardAll(self.git.unstaged.iter().filter(|c| c.code != '!').cloned().collect()), "Discard all changes?",
                    format!("All unstaged edits in these {} files will be discarded, and {untracked} new file(s) will be deleted. Staged changes stay. A snapshot is kept for Undo.", paths.len()),
                    paths.clone(), "Discard all changes") {
                    return Task::none();
                }
                return self.discard_paths(paths);
            }
            Msg::Discard(_) | Msg::DiscardAll => {}
            Msg::UndoDiscard if !self.git_busy => {
                let Some((tree, paths)) = self.undo_discard.as_ref() else { return Task::none() };
                if !self.confirm_action(ConfirmAction::UndoDiscard(tree.clone(), paths.clone()), "Restore discarded files?",
                    "These files will be restored from the snapshot. Any edits made to them after the discard, including edits by an agent, will be replaced.",
                    paths.clone(), "Restore files") {
                    return Task::none();
                }
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
                // A staged rename also staged its old path's deletion: both go back
                // (a copy's source wasn't touched by it).
                let origs: Vec<String> = self
                    .git
                    .staged
                    .iter()
                    .filter(|c| c.code == 'R' && paths.contains(&c.path))
                    .filter_map(|c| c.orig.clone())
                    .collect();
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
                return self.git_task(move |r| commit_in(r, &message, &prefix, stage_all));
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
            Msg::ChatGptSignOut => {
                if !self.confirm_action(ConfirmAction::SignOut, "Sign out of ChatGPT?",
                    "The saved ChatGPT session on this device will be removed. To generate commit messages with this account again, you will need to sign in. Your files and agent sessions are not affected.",
                    Vec::new(), "Sign out") {
                    return Task::none();
                }
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
            Msg::Push if !self.git_busy => return self.start_remote(git::remote::Operation::Push),
            Msg::Pull if !self.git_busy => return self.start_remote(git::remote::Operation::Pull),
            Msg::GitDone(epoch, result) => {
                self.git_busy = false;
                let op = self.busy_op.take();
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
                    Ok(GitOutcome::Switched(summary)) => {
                        for i in (0..self.tabs.len()).rev() {
                            if matches!(self.tabs[i], Tab::Diff(_)) {
                                self.close_tab(i);
                            }
                        }
                        self.undo_discard = None;
                        self.selected_run = None;
                        self.commit_view.selected = None;
                        self.hovered_change = None;
                        self.set_status(summary, false);
                    }
                    Ok(GitOutcome::Undone { message, summary }) => {
                        if self.commit_msg.text().trim().is_empty() {
                            self.set_commit_text(&message);
                        }
                        self.set_status(summary, false);
                    }
                    Ok(GitOutcome::Committed(summary)) => {
                        self.commit_msg = Content::new();
                        self.commit_history = crate::history::History::default();
                        self.set_status(summary, false);
                    }
                    Ok(GitOutcome::Info(out)) => {
                        let fallback = match op {
                            Some(BusyOp::Stage(_) | BusyOp::StageRun(_)) => "Changes staged",
                            Some(BusyOp::Unstage(_)) => "Changes unstaged",
                            _ => "Git action completed",
                        };
                        self.set_status(first_line(&out).unwrap_or(fallback).to_string(), false);
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
                    Err(e) => return self.git_failure("Git action", e, None, false),
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

    fn discard_paths(&mut self, paths: Vec<String>) -> Task<Msg> {
        // Only unstaged, non-conflicted changes; the status letter says how.
        let files: Vec<(String, char)> = paths
            .iter()
            .filter_map(|p| self.git.unstaged.iter().find(|c| &c.path == p && c.code != '!'))
            .flat_map(|c| match (&c.orig, c.code) {
                // A worktree rename (after `git add -N`): the new name goes, the old comes back.
                (Some(orig), 'R') => vec![(c.path.clone(), 'A'), (orig.clone(), 'D')],
                _ => vec![(c.path.clone(), c.code)],
            })
            .collect();
        if files.is_empty() {
            return Task::none();
        }
        self.git_task(move |r| discard_in(r, files))
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

    pub(super) fn cancel_commit_generation_for_head_change(&mut self) {
        if let Some(job) = self.commit_gen.take() {
            job.cancel.store(true, Ordering::Relaxed);
            self.set_commit_text(&job.previous);
        }
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

/// Discard `files` (path, worktree status) in the repository at `repo`,
/// after a snapshot for Undo. Untracked folders (nested repositories) are
/// left alone and named; the rest is discarded.
fn discard_in(repo: &Path, files: Vec<(String, char)>) -> Result<GitOutcome, String> {
    let folders = git::untracked_folders(repo, &files);
    let files: Vec<(String, char)> = files.into_iter().filter(|(p, _)| !folders.contains(p)).collect();
    let left = match folders.as_slice() {
        [] => String::new(),
        [one] => format!("{one} is a folder (a nested repository?): remove it in a terminal"),
        many => format!("{} folders (nested repositories?) left: remove them in a terminal", many.len()),
    };
    if files.is_empty() {
        return Err(format!("nothing was discarded: {left}"));
    }
    // The snapshot is the Undo: without it, nothing is discarded.
    let tree =
        git::snapshot_tree(repo).map_err(|e| format!("couldn't back the files up, nothing was discarded: {e}"))?;
    // From here on, a failure still hands back the snapshot: some files may be discarded.
    let error = git::discard(repo, &files).err();
    let paths: Vec<String> = files.into_iter().map(|(p, _)| p).collect();
    let mut done = match paths.as_slice() {
        [one] => format!("Discarded {}", one.rsplit('/').next().unwrap_or(one)),
        many => format!("Discarded {} files", many.len()),
    };
    if !left.is_empty() {
        done = format!("{done} · {left}");
    }
    // Diff tabs close only when their changes are surely gone.
    Ok(GitOutcome::Discarded { tree, paths, done, close_diffs: error.is_none(), error })
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
    // A copy of the index puts it back exactly if the commit is rejected
    // (unstaging would also drop `git add -N` entries and anything staged meanwhile).
    let backup = if stage_all.is_empty() { None } else { IndexBackup::take(repo) };
    if !stage_all.is_empty() {
        git::stage(repo, &stage_all)?;
    }
    match git::commit(repo, message) {
        Ok(out) => {
            drop(backup);
            Ok(GitOutcome::Committed(first_line(&out).unwrap_or("Committed").to_string()))
        }
        Err(e) => {
            // A rejected Commit All (e.g. by a hook) leaves the index as it was:
            // left staged, the next Commit would commit only those, a different one.
            match backup {
                Some(b) => b.restore(),
                None if !stage_all.is_empty() => drop(git::unstage(repo, &stage_all)),
                None => {}
            }
            Err(e)
        }
    }
}

/// A copy of the repository's index, removed when dropped unless restored.
struct IndexBackup {
    index: PathBuf,
    copy: PathBuf,
}

impl IndexBackup {
    fn take(repo: &Path) -> Option<Self> {
        let out = git::git(repo, &["rev-parse", "--path-format=absolute", "--git-path", "index"]).ok()?;
        let index = PathBuf::from(out.trim());
        let copy = index.with_extension("mira-backup");
        std::fs::copy(&index, &copy).ok()?;
        Some(Self { index, copy })
    }

    /// Put the copy back in place (a rename: git never sees half an index).
    fn restore(self) {
        let _ = std::fs::rename(&self.copy, &self.index);
    }
}

impl Drop for IndexBackup {
    fn drop(&mut self) {
        // Gone already after `restore`.
        let _ = std::fs::remove_file(&self.copy);
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

    /// Exercise real update routing with isolated config/state in a child process.
    /// App::new spawns only /bin/true here, never the user's herdr client.
    #[test]
    fn activity_lifecycle_is_scoped_and_repeated_actions_are_blocked() {
        let Ok(root) = std::env::var("MIRA_ACTIVITY_TEST_ROOT") else {
            let root = crate::files::test_dir("activity");
            std::fs::create_dir_all(root.join("config/mira")).unwrap();
            std::fs::write(root.join("config/mira/config"), "terminal = shell\ntheme = LAGC Tech Dark\n").unwrap();
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "app::scm::tests::activity_lifecycle_is_scoped_and_repeated_actions_are_blocked",
                    "--nocapture",
                ])
                .env("MIRA_ACTIVITY_TEST_ROOT", &root)
                .env("HOME", &root)
                .env("XDG_CONFIG_HOME", root.join("config"))
                .env("XDG_STATE_HOME", root.join("state"))
                .env("SHELL", "/bin/true")
                .output()
                .unwrap();
            std::fs::remove_dir_all(root).unwrap();
            assert!(
                out.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            return;
        };
        let (mut app, task) = App::new(super::super::Start::Folder(root.into()), true);
        drop(task);
        let snapshot = git::Snapshot { root: Some(app.root.clone()), ..Default::default() };
        // A click during background polling attaches to that refresh. Late generations cannot stop it.
        let generation = app.git_gen;
        drop(app.update(Msg::RefreshChanges));
        drop(app.update(Msg::RefreshChanges));
        assert!(app.manual_refresh);
        assert_eq!(app.git_gen, generation);
        drop(app.update(Msg::GitLoaded(generation - 1, Ok(snapshot.clone()))));
        assert!(app.manual_refresh);
        drop(app.update(Msg::GitLoaded(generation, Ok(snapshot.clone()))));
        assert!(!app.manual_refresh);
        assert!(!app.refreshing);
        assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some("Changes refreshed"));
        // Errors stop the spinner too; automatic refreshes never start it.
        drop(app.refresh_git());
        assert!(!app.manual_refresh);
        drop(app.update(Msg::RefreshChanges));
        drop(app.update(Msg::GitLoaded(app.git_gen, Err("refresh failed".into()))));
        assert!(!app.manual_refresh);
        assert!(!app.refreshing);

        let stage = Msg::Stage(vec!["a.txt".into()]);
        drop(app.update(stage.clone()));
        assert!(app.git_busy);
        assert!(app.busy_op.as_ref().unwrap().matches(&stage));
        assert!(!app.busy_op.as_ref().unwrap().matches(&Msg::Stage(vec!["b.txt".into()])));
        drop(app.update(Msg::Push));
        assert!(app.busy_op.as_ref().unwrap().matches(&stage));
        drop(app.update(Msg::GitDone(app.root_epoch, Err("stage failed".into()))));
        let error = app.remote_dialog.as_ref().expect("foreground errors persist in a dialog").id;
        drop(app.update(Msg::RemoteAction(error, super::super::RemoteChoice::Close)));
        assert!(!app.git_busy);
        assert!(app.busy_op.is_none());
        drop(app.update(stage));
        drop(app.update(Msg::GitDone(app.root_epoch, Ok(GitOutcome::Info(String::new())))));
        assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some("Changes staged"));
        // Only the modal's explicit approval starts a discard.
        app.git.unstaged = vec![git::Change::new("a.txt", 'M')];
        drop(app.update(Msg::DiscardAll));
        assert!(app.confirmation.is_some());
        assert!(!app.git_busy);
        assert!(app.busy_op.is_none());
        let id = app.confirmation.as_ref().unwrap().id;
        drop(app.update(Msg::ConfirmProceed(id)));
        assert_eq!(app.busy_op, Some(BusyOp::DiscardAll));
        drop(app.update(Msg::GitDone(app.root_epoch, Ok(GitOutcome::Info("Discarded".into())))));
        assert!(app.busy_op.is_none());
        // An obsolete hunk never starts a spinner.
        drop(app.update(Msg::Hunk(HunkOp::Stage, 123, 0)));
        assert!(!app.git_busy);
        assert!(app.busy_op.is_none());

        // Replacement is serialized even when a new query finishes before the disk write.
        app.replacing = true;
        drop(app.update(Msg::ReplaceAll));
        assert!(app.replacing, "a repeated click did not start another write");
        let old_epoch = app.root_epoch;
        app.root_epoch += 1;
        app.search.query = "new folder query".into();
        drop(app.update(Msg::ReplaceDone(old_epoch, false, Ok(2))));
        assert!(!app.replacing);
        assert!(!app.search.running, "a previous folder's write did not refresh this folder's search");
        // A comment delivery from an old folder must not clear the new folder's draft.
        app.sending_comment = true;
        app.comment = "new draft".into();
        drop(app.update(Msg::SentToPane(old_epoch, Ok(()))));
        assert!(!app.sending_comment);
        assert_eq!(app.comment, "new draft");
    }

    #[test]
    fn hunk_activity_matches_only_its_action_and_diff_generation() {
        let op = BusyOp::from_message(&Msg::Hunk(HunkOp::Stage, 12, 3)).unwrap();
        assert!(op.matches(&Msg::Hunk(HunkOp::Stage, 12, 3)));
        assert!(!op.matches(&Msg::Hunk(HunkOp::Discard, 12, 3)));
        assert!(!op.matches(&Msg::Hunk(HunkOp::Stage, 13, 3)));
        assert!(!op.matches(&Msg::Hunk(HunkOp::Stage, 12, 4)));
    }

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
    fn discard_leaves_nested_repositories_and_does_the_rest() {
        let dir = repo("discard-nested");
        std::fs::create_dir_all(dir.join("sub/.git")).unwrap();
        // Only the nested repository: nothing happens, not even a snapshot.
        let err = discard_in(&dir, vec![("sub/".into(), 'U')]).err().unwrap_or_default();
        assert!(err.starts_with("nothing was discarded"), "{err}");
        std::fs::write(dir.join("a.txt"), "edited\n").unwrap();
        let out = discard_in(&dir, vec![("a.txt".into(), 'M'), ("sub/".into(), 'U')]);
        let Ok(GitOutcome::Discarded { done, error: None, close_diffs: true, paths, .. }) = out else {
            panic!("expected a clean discard")
        };
        assert!(done.contains("sub/ is a folder"), "{done}");
        assert_eq!(paths, ["a.txt"]);
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "a\n");
        assert!(dir.join("sub/.git").exists());
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
        // An intent-to-add entry survives a rejected Commit All too.
        git::git(&dir, &["add", "-N", "new.txt"]).unwrap();
        let before = git::git(&dir, &["status", "--porcelain"]).unwrap();
        let listed = vec![("a.txt".to_string(), false), ("new.txt".to_string(), false)];
        assert!(commit_in(&dir, "edit", "", listed).is_err());
        assert_eq!(git::git(&dir, &["status", "--porcelain"]).unwrap(), before);
        assert!(!dir.join(".git/index.mira-backup").exists(), "no copy left behind");
        git::git(&dir, &["rm", "--cached", "-q", "new.txt"]).unwrap();
        // A plain Commit leaves what you staged yourself alone.
        git::stage(&dir, &["a.txt".to_string()]).unwrap();
        assert!(commit_in(&dir, "edit", "", Vec::new()).is_err());
        assert_eq!(staged(&dir), "a.txt");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
