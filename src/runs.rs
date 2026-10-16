//! Agent runs: one per working period of an agent in this folder, as herdr
//! reports it. A run opens when an agent turns busy and closes when it is ready
//! again (or its completion counter moves, or it goes away). mira snapshots the
//! working tree at both ends, so each run has a reviewable before/after diff.
//!
//! A turn interrupted by a quick follow-up (the user steering the agent, an
//! approval) stays one run: a run that finished less than [`MERGE_SECS`] ago
//! reopens instead of starting a new one.
//!
//! [`Tracker::observe`] is pure: it turns what herdr reported into
//! [`Effect`]s that the app executes (snapshots), which keeps it testable.

use crate::git::Change;
use crate::herdr::{AgentStatus, PaneId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Runs kept per folder; runs without changes and seen ones go first.
pub const MAX_RUNS: usize = 100;

/// A run that finished this recently reopens when its agent works again.
pub const MERGE_SECS: u64 = 10;

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
    /// Files of this run the user has opened since it changed them.
    #[serde(default)]
    pub seen: Vec<String>,
    /// Changed files, computed from the trees (not persisted).
    #[serde(skip)]
    pub files: Option<Vec<Change>>,
}

impl Run {
    pub fn running(&self) -> bool {
        self.finished.is_none()
    }

    /// Finished and changed at least one file.
    pub fn has_changes(&self) -> bool {
        !self.running() && self.files.as_ref().is_some_and(|f| !f.is_empty())
    }

    pub fn touches(&self, path: &str) -> bool {
        self.files.as_ref().is_some_and(|f| f.iter().any(|c| c.path == path))
    }

    pub fn all_seen(&self) -> bool {
        self.files.as_ref().is_none_or(|f| f.iter().all(|c| self.seen.contains(&c.path)))
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
    runs: Vec<Run>,
    panes: HashMap<PaneId, PaneTrack>,
    next: u64,
    /// Derived from `runs` (rebuilt by [`Tracker::reindex`] after every change
    /// that can move it): per changed path, the position in `runs` of its
    /// latest finished run and whether it is new there. The view asks per
    /// row and per frame, so these lookups must not scan the runs.
    origins: HashMap<String, (usize, bool)>,
    /// Paths in `origins` that are new.
    new_count: usize,
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
        let mut t = Self { runs, panes, next, ..Self::default() };
        t.reindex();
        t
    }

    /// The runs, oldest first.
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    pub fn get(&self, id: RunId) -> Option<&Run> {
        self.runs.iter().find(|r| r.id == id)
    }

    fn get_mut(&mut self, id: RunId) -> Option<&mut Run> {
        self.runs.iter_mut().find(|r| r.id == id)
    }

    /// Change run `id` through `f` (its files, trees or prompt); the derived
    /// index follows.
    pub fn edit<R>(&mut self, id: RunId, f: impl FnOnce(&mut Run) -> R) -> Option<R> {
        let out = f(self.get_mut(id)?);
        self.reindex();
        Some(out)
    }

    /// Rebuild `origins` and `new_count` from `runs`.
    fn reindex(&mut self) {
        self.origins.clear();
        for (i, r) in self.runs.iter().enumerate().filter(|(_, r)| !r.running()) {
            for c in r.files.iter().flatten() {
                // Later runs come later in `runs`, so the last insert wins.
                let new = !r.seen.contains(&c.path);
                match self.origins.get_mut(c.path.as_str()) {
                    Some(slot) => *slot = (i, new),
                    None => {
                        self.origins.insert(c.path.clone(), (i, new));
                    }
                }
            }
        }
        self.new_count = self.origins.values().filter(|(_, new)| *new).count();
    }

    /// The latest finished run that changed `path`.
    pub fn origin(&self, path: &str) -> Option<&Run> {
        self.origins.get(path).and_then(|(i, _)| self.runs.get(*i))
    }

    /// Whether `path` changed since the user last opened it (per its latest run).
    pub fn is_new(&self, path: &str) -> bool {
        self.origins.get(path).is_some_and(|(_, new)| *new)
    }

    /// Distinct files changed by agents and not opened since (the app counts
    /// only those still in Changes: `App::fresh_changes`).
    #[cfg(test)]
    pub fn new_files(&self) -> usize {
        self.new_count
    }

    /// The user opened `path`: it is no longer new in any run.
    pub fn mark_seen(&mut self, path: &str) -> bool {
        let mut changed = false;
        for r in self.runs.iter_mut().filter(|r| r.touches(path) && !r.seen.iter().any(|p| p == path)) {
            r.seen.push(path.to_string());
            changed = true;
        }
        if changed {
            self.reindex();
        }
        changed
    }

