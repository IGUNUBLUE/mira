//! Explicit, one-shot approval for actions that can lose work.

use super::{App, HunkOp, Msg, Tab};
use crate::runs::RunId;
use iced::{Task, keyboard};
use std::path::PathBuf;
use std::time::SystemTime;

pub(super) const CONFIRM_FOCUS: &str = "destructive-confirmation";

/// Stable targets: a changed scope requires a new confirmation.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ConfirmAction {
    Discard(Vec<crate::git::Change>),
    DiscardAll(Vec<crate::git::Change>),
    Hunk(u64, usize),
    RevertRun(RunId, Vec<String>, String),
    UndoDiscard(String, Vec<String>),
    Trash(PathBuf, bool),
    Replace(u64, String),
    CloseTab(PathBuf),
    Overwrite(PathBuf, Option<SystemTime>),
    Quit(Vec<PathBuf>),
    SignOut,
    UndoCommit(crate::git::branches::UndoCommit),
}

impl ConfirmAction {
    fn message(&self, app: &App) -> Option<Msg> {
        Some(match self {
            Self::Discard(changes) => Msg::Discard(changes.iter().map(|c| c.path.clone()).collect()),
            Self::DiscardAll(_) => Msg::DiscardAll,
            Self::Hunk(id, i) => {
                let diff = app.active_diff()?.diff.as_ref()?;
                if diff.id != *id || diff.patch.as_ref()?.hunks.get(*i).is_none() {
                    return None;
                }
                Msg::Hunk(HunkOp::Discard, *id, *i)
            }
            Self::RevertRun(id, ..) => Msg::RevertRun(*id),
            Self::UndoDiscard(..) => Msg::UndoDiscard,
            Self::Trash(path, dir) => Msg::TrashPath(path.clone(), *dir),
            Self::Replace(..) => Msg::ReplaceAll,
            Self::CloseTab(path) => {
                Msg::CloseTab(app.tabs.iter().position(|t| matches!(t, Tab::File(f) if &f.path == path))?)
            }
            Self::Overwrite(path, _) => Msg::SaveFile(path.clone()),
            Self::Quit(_) => Msg::CloseRequested,
            Self::SignOut => Msg::ChatGptSignOut,
            Self::UndoCommit(plan) => Msg::UndoCommitApply(plan.clone()),
        })
    }
}

pub struct Confirmation {
    pub id: u64,
    epoch: u64,
    action: ConfirmAction,
    pub title: &'static str,
    pub detail: String,
    pub targets: Vec<String>,
    pub remaining: Option<String>,
    pub accept: &'static str,
    /// Keyboard navigation starts on Cancel; Enter alone never accepts by default.
    pub accept_selected: bool,
}

/// Keep background results flowing, but do not accept input aimed at the obscured UI.
pub(super) fn allowed_during_confirmation(msg: &Msg) -> bool {
    if let Msg::Update(event) = msg {
        return event.background();
    }
    matches!(
        msg,
        Msg::ConfirmCancel
            | Msg::ConfirmProceed(_)
            | Msg::Key(..)
            | Msg::Tick
            | Msg::StatusTick
            | Msg::Spin
            | Msg::Modifiers(_)
            | Msg::WindowFocus(_)
            | Msg::WindowSize(_)
            | Msg::EditorViewport(_)
            | Msg::Term(_)
            | Msg::TermAction(crate::term::Action::Resize { .. })
            | Msg::ReportColorScheme
            | Msg::HerdrPolled(_)
            | Msg::ForRoot(..)
            | Msg::WorkspaceReady(_)
            | Msg::RunSnapshot(..)
            | Msg::RunFiles(..)
            | Msg::RunPrompt(..)
            | Msg::SentToPane(..)
            | Msg::AgentFocused(_)
            | Msg::DiffLoaded(..)
            | Msg::HerdrVersion(_)
            | Msg::TrashDone(_)
            | Msg::GitLoaded(..)
            | Msg::CodeAnalyze(..)
            | Msg::CodeAnalyzed(..)
            | Msg::GitDone(..)
            | Msg::BranchesLoaded(..)
            | Msg::UndoCommitPrepared(..)
            | Msg::RemotePrepared(..)
            | Msg::RemoteDone(..)
            | Msg::CommitGen(..)
            | Msg::SignInEvent(..)
            | Msg::SearchDebounced(_)
            | Msg::SearchDone(..)
            | Msg::ReplaceDone(..)
            | Msg::PaletteFiles(..)
    )
}

