//! Agent runs: one per working period of an agent in this folder, as herdr
//! reports it. A run opens when an agent turns busy and closes when it is ready
//! again (or its completion counter moves, or it goes away). mira snapshots the
//! working tree at both ends, so each run has a reviewable before/after diff.
//!
//! [`Tracker::observe`] is pure: it turns what herdr reported into
//! [`Effect`]s that the app executes (snapshots), which keeps it testable.

use crate::git::Change;
use crate::herdr::{AgentStatus, PaneId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Runs kept per folder; the oldest reviewed ones go first.
pub const MAX_RUNS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(pub u64);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub pane: PaneId,
    /// Display name: the agent's herdr name, else its kind.
    pub agent: String,
    /// Agent kind (`codex`, `kiro`, …) when herdr knows it.
    #[serde(default)]
    pub kind: Option<String>,
    pub dir: PathBuf,
    /// What the user asked, when it can be read from the agent's own log.
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
    /// Unix seconds.
    pub started: u64,
    #[serde(default)]
    pub finished: Option<u64>,
    #[serde(default)]
    pub reviewed: bool,
    /// Changed files, computed from the trees (not persisted).
    #[serde(skip)]
    pub files: Option<Vec<Change>>,
}

impl Run {
    pub fn running(&self) -> bool {
        self.finished.is_none()
    }

    /// Ready to review: finished with both snapshots.
    pub fn trees(&self) -> Option<(&str, &str)> {
        Some((self.before.as_deref()?, self.after.as_deref()?))
    }
}

/// One agent as seen in one poll, already filtered to this folder.
#[derive(Debug, Clone)]
pub struct Seen {
    pub pane: PaneId,
    pub status: AgentStatus,
    pub seq: Option<u64>,
    pub agent: String,
    pub kind: Option<String>,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Snapshot the tree now and store it as the run's "before".
    Before(RunId),
    /// The run finished: snapshot "after" and compute its files.
    After(RunId),
}

#[derive(Debug, Clone, Copy)]
struct PaneTrack {
    status: AgentStatus,
    seq: Option<u64>,
    open: Option<RunId>,
}

#[derive(Debug, Default)]
pub struct Tracker {
    pub runs: Vec<Run>,
    panes: HashMap<PaneId, PaneTrack>,
    next: u64,
}

impl Tracker {
    /// Restore saved runs. A run that was still open when mira closed stays
    /// open on its pane: herdr kept the agent running, so the next poll
    /// closes it normally (or right away if the pane is gone).
    pub fn from_runs(runs: Vec<Run>) -> Self {
        let next = runs.iter().map(|r| r.id.0 + 1).max().unwrap_or(0);
        let panes = runs
            .iter()
            .filter(|r| r.running())
            .map(|r| (r.pane.clone(), PaneTrack { status: AgentStatus::Working, seq: None, open: Some(r.id) }))
            .collect();
        Self { runs, panes, next }
    }

    pub fn get(&self, id: RunId) -> Option<&Run> {
        self.runs.iter().find(|r| r.id == id)
    }

    pub fn get_mut(&mut self, id: RunId) -> Option<&mut Run> {
        self.runs.iter_mut().find(|r| r.id == id)
    }

    pub fn unreviewed(&self) -> usize {
        self.runs
            .iter()
            .filter(|r| !r.running() && !r.reviewed && r.files.as_ref().is_none_or(|f| !f.is_empty()))
            .count()
    }

    /// Feed one poll of herdr's agents (only those in this folder). `now` is
    /// unix seconds.
    pub fn observe(&mut self, seen: &[Seen], now: u64) -> Vec<Effect> {
        let mut effects = Vec::new();
        for s in seen {
            let track = *self.panes.entry(s.pane.clone()).or_insert(PaneTrack {
                status: AgentStatus::Unknown,
                seq: s.seq,
                open: None,
            });
            // `unknown` is a detection gap, not a transition.
            let status = if s.status == AgentStatus::Unknown { track.status } else { s.status };
            let seq_moved = matches!((track.seq, s.seq), (Some(a), Some(b)) if a != b);
            let mut open = track.open;
            if let Some(id) = open
                && (status.ready() || seq_moved)
            {
                self.close(id, now);
                effects.push(Effect::After(id));
                open = None;
            }
            if open.is_none() && status.busy() {
                let id = self.open(s, now);
                effects.push(Effect::Before(id));
                open = Some(id);
            }
            self.panes.insert(s.pane.clone(), PaneTrack { status, seq: s.seq.or(track.seq), open });
        }
        // Agents that left (exited, closed, moved out of this folder).
        let gone: Vec<PaneId> = self.panes.keys().filter(|p| !seen.iter().any(|s| &s.pane == *p)).cloned().collect();
        for pane in gone {
            if let Some(PaneTrack { open: Some(id), .. }) = self.panes.remove(&pane) {
                self.close(id, now);
                effects.push(Effect::After(id));
            }
        }
        self.trim();
        effects
    }

    fn open(&mut self, s: &Seen, now: u64) -> RunId {
        let id = RunId(self.next);
        self.next += 1;
        self.runs.push(Run {
            id,
            pane: s.pane.clone(),
            agent: s.agent.clone(),
            kind: s.kind.clone(),
            dir: s.dir.clone(),
            prompt: None,
            before: None,
            after: None,
            started: now,
            finished: None,
            reviewed: false,
            files: None,
        });
        id
    }

    fn close(&mut self, id: RunId, now: u64) {
        if let Some(r) = self.get_mut(id) {
            r.finished = Some(now.max(r.started));
        }
    }