    pub fn mark_all_seen(&mut self) {
        for r in self.runs.iter_mut().filter(|r| !r.running()) {
            r.seen = r.files.iter().flatten().map(|c| c.path.clone()).collect();
        }
        self.reindex();
    }

    /// Feed one poll of herdr's agents (only those in this folder). `now` is
    /// unix seconds.
    pub fn observe(&mut self, seen: &[Seen], now: u64) -> Vec<Effect> {
        let mut effects = Vec::new();
        let mut reopened = false;
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
            // A different agent now sits in this pane (e.g. herdr restarted and
            // reused the id): whatever was open belonged to the old one.
            let replaced = open.and_then(|id| self.get(id)).is_some_and(
                |r| matches!((r.kind.as_deref(), s.kind.as_deref()), (Some(was), Some(now)) if was != now),
            );
            // Finished and immediately busy again within one poll: same turn.
            let continued = seq_moved && status.busy() && !replaced;
            if let Some(id) = open
                && (status.ready() || seq_moved || replaced)
                && !continued
            {
                self.close(id, now);
                effects.push(Effect::After(id));
                open = None;
            }
            if open.is_none() && status.busy() {
                let id = match self.recent(&s.pane, s.kind.as_deref(), now) {
                    // Reopen: its "before" still holds, a new "after" comes at the end.
                    Some(id) => {
                        self.reopen(id);
                        reopened = true;
                        id
                    }
                    None => {
                        let id = self.open(s, now);
                        effects.push(Effect::Before(id));
                        id
                    }
                };
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
        let trimmed = self.trim();
        // Opening a run doesn't move the index (it has no files yet), but
        // closing, reopening and trimming can.
        if effects.iter().any(|e| matches!(e, Effect::After(_))) || reopened || trimmed {
            self.reindex();
        }
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
            seen: Vec::new(),
            files: None,
        });
        id
    }

    /// A run of this pane and agent that finished within [`MERGE_SECS`].
    fn recent(&self, pane: &PaneId, kind: Option<&str>, now: u64) -> Option<RunId> {
        let r = self.runs.iter().rev().find(|r| &r.pane == pane)?;
        let finished = r.finished?;
        (r.kind.as_deref() == kind && now.saturating_sub(finished) <= MERGE_SECS).then_some(r.id)
    }

    fn reopen(&mut self, id: RunId) {
        if let Some(r) = self.get_mut(id) {
            r.finished = None;
            r.after = None;
            r.files = None;
        }
    }

    fn close(&mut self, id: RunId, now: u64) {
        if let Some(r) = self.get_mut(id) {
            r.finished = Some(now.max(r.started));
        }
    }

    /// Drop runs past [`MAX_RUNS`]; whether any was dropped.
    fn trim(&mut self) -> bool {
        let before = self.runs.len();
        while self.runs.len() > MAX_RUNS {
            let finished = |r: &Run| !r.running();
            let victim = self
                .runs
                .iter()
                .position(|r| finished(r) && r.files.as_ref().is_some_and(Vec::is_empty))
                .or_else(|| self.runs.iter().position(|r| finished(r) && r.all_seen()))
                .or_else(|| self.runs.iter().position(finished));
            match victim {
                Some(i) => {
                    self.runs.remove(i);
                }
                None => break,
            }
        }
        self.runs.len() != before
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
    fn a_completion_between_polls_continues_the_run() {
        let mut t = Tracker::default();
        t.observe(&[seen("w1:p1", Working, Some(1))], 1);
        // Finished and started again within one poll (steering): same turn.
        assert_eq!(t.observe(&[seen("w1:p1", Working, Some(2))], 2), vec![]);
        assert_eq!(t.observe(&[seen("w1:p1", Idle, Some(3))], 5), vec![Effect::After(RunId(0))]);
        assert_eq!(t.runs.len(), 1);
    }

    #[test]
    fn quick_follow_ups_reopen_and_later_prompts_start_new_runs() {
        let mut t = Tracker::default();
        t.observe(&[seen("w1:p1", Working, None)], 100);
        t.observe(&[seen("w1:p1", Idle, None)], 110);
        // Busy again 5 s later: the same turn goes on, no new "before".
        assert_eq!(t.observe(&[seen("w1:p1", Working, None)], 115), vec![]);
        assert!(t.get(RunId(0)).is_some_and(Run::running));
        assert_eq!(t.observe(&[seen("w1:p1", Idle, None)], 120), vec![Effect::After(RunId(0))]);
        // A minute later it is a new prompt.
        assert_eq!(t.observe(&[seen("w1:p1", Working, None)], 180), vec![Effect::Before(RunId(1))]);
    }

    #[test]
    fn files_are_new_until_opened() {
        let mut t = Tracker::default();
        t.observe(&[seen("w1:p1", Working, None)], 1);
        t.observe(&[seen("w1:p1", Idle, None)], 2);
        let change = |p: &str| Change::new(p, 'M');
        t.edit(RunId(0), |r| r.files = Some(vec![change("a.rs"), change("b.rs")]));
        assert_eq!(t.new_files(), 2);
        assert!(t.is_new("a.rs") && !t.is_new("c.rs"));
        assert!(t.mark_seen("a.rs"));
        assert!(!t.mark_seen("a.rs"));
        assert_eq!(t.new_files(), 1);
        assert_eq!(t.origin("b.rs").map(|r| r.id), Some(RunId(0)));
        t.mark_all_seen();
        assert_eq!(t.new_files(), 0);
    }

    #[test]
    fn the_index_follows_the_latest_run_of_each_file() {
        let mut t = Tracker::default();
        let change = |p: &str| Change::new(p, 'M');
        for (i, start) in [(0, 1), (1, 100)] {
            t.observe(&[seen("w1:p1", Working, None)], start);
            t.observe(&[seen("w1:p1", Idle, None)], start + 1);
            let files = if i == 0 { vec![change("a.rs"), change("b.rs")] } else { vec![change("b.rs")] };
            t.edit(RunId(i), |r| r.files = Some(files));
        }
        assert_eq!(t.origin("a.rs").map(|r| r.id), Some(RunId(0)));
        assert_eq!(t.origin("b.rs").map(|r| r.id), Some(RunId(1)));
        assert_eq!(t.new_files(), 2);
        // Opening b.rs clears it in both runs; a later run changing it makes it new again.
        assert!(t.mark_seen("b.rs"));
        assert_eq!((t.new_files(), t.is_new("b.rs")), (1, false));
        t.observe(&[seen("w1:p1", Working, None)], 200);
        // A run still working doesn't count, and doesn't take over the file yet.
        t.edit(RunId(2), |r| r.files = Some(vec![change("b.rs")]));
        assert_eq!((t.new_files(), t.origin("b.rs").map(|r| r.id)), (1, Some(RunId(1))));
        t.observe(&[seen("w1:p1", Idle, None)], 201);
        assert_eq!((t.new_files(), t.origin("b.rs").map(|r| r.id)), (2, Some(RunId(2))));
        // Restored from disk, the index is rebuilt.
        let back = Tracker::from_runs(t.runs.clone());
        assert_eq!((back.new_files(), back.is_new("a.rs")), (2, true));
        // Reopening the run (a quick follow-up) clears its files: b.rs falls back to run 1.
        t.observe(&[seen("w1:p1", Working, None)], 205);
        assert_eq!((t.new_files(), t.origin("b.rs").map(|r| r.id)), (1, Some(RunId(1))));
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
        assert_eq!(back.observe(&[seen("w1:p1", Working, Some(6))], 30), vec![Effect::Before(RunId(1))]);
    }

    #[test]
    fn a_different_agent_in_a_restored_pane_closes_the_old_run() {
        let mut t = Tracker::default();
        t.observe(&[seen("w1:p1", Working, None)], 1);
        let mut back = Tracker::from_runs(t.runs.clone());
        let mut other = seen("w1:p1", Working, None);
        other.kind = Some("kiro".into());
        assert_eq!(back.observe(&[other], 5), vec![Effect::After(RunId(0)), Effect::Before(RunId(1))]);
    }

    #[test]
    fn keeps_at_most_max_runs_dropping_empty_ones_first() {
        let mut t = Tracker::default();
        let change = |p: &str| Change::new(p, 'M');
        for i in 0..(MAX_RUNS as u64 + 5) {
            t.observe(&[seen("w1:p1", Working, None)], i * 100);
            t.observe(&[seen("w1:p1", Idle, None)], i * 100 + 1);
            // Only run 3 never changed anything.
            t.edit(RunId(i), |r| r.files = Some(if i == 3 { vec![] } else { vec![change("a.rs")] }));
        }
        assert_eq!(t.runs.len(), MAX_RUNS);
        assert!(t.get(RunId(3)).is_none(), "run without changes dropped first");
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
