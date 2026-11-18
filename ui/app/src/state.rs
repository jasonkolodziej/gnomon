//! App state: the latest server snapshot, the event feed, and what the user
//! is doing (tab, selection, form, lab).

use gnomon_web_model::{ClusterView, EventView, JobView, LabReport, LabRequest};
use rune::TextEdit;
use std::collections::VecDeque;

/// Events kept for the feed.
pub const MAX_EVENTS: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Timeline,
    Nodes,
    Queue,
    Cluster,
    Policy,
    Events,
    Lab,
}

impl Tab {
    pub const ALL: [Tab; 7] = [
        Tab::Timeline,
        Tab::Nodes,
        Tab::Queue,
        Tab::Cluster,
        Tab::Policy,
        Tab::Events,
        Tab::Lab,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tab::Timeline => "timeline",
            Tab::Nodes => "nodes",
            Tab::Queue => "queue",
            Tab::Cluster => "cluster",
            Tab::Policy => "policy",
            Tab::Events => "events",
            Tab::Lab => "lab",
        }
    }
}

/// The new-job form.
pub struct Form {
    pub name: TextEdit,
    pub user: TextEdit,
    pub cores: TextEdit,
    pub nodes: TextEdit,
    pub walltime: TextEdit,
    pub command: TextEdit,
    pub container: TextEdit,
}

impl Default for Form {
    fn default() -> Self {
        Form {
            name: TextEdit::new("web-demo"),
            user: TextEdit::new("demo"),
            cores: TextEdit::new("2"),
            nodes: TextEdit::new("1"),
            walltime: TextEdit::new("300"),
            command: TextEdit::new("sleep 60"),
            container: TextEdit::new(""),
        }
    }
}

pub struct State {
    pub view: Option<ClusterView>,
    pub events: VecDeque<EventView>,
    pub tab: Tab,
    pub selected: Option<u64>,
    /// Timeline width in seconds.
    pub window: u64,
    pub hide_noise: bool,
    pub form: Form,
    /// Last action result: (message, ok).
    pub toast: Option<(String, bool)>,
    pub lab: LabRequest,
    pub lab_report: Option<LabReport>,
    pub lab_running: bool,
    /// The SSE stream is open.
    pub live: bool,
    /// Showing built-in sample data (`?demo`, or no server).
    pub demo: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            view: None,
            events: VecDeque::new(),
            tab: Tab::Timeline,
            selected: None,
            window: 3600,
            hide_noise: true,
            form: Form::default(),
            toast: None,
            lab: LabRequest::default(),
            lab_report: None,
            lab_running: false,
            live: false,
            demo: false,
        }
    }
}

impl State {
    pub fn apply_view(&mut self, v: ClusterView) {
        self.view = Some(v);
    }

    /// The server's retained history, sent first on every (re)connect: it
    /// replaces the feed, so a history that restarted from a lower index is
    /// not hidden behind the old high-water mark.
    pub fn replace_events(&mut self, events: Vec<EventView>) {
        self.events.clear();
        self.push_events(events);
    }

    /// The controller's history changed: the next events start a new feed.
    pub fn reset_events(&mut self) {
        self.events.clear();
    }

    pub fn push_events(&mut self, events: Vec<EventView>) {
        let last = self.events.back().map(|e| e.index);
        for e in events {
            // The stream replays recent events on reconnect: keep each index once.
            if last.is_some_and(|l| e.index <= l) {
                continue;
            }
            if self.events.len() == MAX_EVENTS {
                self.events.pop_front();
            }
            self.events.push_back(e);
        }
    }

    pub fn now(&self) -> u64 {
        self.view.as_ref().map_or(0, |v| v.now)
    }

    pub fn jobs(&self) -> &[JobView] {
        self.view.as_ref().map_or(&[], |v| v.jobs.as_slice())
    }

    pub fn job(&self, id: u64) -> Option<&JobView> {
        self.jobs().iter().find(|j| j.id == id)
    }

