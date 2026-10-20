//! Publication choices and persistent, actionable foreground Git errors.
use super::{App, BusyOp, Msg, blocking};
use crate::git::remote::{self, Operation, Plan};
use iced::{Task, keyboard};

pub(super) const FOCUS: &str = "git-operation-dialog";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteChoice {
    Close,
    Publish,
    CopyDetails,
    Terminal,
    Retry(bool),
}

pub struct RemoteDialog {
    pub id: u64,
    epoch: u64,
    pub title: String,
    pub context: String,
    pub summary: String,
    pub next: String,
    pub detail: String,
    pub remotes: Vec<String>,
    pub selected_remote: Option<String>,
    pub choices: Vec<(&'static str, RemoteChoice)>,
    pub selected: usize,
    plan: Option<Plan>,
    ipv4: bool,
}

impl App {
    pub(super) fn start_remote(&mut self, operation: Operation) -> Task<Msg> {
        if self.git_busy {
            return Task::none();
        }
        self.git_busy = true;
        let (root, epoch) = (self.root.clone(), self.root_epoch);
        Task::perform(blocking(move || remote::prepare(&root, operation)), move |r| {
            Msg::RemotePrepared(epoch, operation, r)
        })
    }

    fn run_remote(&mut self, plan: Plan, ipv4: bool) -> Task<Msg> {
        if self.git_busy {
            return Task::none();
        }
        self.git_busy = true;
        self.busy_op = Some(match plan.operation {
            Operation::Push => BusyOp::Push,
            Operation::Pull => BusyOp::Pull,
        });
        let label = if plan.publishing() {
            "Publishing"
        } else {
            match plan.operation {
                Operation::Push => "Pushing",
                Operation::Pull => "Pulling",
            }
        };
        self.set_status(format!("{label} {}{}…", plan.branch, if ipv4 { " via IPv4" } else { "" }), false);
        let epoch = self.root_epoch;
        Task::perform(
            blocking(move || {
                let result = remote::execute(&plan, ipv4);
                (plan, result)
            }),
            move |(p, r)| Msg::RemoteDone(epoch, p, ipv4, r),
        )
    }

    /// Errors survive status expiry. Their raw output is readable and copyable.
    pub(super) fn git_failure(&mut self, operation: &str, detail: String, plan: Option<Plan>, ipv4: bool) -> Task<Msg> {
        let advice = remote::advice(&detail);
        let context = plan
            .as_ref()
            .map_or_else(|| format!("Repository: {}\nBranch: {}", self.root.display(), self.git.branch), Plan::context);
        let summary = format!("{operation} failed: {}", advice.summary);
        self.set_error_detail(summary.clone(), detail.clone());
        self.remote_seq = self.remote_seq.wrapping_add(1);
        let mut choices = vec![("Close", RemoteChoice::Close), ("Copy details", RemoteChoice::CopyDetails)];
        if plan.is_some() {
            choices.push(("Continue in terminal", RemoteChoice::Terminal));
            choices.push(("Retry", RemoteChoice::Retry(ipv4)));
            if advice.ipv4 && !ipv4 {
                choices.push(("Retry via IPv4", RemoteChoice::Retry(true)));
            }
        }
        self.remote_dialog = Some(RemoteDialog {
            id: self.remote_seq,
            epoch: self.root_epoch,
            title: format!("{operation} failed"),
            context,
            summary,
            next: advice.next.into(),
            detail,
            remotes: Vec::new(),
            selected_remote: None,
            choices,
            selected: 0,
            plan,
            ipv4,
        });
        iced::widget::operation::focus(FOCUS)
    }

