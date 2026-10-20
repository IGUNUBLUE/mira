//! Branch selector and guarded Undo Last Commit.
use super::{App, ConfirmAction, GitOutcome, Msg, Tab, blocking};
use crate::git::branches::{self, Listing};
use iced::Task;

pub(super) const INPUT: &str = "branch-search";
pub struct BranchPicker {
    pub id: u64,
    epoch: u64,
    pub listing: Listing,
    pub query: String,
    pub matches: Vec<usize>,
    pub selected: usize,
    pub capped: bool,
}
impl BranchPicker {
    fn search(&mut self) {
        let query = self.query.to_lowercase();
        self.matches = self
            .listing
            .branches
            .iter()
            .enumerate()
            .filter(|(_, b)| b.name.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .take(101)
            .collect();
        self.capped = self.matches.len() > 100;
        self.matches.truncate(100);
        self.selected = self.matches.iter().position(|&i| self.listing.branches[i].unavailable.is_none()).unwrap_or(0);
    }
}
impl App {
    fn branch_edits_saved(&mut self) -> bool {
        let root = self.git.root.as_ref().unwrap_or(&self.root);
        if self.tabs.iter().any(|tab| matches!(tab, Tab::File(file) if file.dirty && file.path.starts_with(root))) {
            drop(self.git_failure("Switch branch", "Save or close the unsaved editor tabs in this repository before switching branches. Your edits have not been changed.".into(), None, false));
            return false;
        }
        true
    }
    pub(super) fn update_branches(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::BranchesOpen if !self.git_busy => {
                if !self.branch_edits_saved() {
                    return iced::widget::operation::focus(super::remote::FOCUS);
                }
                let (root, epoch) = (self.root.clone(), self.root_epoch);
                self.git_busy = true;
                return Task::perform(blocking(move || branches::list(&root)), move |result| {
                    Msg::BranchesLoaded(epoch, result)
                });
            }
            Msg::BranchesLoaded(epoch, result) => {
                self.git_busy = false;
                self.busy_op = None;
                if epoch != self.root_epoch {
                    return Task::none();
                }
                match result {
                    Ok(mut listing) => {
                        listing.branches.sort_by(|a, b| {
                            let key =
                                |b: &branches::Branch| (b.unavailable.as_deref() != Some("Current branch"), b.remote);
                            key(a).cmp(&key(b)).then_with(|| a.name.cmp(&b.name))
                        });
                        self.branch_seq = self.branch_seq.wrapping_add(1);
                        let mut picker = BranchPicker {
                            id: self.branch_seq,
                            epoch,
                            listing,
                            query: String::new(),
                            matches: Vec::new(),
                            selected: 0,
                            capped: false,
                        };
                        picker.search();
                        self.branch_picker = Some(picker);
                        self.clear_status();
                        return iced::widget::operation::focus(INPUT);
                    }
                    Err(error) => return self.git_failure("List branches", error, None, false),
                }
            }
            Msg::BranchQuery(id, query) => {
                if let Some(picker) = &mut self.branch_picker
                    && picker.id == id
                    && picker.epoch == self.root_epoch
                {
                    picker.query = query;
                    picker.search();
                    return iced::widget::operation::snap_to(
                        "branch-list",
                        iced::widget::operation::RelativeOffset::START,
                    );
                }
            }
            Msg::BranchMove(step) => {
                if let Some(picker) = &mut self.branch_picker
                    && !picker.matches.is_empty()
                {
                    picker.selected =
                        (picker.selected as isize + step).rem_euclid(picker.matches.len() as isize) as usize;
                    let viewport = (self.window_height - 300.0).clamp(100.0, 400.0);
                    return iced::widget::operation::scroll_to(
                        "branch-list",
                        iced::widget::operation::AbsoluteOffset {
                            x: 0.0,
                            y: (picker.selected as f32 * 55.0 - (viewport - 52.0) / 2.0).max(0.0),
                        },
                    );
                }
            }
            Msg::BranchSubmit(id) => {
                if let Some(picker) = &self.branch_picker
                    && picker.id == id
                    && let Some(&index) = picker.matches.get(picker.selected)
                {
                    return self.update(Msg::BranchPick(id, picker.listing.branches[index].reference.clone()));
                }
            }
            Msg::BranchPick(id, reference) if !self.git_busy => {
                let Some(picker) = self.branch_picker.as_ref().filter(|p| p.id == id && p.epoch == self.root_epoch)
                else {
                    return Task::none();
                };
                let Some(target) = picker
                    .matches
                    .iter()
                    .map(|&i| &picker.listing.branches[i])
                    .find(|b| b.reference == reference && b.unavailable.is_none())
                    .cloned()
                else {
                    return Task::none();
                };
                if !self.branch_edits_saved() {
                    self.branch_picker = None;
                    return iced::widget::operation::focus(super::remote::FOCUS);
                }
                let Some(picker) = self.branch_picker.take() else {
                    return Task::none();
                };
                self.cancel_commit_generation_for_head_change();
                return self.git_task(move |_| branches::switch(&picker.listing, &target).map(GitOutcome::Switched));
            }
            Msg::BranchesClose => {
                self.branch_picker = None;
                return self.focus_terminal();
            }
            Msg::UndoLastCommit if !self.git_busy => {
                self.git_busy = true;
                let (root, epoch) = (self.root.clone(), self.root_epoch);
                return Task::perform(blocking(move || branches::prepare_undo(&root)), move |r| {
                    Msg::UndoCommitPrepared(epoch, r)
                });
            }
            Msg::UndoCommitPrepared(epoch, result) => {
                self.git_busy = false;
                self.busy_op = None;
                if epoch != self.root_epoch {
                    return Task::none();
                }
                match result {
                    Ok(plan) => return self.update(Msg::UndoCommitApply(plan)),
                    Err(error) => return self.git_failure("Undo Last Commit", error, None, false),
                }
            }
            Msg::UndoCommitApply(plan) if !self.git_busy => {
                let detail = format!(
                    "Remove the last commit from local branch '{}'. Its changes will remain staged alongside any existing staged edits. Working files are preserved. Remote refs are not fetched; if this commit was shared, use git revert instead.{}",
                    plan.branch,
                    if plan.merge { " This merge commit will return to its first parent." } else { "" }
                );
                if !self.confirm_action(
                    ConfirmAction::UndoCommit(plan.clone()),
                    "Undo Last Commit?",
                    detail,
                    vec![plan.summary.clone()],
                    "Undo commit",
                ) {
                    return Task::none();
                }
                self.cancel_commit_generation_for_head_change();
                return self.git_task(move |_| {
                    branches::undo(&plan)?;
                    Ok(GitOutcome::Undone {
                        message: plan.message,
                        summary: format!("Undid {} — changes remain staged", plan.summary),
                    })
                });
            }
            _ => {}
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn fixture(name: &str) -> Option<(App, PathBuf)> {
        if let Ok(root) = std::env::var("MIRA_BRANCH_TEST_ROOT") {
            let root = PathBuf::from(root);
            let (mut app, task) = App::new(super::super::Start::Folder(root.clone()), true);
            drop(task);
            let snapshot = crate::git::snapshot(&root, &mut crate::git::Cache::default(), false).unwrap();
            drop(app.update(Msg::GitLoaded(app.git_gen, Ok(snapshot))));
            return Some((app, root));
        }
        let root = crate::files::test_dir(name);
        std::fs::create_dir_all(root.join("config/mira")).unwrap();
        std::fs::write(root.join("config/mira/config"), "terminal = shell\nautosave = off\ntheme = LAGC Tech Light\n")
            .unwrap();
        let git = |args: &[&str]| crate::git::git(&root, args).unwrap();
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(root.join("a.txt"), "base\n").unwrap();
        git(&["add", "a.txt"]);
        git(&["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "base"]);
        git(&["branch", "feature/demo"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "last commit",
        ]);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("app::branches::tests::{name}"), "--nocapture"])
            .env("MIRA_BRANCH_TEST_ROOT", &root)
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
    fn branch_search_stale_clicks_and_unsaved_editors_are_guarded() {
        let Some((mut app, root)) = fixture("branch_search_stale_clicks_and_unsaved_editors_are_guarded") else {
            return;
        };
        let listing = branches::list(&root).unwrap();
        drop(app.update(Msg::BranchesLoaded(app.root_epoch, Ok(listing.clone()))));
        let first = app.branch_picker.as_ref().unwrap().id;
        capture(&app, "branch-selector");
        drop(app.update(Msg::BranchQuery(first, "DEMO".into())));
        let picker = app.branch_picker.as_ref().unwrap();
        assert_eq!(picker.matches.len(), 1);
        assert_eq!(picker.listing.branches[picker.matches[0]].name, "feature/demo");
        drop(app.update(Msg::BranchesClose));
        drop(app.update(Msg::BranchesLoaded(app.root_epoch, Ok(listing))));
        let second = app.branch_picker.as_ref().unwrap().id;
        drop(app.update(Msg::BranchPick(first, "refs/heads/feature/demo".into())));
        assert!(!app.git_busy);
        assert_eq!(app.branch_picker.as_ref().unwrap().id, second);
        drop(app.update(Msg::BranchesClose));
        capture(&app, "changes-controls");
        app.open_file(root.join("a.txt"), None);
        let Some(Tab::File(file)) = app.tabs.first_mut() else { panic!("file") };
        file.dirty = true;
        drop(app.update(Msg::BranchesOpen));
        assert!(!app.git_busy);
        assert!(app.remote_dialog.as_ref().unwrap().detail.contains("unsaved editor"));
        assert_eq!(crate::git::git(&root, &["branch", "--show-current"]).unwrap().trim(), "main");
    }
    #[test]
    fn undo_needs_one_shot_confirmation_and_restores_message_without_overwriting_a_draft() {
        let Some((mut app, root)) =
            fixture("undo_needs_one_shot_confirmation_and_restores_message_without_overwriting_a_draft")
        else {
            return;
        };
        let plan = branches::prepare_undo(&root).unwrap();
        drop(app.update(Msg::UndoCommitPrepared(app.root_epoch, Ok(plan.clone()))));
        let first = app.confirmation.as_ref().unwrap().id;
        assert!(!app.git_busy);
        capture(&app, "undo-confirmation");
        drop(app.confirmation_key(iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter), false));
        assert!(app.confirmation.is_none());
        drop(app.update(Msg::UndoCommitPrepared(app.root_epoch, Ok(plan.clone()))));
        let second = app.confirmation.as_ref().unwrap().id;
        drop(app.update(Msg::ConfirmProceed(first)));
        assert!(!app.git_busy);
        drop(app.update(Msg::ConfirmProceed(second)));
        assert!(app.git_busy);
        assert_eq!(app.busy_op, Some(super::super::BusyOp::UndoCommit));
        drop(app.update(Msg::GitDone(
            app.root_epoch,
            Ok(GitOutcome::Undone { message: plan.message.clone(), summary: "Undid commit".into() }),
        )));
        assert_eq!(app.commit_msg.text(), "last commit");
        app.commit_msg = iced::widget::text_editor::Content::with_text("existing draft");
        drop(app.update(Msg::GitDone(
            app.root_epoch,
            Ok(GitOutcome::Undone { message: "another message".into(), summary: "Undid commit".into() }),
        )));
        assert_eq!(app.commit_msg.text(), "existing draft");
        app.root_epoch += 1;
        drop(app.update(Msg::UndoCommitPrepared(app.root_epoch - 1, Ok(plan))));
        assert!(app.confirmation.is_none());
    }
    fn capture(app: &App, name: &str) {
        let Ok(dir) = std::env::var("MIRA_UI_CAPTURE_DIR") else {
            return;
        };
        use iced::advanced::{
            Layout, layout,
            renderer::{self, Headless},
            widget::Tree,
        };
        use std::future::Future as _;
        let mut new = std::pin::pin!(<iced::Renderer as Headless>::new(
            iced::Font::MONOSPACE,
            iced::Pixels(14.0),
            Some("tiny-skia")
        ));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(Some(mut renderer)) = new.as_mut().poll(&mut cx) else { panic!("renderer") };
        let size = iced::Size::new(900.0, 760.0);
        let mut view = app.view();
        let mut tree = Tree::new(&view);
        let node = view.as_widget_mut().layout(&mut tree, &renderer, &layout::Limits::new(iced::Size::ZERO, size));
        view.as_widget().draw(
            &tree,
            &mut renderer,
            &iced::Theme::Light,
            &renderer::Style::default(),
            Layout::new(&node),
            iced::mouse::Cursor::Unavailable,
            &iced::Rectangle::new(iced::Point::ORIGIN, size),
        );
        let pixels = renderer.screenshot(iced::Size::new(900, 760), 1.0, crate::theme::p().sidebar);
        std::fs::create_dir_all(&dir).unwrap();
        image::save_buffer(PathBuf::from(dir).join(format!("{name}.png")), &pixels, 900, 760, image::ColorType::Rgba8)
            .unwrap();
    }
}