    pub fn writable(&self) -> bool {
        self.view
            .as_ref()
            .is_some_and(|v| v.writable && v.connected)
            && !self.demo
    }

    /// Jobs in queue order: running, reserved, pending, then finished (newest first).
    pub fn queue(&self) -> Vec<&JobView> {
        let rank = |s: &str| match s {
            "RUNNING" => 0,
            "RESERVED" => 1,
            "PENDING" => 2,
            _ => 3,
        };
        let mut v: Vec<&JobView> = self.jobs().iter().collect();
        v.sort_by(|a, b| {
            rank(&a.state)
                .cmp(&rank(&b.state))
                .then(if rank(&a.state) == 3 {
                    b.id.cmp(&a.id)
                } else {
                    b.priority.cmp(&a.priority).then(a.id.cmp(&b.id))
                })
        });
        v
    }

    /// Move the queue selection by `delta` rows.
    pub fn select_step(&mut self, delta: i64) {
        let ids: Vec<u64> = self.queue().iter().map(|j| j.id).collect();
        if ids.is_empty() {
            return;
        }
        let at = self
            .selected
            .and_then(|s| ids.iter().position(|&i| i == s))
            .map_or(0, |p| {
                (p as i64 + delta).clamp(0, ids.len() as i64 - 1) as usize
            });
        self.selected = Some(ids[at]);
    }

    /// Cores in use on `node`, from the running jobs placed there.
    pub fn used_cores(&self, node: &str) -> u32 {
        self.jobs()
            .iter()
            .filter(|j| j.state == "RUNNING" && j.nodes.iter().any(|n| n == node))
            .map(|j| j.cores)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: u64, state: &str, prio: u64) -> JobView {
        JobView {
            id,
            state: state.into(),
            priority: prio,
            ..Default::default()
        }
    }

    #[test]
    fn queue_order_is_running_reserved_pending_then_newest_finished() {
        let mut s = State::default();
        s.view = Some(ClusterView {
            jobs: vec![
                job(1, "COMPLETED", 0),
                job(2, "PENDING", 5),
                job(3, "RUNNING", 0),
                job(4, "PENDING", 9),
                job(5, "FAILED", 0),
                job(6, "RESERVED", 0),
            ],
            ..Default::default()
        });
        let ids: Vec<u64> = s.queue().iter().map(|j| j.id).collect();
        assert_eq!(ids, [3, 6, 4, 2, 5, 1]);
        s.select_step(1);
        assert_eq!(s.selected, Some(3), "first step selects the top row");
        s.select_step(2);
        assert_eq!(s.selected, Some(4));
        s.select_step(-10);
        assert_eq!(s.selected, Some(3), "clamped");
    }

    #[test]
    fn replayed_events_are_kept_once() {
        let mut s = State::default();
        let ev = |i| EventView {
            index: i,
            ..Default::default()
        };
        s.push_events(vec![ev(1), ev(2)]);
        s.push_events(vec![ev(2), ev(3)]);
        let idx: Vec<u64> = s.events.iter().map(|e| e.index).collect();
        assert_eq!(idx, [1, 2, 3]);
    }

    #[test]
    fn a_restarted_history_is_not_hidden_by_the_old_high_water_mark() {
        let mut s = State::default();
        let ev = |i| EventView {
            index: i,
            ..Default::default()
        };
        s.push_events(vec![ev(9_000), ev(10_000)]);
        s.push_events(vec![ev(5)]);
        assert_eq!(s.events.len(), 2, "live events keep the high-water mark");
        s.reset_events();
        s.push_events(vec![ev(5)]);
        assert_eq!(s.events.back().map(|e| e.index), Some(5));
        s.replace_events(vec![ev(1), ev(2)]);
        assert_eq!(s.events.iter().map(|e| e.index).collect::<Vec<_>>(), [1, 2]);
    }
}
