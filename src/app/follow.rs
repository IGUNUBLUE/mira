//! herdr: polling agents, following herdr's focused workspace, sending text to agents.

use super::{App, AutoSave, Focus, Msg, SearchState, Tab, blocking, file_name};
use crate::explorer::Explorer;
use crate::git::{self, Snapshot};
use crate::herdr::{self, Agent, PaneId};
use crate::persist;
use crate::runs::{RunId, Seen, Tracker};

use iced::Task;
use iced::widget::text_editor::Content;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

impl App {
    pub(super) fn update_follow(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::StatusTick => {
                // A pending confirmation goes with the status that asked for it.
                if self.status_expired() {
                    self.clear_status();
                }
                self.save_state();
                // In the background, every third tick (about 2 s) is enough to catch runs.
                self.status_ticks = self.status_ticks.wrapping_add(1);
                if !self.window_focused && !self.status_ticks.is_multiple_of(3) {
                    return Task::none();
                }
                return self.poll_herdr();
            }
            Msg::HerdrPolled(result) => {
                self.polling = false;
                match result {
                    Ok((agents, focus)) => {
                        self.herdr_error = None;
                        let follow = focus.map_or_else(Task::none, |f| self.follow(f));
                        return Task::batch([follow, self.on_agents(agents, true)]);
                    }
                    // No server: its agents are gone (or the client in the
                    // terminal is still starting it). Close what was open; the
                    // workspace is asked for once the server answers.
                    Err(herdr::Error::NotRunning) => {
                        self.herdr_error = None;
                        return self.on_agents(Vec::new(), false);
                    }
                    Err(e) => self.herdr_error = Some(e.to_string()),
                }
            }
            Msg::ForRoot(epoch, msg) => {
                if epoch == self.root_epoch {
                    return self.update(*msg);
                }
            }
            Msg::WorkspaceReady(result) => {
                self.ensuring_workspace = false;
                match result {
                    Ok(ws) => self.workspace = Some(ws),
                    // The server stopped in between: the next poll that reaches it asks again.
                    Err(herdr::Error::NotRunning) => {}
                    Err(e) => {
                        self.workspace_retry = Instant::now() + Duration::from_secs(30);
                        self.set_status(format!("herdr workspace: {e}"), true);
                    }
                }
            }
            Msg::SendToAgent(text) => return self.send_to_agent(text, None, false),
            Msg::FocusAgent(pane) => {
                let focus = self.focus_terminal();
                let Some(client) = self.herdr.clone() else { return focus };
                let task = Task::perform(blocking(move || client.focus_pane(&pane)), Msg::AgentFocused);
                return Task::batch([task, focus]);
            }
            Msg::AgentFocused(Err(e)) => self.set_status(e.to_string(), true),
            Msg::SentToPane(epoch, result) => {
                // The comment box empties only once the agent got it.
                let comment = std::mem::take(&mut self.sending_comment);
                if epoch != self.root_epoch {
                    if let Err(e) = result {
                        self.set_status(format!("Previous folder: {e}"), true);
                    }
                    return Task::none();
                }
                match result {
                    Ok(()) => {
                        if comment {
                            self.comment.clear();
                        }
                        self.set_status("Sent to the agent — review and press Enter", false);
                    }
                    Err(e) => {
                        self.set_status(format!("{e}{}", if comment { " — the comment is kept" } else { "" }), true)
                    }
                }
            }
            _ => {}
        }
        Task::none()
    }

    fn poll_herdr(&mut self) -> Task<Msg> {
        let Some(client) = self.herdr.clone() else { return Task::none() };
        if self.polling {
            return Task::none();
        }
        self.polling = true;
        // Follow herdr's focus once mira knows its workspace (or is waiting to adopt one).
        let follow = (self.workspace.is_some() || self.adopt_focused) && !self.ensuring_workspace;
        let since = self.workspace.clone();
        Task::perform(
            blocking(move || {
                let agents = client.agents()?;
                let focus = if follow {
                    // Resolve the folder (a git call) only when the focus moved.
                    client.panes().ok().and_then(|panes| {
                        let moved = herdr::focused(&panes).is_some_and(|p| Some(&p.workspace_id) != since.as_ref());
                        moved.then(|| focused_root(&panes)).flatten()
                    })
                } else {
                    None
                };
                Ok((agents, focus.map(|(workspace, root)| Focus { workspace, root, since })))
            }),
            Msg::HerdrPolled,
        )
    }

    /// Wrap a task whose result belongs to the current folder.
    pub(super) fn for_root(&self, task: Task<Msg>) -> Task<Msg> {
        let epoch = self.root_epoch;
        task.map(move |m| Msg::ForRoot(epoch, Box::new(m)))
    }

    /// herdr's focused workspace changed: show its folder.
    fn follow(&mut self, focus: Focus) -> Task<Msg> {
        if focus.since != self.workspace || self.ensuring_workspace {
            return Task::none();
        }
        self.workspace = Some(focus.workspace);
        self.adopt_focused = false;
        if focus.root == self.root {
            return Task::none();
        }
        self.switch_root(focus.root)
    }

    /// Show another folder: git, explorer, search and agent turns follow it.
    /// File tabs stay open (they are absolute paths); diff tabs belong to the
    /// old repository and close.
    fn switch_root(&mut self, root: PathBuf) -> Task<Msg> {
        if self.autosave != AutoSave::Off {
            self.save_dirty_tabs();
        }
        self.save_state();
        if let Some(job) = self.commit_gen.take() {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.root_epoch += 1;
        let _ = persist::set_last_root(&root);
        // Known again once this folder's first snapshot lands.
        self.repo_prefix = String::new();
        self.git_loaded = false;
        self.explorer = Explorer::new(root.clone());
        let runs = persist::load(&root).map(|s| s.runs).unwrap_or_default();
        self.runs = Tracker::from_runs(runs);
        self.root = root;
        self.git = Snapshot::default();
        self.git_rev += 1;
        // A refresh of the old folder may still hold the old cache; it's dropped with it.
        self.git_cache = Default::default();
        self.refreshing = false;
        self.manual_refresh = false;
        self.busy_op = None;
        // Snapshots queued or in flight for the old folder still land in its saved state.
        self.snapshots_off = false;
        self.selected_run = None;
        // Undo of a discard belongs to the old folder's files.
        self.undo_discard = None;
        self.hovered_change = None;
        self.clear_confirms();
        self.remote_dialog = None;
        self.branch_picker = None;
        self.state_dirty = false;
        self.agents.clear();
        self.commit_msg = Content::new();
        self.commit_history = crate::history::History::default();
        // Same shared counter, moved on: a search of the old folder stops.
        let generation = std::sync::Arc::clone(&self.search.generation);
        generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.search = SearchState { generation, ..SearchState::default() };
        // Its file list is the old folder's, and so are the Files menu and a name being typed.
        self.palette = None;
        self.explorer_menu = None;
        self.name_edit = None;
        self.explorer_target = None;
        let keep: Vec<bool> = self.tabs.iter().map(|t| t.file_path().is_some()).collect();
        self.active_tab = super::documents::active_after_retain(self.active_tab, &keep);
        self.tabs.retain(|t| t.file_path().is_some());
        for tab in &mut self.tabs {
            match tab {
                Tab::File(t) => t.crumbs = super::documents::crumbs(&self.root, &t.path),
                Tab::Image(t) => t.crumbs = super::documents::crumbs(&self.root, &t.path),
                Tab::Diff(_) => {}
            }
        }
        self.sync_center();
        self.set_status(format!("Workspace: {}", file_name(&self.root)), false);
        Task::batch([self.refresh_git(), self.load_all_run_files()])
    }

    /// herdr's agents, as polled (`server`: herdr answered; with no server
    /// there is no workspace to ask for).
    fn on_agents(&mut self, agents: Vec<Agent>, server: bool) -> Task<Msg> {
        let root = self.root.clone();
        let here: Vec<Agent> = agents.into_iter().filter(|a| a.dir().is_some_and(|d| d.starts_with(&root))).collect();
        let seen: Vec<Seen> = here
            .iter()
            .map(|a| Seen {
                pane: a.pane_id.clone(),
                status: a.agent_status,
                seq: a.completion_seq,
                change: a.state_change_seq,
                agent: a.label(),
                kind: a.agent.clone(),
                dir: a.dir().map_or_else(|| root.clone(), Path::to_path_buf),
            })
            .collect();
        self.agents = here;
        let effects = self.runs.observe(&seen, persist::now());
        if !effects.is_empty() || self.runs.take_touched() {
            self.state_dirty = true;
        }
        let mut tasks: Vec<Task<Msg>> = effects.into_iter().map(|e| self.run_effect(e)).collect();
        if server
            && self.workspace.is_none()
            && !self.adopt_focused
            && !self.ensuring_workspace
            && Instant::now() >= self.workspace_retry
        {
            tasks.push(self.ensure_workspace());
        }
        Task::batch(tasks)
    }

    fn ensure_workspace(&mut self) -> Task<Msg> {
        let Some(client) = self.herdr.clone() else { return Task::none() };
        self.ensuring_workspace = true;
        let (root, label) = (self.root.clone(), file_name(&self.root));
        Task::perform(blocking(move || client.ensure_workspace(&root, &label)), Msg::WorkspaceReady)
    }

    /// Type `text` without pressing Enter: into the pane of run `target`
    /// through herdr when that agent is still there, else into the terminal
    /// (herdr's focused pane) as a paste. `comment`: it is the diff's comment
    /// box, emptied only once the text was delivered.
    pub(super) fn send_to_agent(&mut self, text: String, target: Option<RunId>, comment: bool) -> Task<Msg> {
        if self.sending_comment {
            self.set_status("Wait for the comment to reach the agent", false);
            return Task::none();
        }
        let pane: Option<PaneId> = target
            .and_then(|id| self.runs.get(id))
            .map(|r| r.pane.clone())
            .filter(|p| self.agents.iter().any(|a| &a.pane_id == p));
        let focus = self.focus_terminal();
        match (pane, self.herdr.clone()) {
            (Some(pane), Some(client)) => {
                self.sending_comment = comment;
                let epoch = self.root_epoch;
                let task = Task::perform(
                    blocking(move || {
                        client.send_text(&pane, &text)?;
                        client.focus_pane(&pane)
                    }),
                    move |result| Msg::SentToPane(epoch, result),
                );
                Task::batch([task, focus])
            }
            _ => {
                // A paste, bracketed when the program asked for it (herdr does).
                let Some(t) = &mut self.term else {
                    let what = if comment { "the comment is kept" } else { "nothing was sent" };
                    self.set_status(format!("No terminal to type into ({what}): reattach it first"), true);
                    return Task::none();
                };
                t.apply(crate::term::Action::Paste(text));
                if comment {
                    self.comment.clear();
                }
                let msg = if target.is_some() {
                    "That agent is gone — typed into the focused pane instead"
                } else {
                    "Typed into the focused pane — review and press Enter"
                };
                self.set_status(msg, target.is_some());
                focus
            }
        }
    }
}

/// Folder of the focused pane: its git repository, else its directory.
pub(super) fn focused_root(panes: &[herdr::Pane]) -> Option<(String, PathBuf)> {
    let pane = herdr::focused(panes)?;
    let cwd = pane.cwd.as_deref()?.canonicalize().ok()?;
    let root = git::repo_root(&cwd).and_then(|r| r.canonicalize().ok()).unwrap_or(cwd);
    Some((pane.workspace_id.clone(), root))
}