    fn trim(&mut self) {
        while self.runs.len() > MAX_RUNS {
            let victim =
                self.runs.iter().position(|r| r.reviewed).or_else(|| self.runs.iter().position(|r| !r.running()));
            match victim {
                Some(i) => {
                    self.runs.remove(i);
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(pane: &str, status: AgentStatus, seq: Option<u64>) -> Seen {
        Seen {
            pane: PaneId(pane.into()),
            status,
            seq,
            agent: "codex".into(),
            kind: Some("codex".into()),
            dir: PathBuf::from("/repo"),
        }
    }

    use AgentStatus::{Blocked, Done, Idle, Unknown, Working};

    #[test]
    fn a_working_period_becomes_one_run() {
        let mut t = Tracker::default();
        assert_eq!(t.observe(&[seen("w1:p1", Idle, Some(1))], 10), vec![]);
        assert_eq!(t.observe(&[seen("w1:p1", Working, Some(1))], 11), vec![Effect::Before(RunId(0))]);
        // Blocked on an approval and unknown frames stay inside the run.
        assert_eq!(t.observe(&[seen("w1:p1", Blocked, Some(1))], 12), vec![]);
        assert_eq!(t.observe(&[seen("w1:p1", Unknown, Some(1))], 13), vec![]);
        assert_eq!(t.observe(&[seen("w1:p1", Done, Some(2))], 14), vec![Effect::After(RunId(0))]);
        assert_eq!(t.observe(&[seen("w1:p1", Idle, Some(2))], 15), vec![]);
        let r = t.get(RunId(0)).expect("run");
        assert_eq!((r.started, r.finished), (11, Some(14)));
    }

    #[test]
    fn a_completion_between_polls_splits_runs() {
        let mut t = Tracker::default();
        t.observe(&[seen("w1:p1", Working, Some(1))], 1);
        // Finished and started again within one poll: the counter moved.
        assert_eq!(
            t.observe(&[seen("w1:p1", Working, Some(2))], 2),
            vec![Effect::After(RunId(0)), Effect::Before(RunId(1))]
        );
    }

    #[test]
    fn agents_are_independent_and_leaving_closes_the_run() {
        let mut t = Tracker::default();
        let both = [seen("w1:p1", Working, None), seen("w1:p2", Idle, None)];
        assert_eq!(t.observe(&both, 1), vec![Effect::Before(RunId(0))]);
        assert_eq!(
            t.observe(&[seen("w1:p2", Working, None)], 2),
            vec![Effect::Before(RunId(1)), Effect::After(RunId(0))]
        );
        assert!(t.get(RunId(0)).is_some_and(|r| !r.running()));
        assert!(t.get(RunId(1)).is_some_and(Run::running));
    }

    #[test]
    fn restored_open_runs_close_on_the_next_poll() {
        let mut t = Tracker::default();
        t.observe(&[seen("w1:p1", Working, Some(5))], 1);
        let mut back = Tracker::from_runs(t.runs.clone());
        assert_eq!(back.observe(&[seen("w1:p1", Idle, Some(6))], 9), vec![Effect::After(RunId(0))]);
        // New ids continue after the restored ones.
        assert_eq!(back.observe(&[seen("w1:p1", Working, Some(6))], 10), vec![Effect::Before(RunId(1))]);
    }

    #[test]
    fn keeps_at_most_max_runs_dropping_reviewed_first() {
        let mut t = Tracker::default();
        for i in 0..(MAX_RUNS as u64 + 5) {
            t.observe(&[seen("w1:p1", Working, Some(i))], i * 2);
            t.observe(&[seen("w1:p1", Idle, Some(i + 1))], i * 2 + 1);
            if i == 50
                && let Some(r) = t.get_mut(RunId(10))
            {
                r.reviewed = true;
            }
        }
        assert_eq!(t.runs.len(), MAX_RUNS);
        assert!(t.get(RunId(10)).is_none(), "reviewed run dropped first");
        assert!(t.get(RunId(0)).is_none());
    }
}

#[cfg(test)]
mod e2e {
    /// Live check against a real herdr server: `MIRA_E2E_SOCKET=… MIRA_E2E_ROOT=…
    /// cargo test e2e -- --ignored --nocapture`. Polls for 90 s.
    #[test]
    #[ignore]
    fn tracks_a_real_agent() {
        use super::*;
        let (Ok(sock), Ok(root)) = (std::env::var("MIRA_E2E_SOCKET"), std::env::var("MIRA_E2E_ROOT")) else { return };
        let client = crate::herdr::Client::new(sock.into());
        let mut t = Tracker::default();
        for _ in 0..180 {
            let agents = client.agents().expect("agent.list");
            let seen: Vec<Seen> = agents
                .iter()
                .filter(|a| a.dir().is_some_and(|d| d.starts_with(&root)))
                .map(|a| Seen {
                    pane: a.pane_id.clone(),
                    status: a.agent_status,
                    seq: a.completion_seq,
                    agent: a.label(),
                    kind: a.agent.clone(),
                    dir: a.dir().map(std::path::Path::to_path_buf).unwrap_or_default(),
                })
                .collect();
            let fx = t.observe(&seen, crate::persist::now());
            let st: Vec<String> = seen.iter().map(|s| format!("{}:{:?}:{:?}", s.agent, s.status, s.seq)).collect();
            if !fx.is_empty() {
                println!("E2E {st:?} -> {fx:?}");
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        println!("E2E runs: {:?}", t.runs.iter().map(|r| (r.id, &r.agent, r.started, r.finished)).collect::<Vec<_>>());
    }
}