    pub(super) fn update_remote(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::RemotePrepared(epoch, operation, result) => {
                self.git_busy = false;
                self.busy_op = None;
                if epoch != self.root_epoch {
                    if let Err(e) = result {
                        self.set_error_detail(format!("Previous folder: {} preparation failed", operation.name()), e);
                    }
                    return Task::none();
                }
                match result {
                    Ok(plan) if plan.publishing() => {
                        self.clear_status();
                        self.remote_seq = self.remote_seq.wrapping_add(1);
                        self.remote_dialog = Some(RemoteDialog {
                            id: self.remote_seq, epoch, title: "Publish branch".into(), context: plan.context(),
                            summary: format!("Publish '{}' to a remote?", plan.branch),
                            next: "Choose the intended remote. Git will publish this branch and set its tracking branch only if the push succeeds.".into(),
                            detail: String::new(), remotes: plan.remotes.clone(), selected_remote: plan.preferred.clone(),
                            choices: vec![("Cancel", RemoteChoice::Close), ("Publish branch", RemoteChoice::Publish)],
                            selected: 0, plan: Some(plan), ipv4: false,
                        });
                        return iced::widget::operation::focus(FOCUS);
                    }
                    Ok(plan) => return self.run_remote(plan, false),
                    Err(e) => return self.git_failure(operation.name(), e, None, false),
                }
            }
            Msg::RemoteDone(epoch, plan, ipv4, result) => {
                self.git_busy = false;
                self.busy_op = None;
                if epoch != self.root_epoch {
                    match result {
                        Ok(summary) => {
                            self.set_status(format!("Previous folder ({}): {summary}", plan.root.display()), false)
                        }
                        Err(e) => {
                            let task = self.git_failure(
                                &format!("Previous folder {}", plan.operation.name()),
                                e,
                                Some(plan),
                                ipv4,
                            );
                            if let Some(dialog) = &mut self.remote_dialog {
                                dialog.epoch = epoch;
                            }
                            return task;
                        }
                    }
                } else {
                    match result {
                        Ok(summary) => self.set_status(summary, false),
                        Err(e) => {
                            let name = plan.operation.name();
                            return self.git_failure(name, e, Some(plan), ipv4);
                        }
                    }
                }
                return self.refresh_git();
            }
            Msg::RemoteSelect(id, remote) => {
                if let Some(dialog) = &mut self.remote_dialog
                    && dialog.id == id
                    && dialog.epoch == self.root_epoch
                    && dialog.remotes.contains(&remote)
                {
                    dialog.selected_remote = Some(remote);
                }
            }
            Msg::RemoteAction(id, choice) => {
                let Some(dialog) = self.remote_dialog.as_ref().filter(|d| d.id == id) else {
                    return Task::none();
                };
                if !dialog.choices.iter().any(|(_, c)| c == &choice) {
                    return Task::none();
                }
                if choice == RemoteChoice::CopyDetails {
                    return iced::clipboard::write(format!(
                        "{}\n{}\n{}\n\n{}",
                        dialog.summary, dialog.context, dialog.next, dialog.detail
                    ));
                }
                if choice == RemoteChoice::Publish && dialog.selected_remote.is_none() {
                    return Task::none();
                }
                let Some(mut dialog) = self.remote_dialog.take() else {
                    return Task::none();
                };
                if choice == RemoteChoice::Close {
                    return self.focus_terminal();
                }
                // A late failure can be read/copied after a folder switch, but never retried there.
                if dialog.epoch != self.root_epoch {
                    self.set_status("Folder changed — start the Git operation again in its own workspace", true);
                    return self.focus_terminal();
                }
                let Some(mut plan) = dialog.plan.take() else {
                    return Task::none();
                };
                match choice {
                    RemoteChoice::Publish => {
                        plan.publish_to = dialog.selected_remote;
                        return self.run_remote(plan, false);
                    }
                    RemoteChoice::Retry(ipv4) => return self.run_remote(plan, ipv4),
                    RemoteChoice::Terminal => {
                        self.set_status("Git command copied — open a shell, review it and run it to continue", false);
                        return Task::batch([iced::clipboard::write(plan.command(dialog.ipv4)), self.focus_terminal()]);
                    }
                    RemoteChoice::Close | RemoteChoice::CopyDetails => {}
                }
            }
            _ => {}
        }
        Task::none()
    }