impl App {
    /// Asking only changes UI state. The actual action must call this again after approval.
    pub(super) fn confirm_action(
        &mut self,
        action: ConfirmAction,
        title: &'static str,
        detail: impl Into<String>,
        mut targets: Vec<String>,
        accept: &'static str,
    ) -> bool {
        if self.approval.as_ref() == Some(&action) {
            return true;
        }
        if self.confirmation.is_some() {
            return false;
        }
        let remaining = (targets.len() > 12).then(|| format!("… and {} more files", targets.len() - 12));
        targets.truncate(12);
        self.confirm_seq += 1;
        self.confirmation = Some(Confirmation {
            id: self.confirm_seq,
            epoch: self.root_epoch,
            action,
            title,
            detail: detail.into(),
            targets,
            remaining,
            accept,
            accept_selected: false,
        });
        false
    }

    pub(super) fn update_confirmation(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::ConfirmCancel => {
                let Some(pending) = self.confirmation.take() else { return Task::none() };
                return self.confirmation_focus(&pending.action);
            }
            Msg::ConfirmProceed(id) => {
                let Some(pending) = self.confirmation.take() else { return Task::none() };
                if pending.id != id {
                    self.confirmation = Some(pending);
                    return Task::none();
                }
                if pending.epoch != self.root_epoch {
                    self.set_status("The folder changed. Review the action again.", true);
                    return Task::none();
                }
                let Some(action) = pending.action.message(self) else {
                    self.set_status("The target changed or closed. Review the action again.", true);
                    return Task::none();
                };
                self.approval = Some(pending.action.clone());
                let task = self.update(action);
                // Approval never escapes this synchronous dispatch into a worker or a later click.
                self.approval = None;
                return if self.confirmation.is_some() {
                    task
                } else {
                    Task::batch([task, self.confirmation_focus(&pending.action)])
                };
            }
            _ => {}
        }
        Task::none()
    }

    pub(super) fn confirmation_key(&mut self, key: keyboard::Key, repeat: bool) -> Task<Msg> {
        use keyboard::key::Named;
        let Some(pending) = &mut self.confirmation else { return Task::none() };
        match key {
            keyboard::Key::Named(Named::Escape) => self.update(Msg::ConfirmCancel),
            keyboard::Key::Named(Named::Tab) => {
                pending.accept_selected = !pending.accept_selected;
                Task::none()
            }
            keyboard::Key::Named(Named::Enter) if !repeat => {
                let msg = if pending.accept_selected { Msg::ConfirmProceed(pending.id) } else { Msg::ConfirmCancel };
                self.update(msg)
            }
            _ => Task::none(),
        }
    }

    fn confirmation_focus(&mut self, action: &ConfirmAction) -> Task<Msg> {
        match action {
            ConfirmAction::Replace(..) => iced::widget::operation::focus(super::SEARCH_INPUT),
            ConfirmAction::Overwrite(..) | ConfirmAction::CloseTab(_) | ConfirmAction::Quit(_)
                if matches!(self.active_tab.and_then(|i| self.tabs.get(i)), Some(Tab::File(_))) =>
            {
                iced::widget::operation::focus(super::EDITOR)
            }
            _ => self.focus_terminal(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{Change, Snapshot};
    use iced::keyboard::{Key, key::Named};
    use std::sync::Arc;

    #[test]
    fn workspace_tabs_restore_their_own_files_cursor_and_unsaved_edits() {
        let Some((mut app, root)) = fixture("workspace_tabs_restore_their_own_files_cursor_and_unsaved_edits") else {
            return;
        };
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("b.txt"), "other workspace\n").unwrap();
        drop(app.update(Msg::OpenFile(root.join("a.txt"), None)));
        drop(
            app.update(Msg::Edit(iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Insert(
                'x',
            )))),
        );
        let before = app.active_file().unwrap().content.cursor();
        drop(app.switch_root(other.clone()));
        assert!(app.tabs.is_empty(), "old workspace files must not remain visible");
        drop(app.update(Msg::OpenFile(other.join("b.txt"), None)));
        assert_eq!(app.tabs.len(), 1);
        drop(app.switch_root(root.clone()));
        assert_eq!(app.tabs.len(), 1);
        let file = app.active_file().unwrap();
        assert_eq!(file.path, root.join("a.txt"));
        assert_eq!(file.content.cursor(), before);
        assert!(file.dirty);
        assert!(file.content.text().starts_with('x'));
        drop(app.switch_root(other));
        assert_eq!(app.active_file().unwrap().path.file_name().unwrap(), "b.txt");
        drop(app.update(Msg::CloseRequested));
        assert!(
            matches!(app.confirmation.as_ref().map(|c| &c.action), Some(ConfirmAction::Quit(paths)) if paths == &vec![root.join("a.txt")])
        );
    }

    #[test]
    fn autosave_on_quit_includes_files_parked_in_other_workspaces() {
        let Some((mut app, root)) = fixture("autosave_on_quit_includes_files_parked_in_other_workspaces") else {
            return;
        };
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        drop(app.update(Msg::OpenFile(root.join("a.txt"), None)));
        drop(
            app.update(Msg::Edit(iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Insert(
                'x',
            )))),
        );
        drop(app.switch_root(other));
        app.autosave = super::super::AutoSave::OnWindowChange;
        drop(app.update(Msg::CloseRequested));
        assert!(app.confirmation.is_none());
        assert!(std::fs::read_to_string(root.join("a.txt")).unwrap().starts_with('x'));
        assert!(app.all_document_tabs().all(|t| !matches!(t, Tab::File(f) if f.dirty)));
    }

    /// App::new reads/writes configuration and spawns a terminal. Isolate every test in
    /// its own process and home, with /bin/true as the shell, to leave live sessions alone.
    fn fixture(name: &str) -> Option<(App, PathBuf)> {
        if let Ok(root) = std::env::var("MIRA_CONFIRM_TEST_ROOT") {
            let root = PathBuf::from(root);
            let (mut app, task) = App::new(super::super::Start::Folder(root.clone()), true);
            drop(task);
            app.autosave = super::super::AutoSave::Off;
            let snap =
                Snapshot { root: Some(root.clone()), unstaged: vec![Change::new("a.txt", 'M')], ..Default::default() };
            drop(app.update(Msg::GitLoaded(app.git_gen, Ok(snap))));
            return Some((app, root));
        }
        let root = crate::files::test_dir(name);
        std::fs::create_dir_all(root.join("config/mira")).unwrap();
        std::fs::write(root.join("config/mira/config"), "terminal = shell\nautosave = off\ntheme = LAGC Tech Dark\n")
            .unwrap();
        std::fs::write(root.join("a.txt"), "disk contents\n").unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("app::confirm::tests::{name}"), "--nocapture"])
            .env("MIRA_CONFIRM_TEST_ROOT", &root)
            .env("HOME", &root)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("SHELL", "/bin/true")
            .output()
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        None
    }

    #[test]
    fn updating_preserves_parked_edits_and_commit_drafts() {
        let Some((mut app, root)) = fixture("updating_preserves_parked_edits_and_commit_drafts") else {
            return;
        };
        drop(app.update(Msg::OpenFile(root.join("a.txt"), None)));
        drop(
            app.update(Msg::Edit(iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Insert(
                'x',
            )))),
        );
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        drop(app.switch_root(other));
        drop(app.update(Msg::Update(super::super::UpdateEvent::Install)));
        assert!(app.updates.error.as_deref().unwrap().contains("other workspaces"));
        assert_eq!(app.updates.phase, super::super::UpdatePhase::Idle);
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "disk contents\n");
        drop(app.switch_root(root));
        app.tabs.clear();
        app.commit_msg = iced::widget::text_editor::Content::with_text("draft commit");
        drop(app.update(Msg::Update(super::super::UpdateEvent::Install)));
        assert!(app.updates.error.as_deref().unwrap().contains("draft commit"));
        assert_eq!(app.commit_msg.text(), "draft commit");
    }

    #[test]
    fn update_cancellation_ignores_late_results_and_automatic_checks_respect_schedule() {
        let Some((mut app, _)) =
            fixture("update_cancellation_ignores_late_results_and_automatic_checks_respect_schedule")
        else {
            return;
        };
        app.updates.loaded = true;
        app.updates.automatic = false;
        drop(app.maybe_check_updates());
        assert_eq!(app.updates.phase, super::super::UpdatePhase::Idle);
        app.updates.automatic = true;
        app.updates.cache.checked_at = crate::persist::now();
        drop(app.maybe_check_updates());
        assert_eq!(app.updates.phase, super::super::UpdatePhase::Idle);
        drop(app.update(Msg::Update(super::super::UpdateEvent::Check)));
        assert_eq!(app.updates.phase, super::super::UpdatePhase::Checking);
        drop(app.update(Msg::Update(super::super::UpdateEvent::Cancel)));
        drop(app.update(Msg::Update(super::super::UpdateEvent::Checked(1, Err("stale response".into())))));
        assert!(app.updates.error.is_none());
        assert_eq!(app.updates.phase, super::super::UpdatePhase::Idle);
        drop(app.maybe_check_updates());
        assert_eq!(app.updates.phase, super::super::UpdatePhase::Idle);
    }

    fn pending_id(app: &App) -> u64 {
        app.confirmation.as_ref().unwrap().id
    }

    #[test]
    fn discard_requires_explicit_one_shot_approval_and_cancel_keeps_contents() {
        let Some((mut app, root)) = fixture("discard_requires_explicit_one_shot_approval_and_cancel_keeps_contents")
        else {
            return;
        };
        let discard = Msg::Discard(vec!["a.txt".into()]);
        drop(app.update(discard.clone()));
        let first = pending_id(&app);
        assert!(!app.git_busy);
        assert_eq!(app.confirmation.as_ref().unwrap().targets, ["a.txt"]);
        assert!(!app.confirmation.as_ref().unwrap().accept_selected);
        // Neither a second click nor shortcuts aimed at the obscured UI can approve it.
        drop(app.update(discard.clone()));
        drop(app.update(Msg::Stage(vec!["a.txt".into()])));
        drop(app.update(Msg::CloseRequested));
        assert_eq!(pending_id(&app), first);
        assert!(!app.git_busy);
        app.clear_status();
        assert_eq!(pending_id(&app), first, "a dialog never expires with the status bar");
        drop(app.confirmation_key(Key::Named(Named::Enter), false));
        assert!(app.confirmation.is_none(), "Enter initially activates Cancel");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "disk contents\n");
        drop(app.update(discard.clone()));
        let second = pending_id(&app);
        drop(app.update(Msg::ConfirmProceed(first)));
        assert_eq!(pending_id(&app), second, "an old button cannot approve a new dialog");
        drop(app.update(Msg::ConfirmProceed(second)));
        assert!(app.git_busy);
        assert!(app.confirmation.is_none());
        assert!(app.approval.is_none());
        drop(app.update(Msg::GitDone(app.root_epoch, Err("fixture worker not run".into()))));
        let error = app.remote_dialog.as_ref().unwrap().id;
        drop(app.update(Msg::RemoteAction(error, super::super::RemoteChoice::Close)));
        drop(app.update(discard));
        assert!(app.confirmation.is_some(), "approval cannot be reused for the next discard");
        drop(app.confirmation_key(Key::Named(Named::Escape), false));
        assert!(app.confirmation.is_none());
    }

    #[test]
    fn changed_scope_and_folder_invalidate_an_approval() {
        let Some((mut app, _)) = fixture("changed_scope_and_folder_invalidate_an_approval") else { return };
        drop(app.update(Msg::DiscardAll));
        let first = pending_id(&app);
        let mut snapshot = app.git.clone();
        snapshot.unstaged.push(Change::new("b.txt", 'U'));
        drop(app.update(Msg::GitLoaded(app.git_gen, Ok(snapshot))));
        drop(app.update(Msg::ConfirmProceed(first)));
        let second = pending_id(&app);
        assert_ne!(first, second);
        assert!(!app.git_busy);
        assert_eq!(app.confirmation.as_ref().unwrap().targets, ["a.txt", "b.txt"]);
        assert!(app.confirmation.as_ref().unwrap().detail.contains("1 new file"));
        app.git.unstaged[0].code = 'U';
        drop(app.update(Msg::ConfirmProceed(second)));
        let third = pending_id(&app);
        assert_ne!(second, third, "a changed deletion warning requires fresh approval");
        assert!(!app.git_busy);
        app.root_epoch += 1;
        drop(app.update(Msg::ConfirmProceed(third)));
        assert!(app.confirmation.is_none());
        assert!(!app.git_busy);
        assert!(app.approval.is_none());
    }

    #[test]
    fn close_and_overwrite_preserve_unsaved_edits_until_approved() {
        let Some((mut app, root)) = fixture("close_and_overwrite_preserve_unsaved_edits_until_approved") else {
            return;
        };
        let path = root.join("a.txt");
        app.open_file(path.clone(), None);
        let Tab::File(file) = &mut app.tabs[0] else { panic!("file tab") };
        file.content = iced::widget::text_editor::Content::with_text("editor draft\n");
        file.dirty = true;
        drop(app.update(Msg::CloseTab(0)));
        assert_eq!(app.tabs.len(), 1);
        drop(app.update(Msg::ConfirmCancel));
        drop(app.update(Msg::CloseRequested));
        let id = pending_id(&app);
        drop(app.update(Msg::CloseRequested));
        assert_eq!(pending_id(&app), id);
        drop(app.update(Msg::ConfirmCancel));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "disk contents\n");
        std::fs::write(&path, "agent contents\n").unwrap();
        let Tab::File(file) = &mut app.tabs[0] else { panic!("file tab") };
        file.conflict = true;
        drop(app.update(Msg::Save));
        assert_eq!(app.confirmation.as_ref().unwrap().accept, "Overwrite file");
        drop(app.update(Msg::ConfirmCancel));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "agent contents\n");
        drop(app.update(Msg::Save));
        let first = pending_id(&app);
        // A later disk change must be warned about again, even for the same file.
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1234)),
            )
            .unwrap();
        drop(app.update(Msg::ConfirmProceed(first)));
        let second = pending_id(&app);
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "agent contents\n");
        drop(app.update(Msg::ConfirmProceed(second)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "editor draft\n");
        let Tab::File(file) = &mut app.tabs[0] else { panic!("file tab") };
        file.content = iced::widget::text_editor::Content::with_text("unsaved again\n");
        file.dirty = true;
        drop(app.update(Msg::CloseTab(0)));
        let id = pending_id(&app);
        drop(app.update(Msg::ConfirmProceed(id)));
        assert!(app.tabs.is_empty());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "editor draft\n");
    }

    #[test]
    fn trash_replace_and_undo_warn_before_they_start() {
        let Some((mut app, root)) = fixture("trash_replace_and_undo_warn_before_they_start") else { return };
        let path = root.join("a.txt");
        drop(app.update(Msg::TrashPath(path.clone(), false)));
        assert_eq!(app.confirmation.as_ref().unwrap().accept, "Move to trash");
        drop(app.update(Msg::ConfirmCancel));
        assert!(path.exists());
        let folder = root.join("folder");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("nested.txt"), "keep").unwrap();
        drop(app.update(Msg::TrashPath(folder.clone(), true)));
        assert!(app.confirmation.as_ref().unwrap().detail.contains("everything inside"));
        drop(app.update(Msg::ConfirmCancel));
        assert!(folder.join("nested.txt").exists());
        app.search.query = "contents".into();
        app.search.replace = "replacement".into();
        app.search.total = 1;
        app.search.results =
            vec![crate::search::FileMatches { path: path.clone(), rel: "a.txt".into(), lines: vec![], count: 1 }];
        drop(app.update(Msg::ReplaceAll));
        assert!(!app.replacing);
        drop(app.update(Msg::ConfirmCancel));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "disk contents\n");
        app.undo_discard = Some(("snapshot".into(), vec!["a.txt".into()]));
        drop(app.update(Msg::UndoDiscard));
        assert!(app.undo_discard.is_some(), "cancel must not consume the backup");
        assert!(app.confirmation.as_ref().unwrap().detail.contains("after the discard"));
        drop(app.update(Msg::ConfirmCancel));
        assert!(app.undo_discard.is_some());
        drop(app.update(Msg::ChatGptSignOut));
        assert_eq!(app.confirmation.as_ref().unwrap().accept, "Sign out");
        drop(app.update(Msg::ConfirmCancel));
    }

    #[test]
    fn hunk_confirmation_cannot_discard_a_reloaded_diff() {
        let Some((mut app, _)) = fixture("hunk_confirmation_cannot_discard_a_reloaded_diff") else { return };
        let mut diff = crate::diff::compute("old\n", "new\n", Some(3));
        diff.patch = crate::hunks::parse(
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old\n+new\n",
            false,
        );
        let id = diff.id;
        app.tabs.push(Tab::Diff(super::super::DiffTab {
            rel: "a.txt".into(),
            source: super::super::DiffSource::Index { staged: false },
            full: false,
            diff: Some(Arc::new(diff)),
            anchor: None,
            loaded_mtime: None,
            on_disk: true,
            load: 0,
        }));
        app.active_tab = Some(0);
        drop(app.update(Msg::Hunk(HunkOp::Discard, id, 0)));
        assert!(!app.git_busy);
        let pending = pending_id(&app);
        let Tab::Diff(tab) = &mut app.tabs[0] else { panic!("diff tab") };
        tab.diff = Some(Arc::new(crate::diff::compute("old\n", "newer\n", Some(3))));
        drop(app.update(Msg::ConfirmProceed(pending)));
        assert!(!app.git_busy);
        assert!(app.confirmation.is_none());
        assert!(app.status.as_ref().unwrap().0.contains("changed"));
    }
}
