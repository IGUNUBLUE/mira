//! Global update state; unrelated to the currently followed repository.
use super::{App, Dialog, Msg, Tab, TermKind, blocking};
use crate::{persist, updater};
use iced::Task;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Checking,
    Downloading,
    Saving,
    Installing,
    Restarting,
}

pub struct Updates {
    pub automatic: bool,
    pub loaded: bool,
    pub phase: Phase,
    pub status: String,
    pub error: Option<String>,
    pub release: Option<updater::Release>,
    pub target: Result<updater::Target, String>,
    pub ready: Option<Arc<updater::Prepared>>,
    pub installed: Option<Arc<updater::Installed>>,
    pub cache: updater::Cache,
    generation: u64,
    cancel: Arc<AtomicBool>,
    last_attempt: Option<Instant>,
    manual: bool,
    health_sent: bool,
    pending_check: bool,
}
impl Updates {
    pub fn new() -> Self {
        Self {
            automatic: updater::automatic(),
            loaded: false,
            phase: Phase::Idle,
            status: "Reading update information…".into(),
            error: None,
            release: None,
            target: Err("Reading installation information…".into()),
            ready: None,
            installed: None,
            cache: updater::Cache::default(),
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            last_attempt: None,
            manual: false,
            health_sent: false,
            pending_check: false,
        }
    }
    pub fn cancel_network(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn busy(&self) -> bool {
        self.phase != Phase::Idle
    }
    pub fn visual_busy(&self) -> bool {
        self.busy() && (self.phase != Phase::Checking || self.manual)
    }
    pub fn notice(&self) -> bool {
        self.release.is_some() || self.ready.is_some() || self.installed.is_some()
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Loaded(Result<Arc<updater::Loaded>, String>),
    Open,
    CheckAndOpen,
    Check,
    Automatic(bool),
    Checked(u64, Result<Arc<(updater::Cache, Option<updater::Release>)>, String>),
    Saved(u64, Result<updater::Cache, String>),
    Notes,
    Download,
    Progress(u64, u64, u64),
    Downloaded(u64, Result<Arc<updater::Prepared>, String>),
    Cancel,
    Install,
    Installed(Result<Arc<updater::Installed>, String>),
    Restarted(Result<(), String>),
    CopyError,
    WindowOpened,
    Acknowledged(Result<(), String>),
}
impl Event {
    pub fn background(&self) -> bool {
        matches!(
            self,
            Self::Loaded(_)
                | Self::Checked(..)
                | Self::Saved(..)
                | Self::Progress(..)
                | Self::Downloaded(..)
                | Self::Installed(_)
                | Self::Restarted(_)
                | Self::WindowOpened
                | Self::Acknowledged(_)
        )
    }
}
fn message(event: Event) -> Msg {
    Msg::Update(event)
}

pub fn load() -> Task<Msg> {
    Task::perform(blocking(|| updater::load().map(Arc::new)), |r| message(Event::Loaded(r)))
}

impl App {
    pub(super) fn maybe_check_updates(&mut self) -> Task<Msg> {
        if self.updates.automatic
            && self.updates.loaded
            && !self.updates.busy()
            && self.updates.ready.is_none()
            && self.updates.installed.is_none()
            && updater::due(self.updates.cache.checked_at, persist::now())
            && self.updates.last_attempt.is_none_or(|at| at.elapsed() >= updater::INTERVAL)
        {
            return self.check_updates(false);
        }
        Task::none()
    }
    fn check_updates(&mut self, manual: bool) -> Task<Msg> {
        if !self.updates.loaded {
            self.updates.pending_check |= manual;
            return Task::none();
        }
        if self.updates.busy() || self.updates.ready.is_some() || self.updates.installed.is_some() {
            return Task::none();
        }
        self.updates.generation += 1;
        let id = self.updates.generation;
        self.updates.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.updates.cancel.clone();
        let cache = self.updates.cache.clone();
        self.updates.phase = Phase::Checking;
        self.updates.manual = manual;
        self.updates.last_attempt = Some(Instant::now());
        self.updates.error = None;
        self.updates.status = "Checking for updates…".into();
        Task::perform(
            blocking(move || {
                updater::check(cache, &cancel)
                    .and_then(|cache| updater::available(&cache).map(|release| Arc::new((cache, release))))
            }),
            move |r| message(Event::Checked(id, r)),
        )
    }
    fn update_blocker(&self) -> Option<&'static str> {
        if self.all_document_tabs().any(|t| matches!(t, Tab::File(f) if f.dirty)) {
            Some(
                "Save your edited files, including files in other workspaces, before restarting. The download stays ready.",
            )
        } else if self.git_busy || self.replacing || self.commit_gen.is_some() || self.signin.is_some() {
            Some("Finish or stop the current Git, replace or commit-message operation before restarting.")
        } else if !self.commit_msg.text().trim().is_empty() {
            Some("Finish or clear your draft commit message before restarting. The download stays ready.")
        } else if self.name_edit.is_some() {
            Some("Finish or cancel file name editing before restarting.")
        } else if self.confirmation.is_some() || self.remote_dialog.is_some() || self.branch_picker.is_some() {
            Some("Finish the current dialog before restarting.")
        } else {
            None
        }
    }
    fn restart_updated(&mut self, installed: Arc<updater::Installed>) -> Task<Msg> {
        if let Some(reason) = self.update_blocker() {
            self.updates.error = Some(reason.into());
            self.updates.phase = Phase::Idle;
            return Task::none();
        }
        self.save_state();
        self.updates.phase = Phase::Restarting;
        self.updates.status = "Starting the updated Mira…".into();
        let root = self.root.clone();
        let follow = matches!(self.term_kind, TermKind::Herdr);
        Task::perform(blocking(move || updater::restart(&installed, &root, follow)), |r| message(Event::Restarted(r)))
    }
    pub(super) fn update_updates(&mut self, event: Event) -> Task<Msg> {
        match event {
            Event::Loaded(result) => {
                self.updates.loaded = true;
                match result {
                    Ok(loaded) => {
                        self.updates.cache = loaded.cache.clone();
                        self.updates.release = loaded.available.clone();
                        self.updates.target = loaded.target.clone();
                        self.updates.ready = loaded.ready.clone().map(Arc::new);
                        self.updates.status = if self.updates.ready.is_some() {
                            "Update downloaded and verified. Ready when you are.".into()
                        } else if let Some(release) = &self.updates.release {
                            release.label.clone()
                        } else if self.updates.cache.checked_at > self.updates.cache.successful_at {
                            "The last check did not finish. Choose Check for updates to try again.".into()
                        } else if self.updates.cache.successful_at != 0 {
                            "The last check found no newer release.".into()
                        } else {
                            "Choose Check for updates to look for a new release.".into()
                        };
                    }
                    Err(e) => {
                        self.updates.error = Some(e);
                        self.updates.status = "Update information is unavailable.".into();
                    }
                }
                if self.updates.pending_check {
                    self.updates.pending_check = false;
                    return self.check_updates(true);
                }
                return self.maybe_check_updates();
            }
            Event::Open | Event::CheckAndOpen => {
                let check = matches!(event, Event::CheckAndOpen);
                if self.dialog == Some(Dialog::Settings) {
                    let task = self.update_settings(Msg::CloseDialog);
                    if self.dialog.is_some() {
                        return task;
                    }
                }
                self.gear_menu = false;
                self.dialog = Some(Dialog::Updates);
                let focus = iced::widget::operation::focus("mira-updates-dialog");
                return if check { Task::batch([focus, self.check_updates(true)]) } else { focus };
            }
            Event::Check => return self.check_updates(true),
            Event::Automatic(on) => {
                if let Err(e) = persist::config_set("update_checks", if on { "on" } else { "off" }) {
                    self.updates.error = Some(e);
                    return Task::none();
                }
                self.updates.automatic = on;
                if !on && self.updates.phase == Phase::Checking && !self.updates.manual {
                    self.updates.cancel.store(true, Ordering::Relaxed);
                    self.updates.generation += 1;
                    self.updates.phase = Phase::Idle;
                    self.updates.status = "Automatic update checks are off.".into();
                }
                if on {
                    return self.maybe_check_updates();
                }
            }
            Event::Checked(id, result) if id == self.updates.generation => {
                self.updates.phase = Phase::Idle;
                self.updates.cache.checked_at = persist::now();
                match result {
                    Ok(checked) => {
                        self.updates.cache = checked.0.clone();
                        self.updates.release = checked.1.clone();
                        self.updates.status = self
                            .updates
                            .release
                            .as_ref()
                            .map_or_else(|| "The last check found no newer release.".into(), |r| r.label.clone());
                    }
                    Err(e) => {
                        self.updates.error = Some(e);
                        self.updates.status = "Could not check for updates.".into();
                    }
                }
            }
            Event::Download => {
                if self.updates.busy() || self.updates.ready.is_some() || self.updates.installed.is_some() {
                    return Task::none();
                }
                let (Some(release), Ok(target)) = (self.updates.release.clone(), self.updates.target.clone()) else {
                    return Task::none();
                };
                self.updates.generation += 1;
                let id = self.updates.generation;
                self.updates.cancel = Arc::new(AtomicBool::new(false));
                let cancel = self.updates.cancel.clone();
                self.updates.phase = Phase::Downloading;
                self.updates.status = "Downloading update…".into();
                self.updates.error = None;
                let (tx, rx) = iced::futures::channel::mpsc::unbounded();
                std::thread::spawn(move || {
                    let result = updater::download(release, target, &cancel, &mut |done, total| {
                        let _ = tx.unbounded_send(Event::Progress(id, done, total)); // Receiver may close on cancellation/exit.
                    })
                    .map(Arc::new);
                    let _ = tx.unbounded_send(Event::Downloaded(id, result));
                });
                return Task::run(rx, message);
            }
            Event::Progress(id, done, total)
                if id == self.updates.generation && self.updates.phase == Phase::Downloading =>
            {
                self.updates.status = format!(
                    "Downloading update… {}%",
                    done.saturating_mul(100).checked_div(total).unwrap_or(0).min(100)
                );
            }
            Event::Downloaded(id, result) if id == self.updates.generation => {
                self.updates.phase = Phase::Idle;
                match result {
                    Ok(prepared) => {
                        self.updates.ready = Some(prepared.clone());
                        self.updates.phase = Phase::Saving;
                        self.updates.status = "Recording the verified update…".into();
                        let cache = self.updates.cache.clone();
                        return Task::perform(blocking(move || updater::cache_ready(cache, &prepared)), move |r| {
                            message(Event::Saved(id, r))
                        });
                    }
                    Err(e) => {
                        self.updates.error = Some(e);
                        self.updates.status = "The update could not be downloaded.".into();
                    }
                }
            }
            Event::Saved(id, result) if id == self.updates.generation => {
                self.updates.phase = Phase::Idle;
                match result {
                    Ok(cache) => {
                        self.updates.cache = cache;
                        self.updates.status = "Update downloaded and verified. Ready when you are.".into();
                    }
                    Err(e) => {
                        self.updates.ready = None;
                        self.updates.error = Some(e);
                        self.updates.status = "Could not record the download. Try downloading again.".into();
                    }
                }
            }
            Event::Notes => {
                let url = self
                    .updates
                    .release
                    .as_ref()
                    .map_or_else(|| format!("{}/releases", updater::REPO), |r| r.notes.clone());
                return self.update_settings(Msg::OpenLink(url));
            }
            Event::Cancel => {
                if matches!(self.updates.phase, Phase::Checking | Phase::Downloading) {
                    self.updates.cancel.store(true, Ordering::Relaxed);
                    self.updates.generation += 1;
                    self.updates.phase = Phase::Idle;
                    self.updates.status = "Update request cancelled.".into();
                    self.updates.error = None;
                }
            }
            Event::Install => {
                if self.updates.busy() {
                    return Task::none();
                }
                if let Some(reason) = self.update_blocker() {
                    self.updates.error = Some(reason.into());
                    return Task::none();
                }
                if let Some(installed) = self.updates.installed.clone() {
                    return self.restart_updated(installed);
                }
                let Some(prepared) = self.updates.ready.clone() else {
                    return Task::none();
                };
                self.updates.phase = Phase::Installing;
                self.updates.status = "Installing the verified update…".into();
                self.updates.error = None;
                return Task::perform(
                    blocking(move || updater::install(&prepared, &AtomicBool::new(false)).map(Arc::new)),
                    |r| message(Event::Installed(r)),
                );
            }
            Event::Installed(result) => {
                self.updates.phase = Phase::Idle;
                self.updates.ready = None;
                match result {
                    Ok(installed) => {
                        self.updates.installed = Some(installed.clone());
                        return self.restart_updated(installed);
                    }
                    Err(e) => {
                        self.updates.error = Some(e);
                        self.updates.status = "The current Mira stays open.".into();
                    }
                }
            }
            Event::Restarted(result) => {
                self.updates.phase = Phase::Idle;
                // Restart is one-shot. After rollback or a late startup, manual reopening is safe.
                self.updates.installed = None;
                self.updates.target = Err("Close this window and reopen Mira before trying another update.".into());
                match result {
                    Ok(()) if self.update_blocker().is_none() => return iced::exit(),
                    Ok(()) => {
                        self.updates.error =
                            Some("The new Mira is ready. Save any edits in this window, then close it.".into())
                    }
                    Err(e) => self.updates.error = Some(e),
                }
                self.updates.status = "This window has been kept open.".into();
            }
            Event::CopyError => {
                if let Some(error) = &self.updates.error {
                    return iced::clipboard::write(error.clone());
                }
            }
            Event::WindowOpened if !self.updates.health_sent && updater::has_health() => {
                self.updates.health_sent = true;
                return Task::perform(blocking(updater::acknowledge), |r| message(Event::Acknowledged(r)));
            }
            Event::Acknowledged(Err(e)) => self.updates.error = Some(e),
            Event::Checked(..)
            | Event::Saved(..)
            | Event::Progress(..)
            | Event::Downloaded(..)
            | Event::WindowOpened
            | Event::Acknowledged(Ok(())) => {}
        }
        Task::none()
    }
}