    pub(super) fn remote_key(&mut self, key: keyboard::Key, repeat: bool, shift: bool) -> Task<Msg> {
        use keyboard::key::Named;
        let Some(dialog) = &mut self.remote_dialog else {
            return Task::none();
        };
        match key {
            keyboard::Key::Named(Named::Escape) => {
                let id = dialog.id;
                return self.update(Msg::RemoteAction(id, RemoteChoice::Close));
            }
            keyboard::Key::Named(Named::Tab) => {
                let len = dialog.choices.len();
                dialog.selected = if shift { (dialog.selected + len - 1) % len } else { (dialog.selected + 1) % len };
            }
            keyboard::Key::Named(Named::Enter) if !repeat => {
                let id = dialog.id;
                let choice = dialog.choices[dialog.selected].1;
                return self.update(Msg::RemoteAction(id, choice));
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
        if let Ok(root) = std::env::var("MIRA_REMOTE_TEST_ROOT") {
            let root = PathBuf::from(root);
            let (app, task) = App::new(super::super::Start::Folder(root.clone()), true);
            drop(task);
            return Some((app, root));
        }
        let root = crate::files::test_dir(name);
        std::fs::create_dir_all(root.join("config/mira")).unwrap();
        std::fs::write(root.join("config/mira/config"), "terminal = shell\nautosave = off\ntheme = LAGC Tech Light\n")
            .unwrap();
        let git = |args: &[&str]| crate::git::git(&root, args).unwrap();
        git(&["init", "-q", "-b", "feature/demo"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ]);
        git(&["remote", "add", "team", "git@example.invalid:team/project.git"]);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("app::remote::tests::{name}"), "--nocapture"])
            .env("MIRA_REMOTE_TEST_ROOT", &root)
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
    fn publish_cancel_stale_choices_and_old_folders_never_start_a_push() {
        let Some((mut app, root)) = fixture("publish_cancel_stale_choices_and_old_folders_never_start_a_push") else {
            return;
        };
        let plan = remote::prepare(&root, Operation::Push).unwrap();
        drop(app.update(Msg::RemotePrepared(app.root_epoch, Operation::Push, Ok(plan.clone()))));
        let first = app.remote_dialog.as_ref().unwrap().id;
        assert!(!app.git_busy);
        capture(&app, "publish");
        drop(app.remote_key(keyboard::Key::Named(keyboard::key::Named::Enter), false, false));
        assert!(app.remote_dialog.is_none(), "Enter starts on Cancel");
        drop(app.update(Msg::RemotePrepared(app.root_epoch, Operation::Push, Ok(plan))));
        let second = app.remote_dialog.as_ref().unwrap().id;
        drop(app.update(Msg::RemoteAction(first, RemoteChoice::Publish)));
        assert_eq!(app.remote_dialog.as_ref().unwrap().id, second);
        drop(app.update(Msg::Push));
        assert!(!app.git_busy, "input behind the modal cannot start a new push");
        app.root_epoch += 1;
        drop(app.update(Msg::RemoteAction(second, RemoteChoice::Publish)));
        assert!(!app.git_busy);
        assert!(app.remote_dialog.is_none());
    }
    #[test]
    fn errors_persist_and_only_connection_failures_offer_ipv4_retry() {
        let Some((mut app, root)) = fixture("errors_persist_and_only_connection_failures_offer_ipv4_retry") else {
            return;
        };
        let mut plan = remote::prepare(&root, Operation::Push).unwrap();
        plan.publish_to = Some("team".into());
        drop(app.update(Msg::RemoteDone(app.root_epoch, plan.clone(), false, Err("Connection timed out".into()))));
        let dialog = app.remote_dialog.as_ref().unwrap();
        let id = dialog.id;
        assert!(dialog.choices.iter().any(|(_, c)| *c == RemoteChoice::Retry(true)));
        assert!(dialog.context.contains("feature/demo"));
        assert!(dialog.context.contains("team/feature/demo"));
        capture(&app, "connection-error");
        app.clear_status();
        assert_eq!(app.remote_dialog.as_ref().unwrap().id, id, "status expiry does not hide a failure");
        drop(app.update(Msg::RemoteAction(id, RemoteChoice::Retry(true))));
        assert!(app.git_busy);
        assert_eq!(app.busy_op, Some(BusyOp::Push));
        assert!(app.status.as_ref().unwrap().0.contains("IPv4"));
        drop(app.update(Msg::RemoteDone(
            app.root_epoch,
            plan.clone(),
            true,
            Err("Permission denied (publickey).".into()),
        )));
        let dialog = app.remote_dialog.as_ref().unwrap();
        assert!(!dialog.choices.iter().any(|(_, c)| *c == RemoteChoice::Retry(false)));
        assert!(dialog.next.contains("SSH key/agent"));
        let id = dialog.id;
        drop(app.update(Msg::RemoteAction(id, RemoteChoice::Close)));
        app.root_epoch += 1;
        drop(app.update(Msg::RemoteDone(app.root_epoch - 1, plan, false, Err("Connection refused".into()))));
        let id = app.remote_dialog.as_ref().unwrap().id;
        drop(app.update(Msg::RemoteAction(id, RemoteChoice::Retry(true))));
        assert!(!app.git_busy, "a previous folder's failure cannot run against the new folder");
    }
    /// Optional local captures use the actual modal view and iced's headless renderer.
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
        let size = iced::Size::new(800.0, 760.0);
        let mut view = app.view();
        let mut tree = Tree::new(&view);
        let node = view.as_widget_mut().layout(&mut tree, &renderer, &layout::Limits::new(iced::Size::ZERO, size));
        let bounds = iced::Rectangle::new(iced::Point::ORIGIN, size);
        view.as_widget().draw(
            &tree,
            &mut renderer,
            &iced::Theme::Light,
            &renderer::Style::default(),
            Layout::new(&node),
            iced::mouse::Cursor::Unavailable,
            &bounds,
        );
        let pixels = renderer.screenshot(iced::Size::new(800, 760), 1.0, crate::theme::p().sidebar);
        std::fs::create_dir_all(&dir).unwrap();
        image::save_buffer(PathBuf::from(dir).join(format!("{name}.png")), &pixels, 800, 760, image::ColorType::Rgba8)
            .unwrap();
    }
}
