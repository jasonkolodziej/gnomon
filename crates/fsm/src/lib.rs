//! The replicated state machine (report section 4.1).
//!
//! All cluster state lives here. Mutations arrive as ordered [`Event`]s —
//! in production, committed entries from an embedded Raft log (`openraft`);
//! this crate is deliberately Raft-agnostic so the consensus crate can be
//! swapped without touching state logic. Reads are served from immutable
//! snapshots, so the API tier and the scheduler workers never take a lock
//! that a writer holds — the structural fix for Slurm's `slurmctld` mutex
//! contention.
//!
//! Time comes from an injected [`Clock`], and every applied event can be
//! journaled, so the state is a deterministic function of (initial state,
//! event log): the basis for exact replay (`gnomon-sim`).

use gnomon_core::{
    Assignment, Clock, Event, Job, JobId, JobState, NodeInfo, Resources, StateIndex, SystemClock,
};
use gnomon_graph::{ResourceGraph, VertexId, VertexKind};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// The committed state. Cloned (cheaply, at skeleton scale) into snapshots;
/// a production version uses persistent/im-style data structures.
///
/// Serializable so a durable store can persist a full state image (Raft
/// snapshot / journal compaction) and [`Fsm::restore`] it on restart.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    pub jobs: BTreeMap<JobId, Job>,
    pub graph: ResourceGraph,
    /// Live node agents, keyed by name (phase 4).
    pub nodes: BTreeMap<String, NodeInfo>,
    /// Work queued per node for FetchWork (phase 4).
    pub assignments: BTreeMap<String, Vec<Assignment>>,
    /// Node names each placed job holds, in rank order (rank `i` runs on
    /// `allocations[job][i]`). Kept after completion for job listings;
    /// dropped when a reservation is cleared or cancelled.
    pub allocations: BTreeMap<JobId, Vec<String>>,
    /// Per-rank exit reports of running gang jobs, `rank -> (end, code)`,
    /// aggregated into the job's completion once every rank has reported.
    pub rank_reports: BTreeMap<JobId, BTreeMap<u32, (u64, i32)>>,
    next_job_id: u64,
}

impl State {
    pub fn pending_jobs(&self) -> impl Iterator<Item = &Job> {
        self.jobs.values().filter(|j| j.state == JobState::Pending)
    }
}

/// Immutable point-in-time view handed to scheduler workers and the API.
#[derive(Clone)]
pub struct Snapshot {
    pub index: StateIndex,
    /// Clock reading when the snapshot was taken: plans are pure functions
    /// of (state, now), which is what makes them replayable.
    pub now: u64,
    inner: Arc<State>,
}

impl Snapshot {
    pub fn state(&self) -> &State {
        &self.inner
    }
    pub fn job(&self, id: JobId) -> Option<&Job> {
        self.inner.jobs.get(&id)
    }
    pub fn pending_jobs(&self) -> impl Iterator<Item = &Job> {
        self.inner.pending_jobs()
    }
    pub fn graph(&self) -> &ResourceGraph {
        &self.inner.graph
    }
    pub fn node(&self, name: &str) -> Option<&NodeInfo> {
        self.inner.nodes.get(name)
    }
    pub fn nodes(&self) -> impl Iterator<Item = &NodeInfo> {
        self.inner.nodes.values()
    }
}

/// One journaled event with the clock reading it was applied at — enough to
/// reproduce state exactly (`gnomon-sim` replays these).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct JournalEntry {
    pub at: u64,
    pub event: Event,
}

/// Write-ahead hook: receives every event before it is applied, under the
/// FSM's write lock, so records arrive in exact log order. A durable
/// implementation (the controller's on-disk journal in `gnomon-consensus`)
/// persists the record before returning; state never runs ahead of the log.
pub trait EventSink: Send + Sync {
    /// Record the event that will become state index `index`.
    fn record(&self, index: StateIndex, entry: &JournalEntry);
}

/// Result of applying one event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Applied {
    pub index: StateIndex,
    /// False when a start/reserve was rejected (job no longer pending, or
    /// commit-time revalidation failed). The event is still logged — Raft
    /// entries are immutable — but it had no effect on state.
    pub accepted: bool,
}

pub struct Fsm {
    state: RwLock<State>,
    index: AtomicU64,
    clock: Arc<dyn Clock>,
    journal: Option<Mutex<Vec<JournalEntry>>>,
    sink: OnceLock<Arc<dyn EventSink>>,
}

impl Default for Fsm {
    fn default() -> Self {
        Self::new()
    }
}

impl Fsm {
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Fsm {
            state: RwLock::new(State::default()),
            index: AtomicU64::new(0),
            clock,
            journal: None,
            sink: OnceLock::new(),
        }
    }

    /// Like [`Fsm::with_clock`], but records every applied event in order.
    pub fn recording(clock: Arc<dyn Clock>) -> Self {
        Fsm {
            journal: Some(Mutex::new(Vec::new())),
            ..Self::with_clock(clock)
        }
    }

    /// Attach a write-ahead [`EventSink`] (once; later calls are rejected
    /// and return the sink back).
    pub fn set_sink(&self, sink: Arc<dyn EventSink>) -> Result<(), Arc<dyn EventSink>> {
        self.sink.set(sink)
    }

    /// Replace the whole state and index with a persisted image (Raft
    /// snapshot install / restart recovery). The image is the exact state a
    /// previous [`Fsm::snapshot`] captured, so a state fingerprint (the
    /// `Debug` rendering of [`State`]) is preserved.
    pub fn restore(&self, state: State, index: StateIndex) {
        let mut st = self.state.write().unwrap();
        *st = state;
        self.index.store(index, Ordering::Release);
    }

    pub fn now(&self) -> u64 {
        self.clock.now()
    }
    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    /// The journal so far (empty unless created with [`Fsm::recording`]).
    pub fn journal(&self) -> Vec<JournalEntry> {
        self.journal
            .as_ref()
            .map(|j| j.lock().unwrap().clone())
            .unwrap_or_default()
    }

    /// Current Raft-style index.
    pub fn index(&self) -> StateIndex {
        self.index.load(Ordering::Acquire)
    }

    /// Immutable snapshot; the read path for API handlers and workers.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            index: self.index(),
            now: self.now(),
            inner: Arc::new(self.state.read().unwrap().clone()),
        }
    }

    /// Borrow live state under a read lock without cloning (single-writer
    /// loops such as the scheduling pass and the simulator).
    pub fn read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        f(&self.state.read().unwrap())
    }

    /// Apply one event in log order. Returns the new index.
    pub fn apply(&self, ev: Event) -> StateIndex {
        self.apply_with_expect(ev, &[]).index
    }

    pub fn apply_checked(&self, ev: Event) -> Applied {
        self.apply_with_expect(ev, &[])
    }

    /// Apply with the plan's per-vertex read set (leader-side fast path; the
    /// journaled event is identical, followers simply revalidate).
    pub fn apply_with_expect(&self, ev: Event, expect: &[(VertexId, u64)]) -> Applied {
        self.apply_inner(ev, expect, None)
    }

    /// Apply one event as if the clock read `now`: the deterministic path
    /// for replicated logs and recovery, where every replica (and every
    /// restart) must see the timestamp the leader stamped, not its own clock.
    pub fn apply_at(&self, ev: Event, now: u64) -> Applied {
        self.apply_inner(ev, &[], Some(now))
    }

    fn apply_inner(&self, ev: Event, expect: &[(VertexId, u64)], at: Option<u64>) -> Applied {
        let now = at.unwrap_or_else(|| self.now());
        let mut st = self.state.write().unwrap();
        if let Some(sink) = self.sink.get() {
            let entry = JournalEntry {
                at: now,
                event: ev.clone(),
            };
            sink.record(self.index() + 1, &entry);
        }
        if let Some(j) = &self.journal {
            j.lock().unwrap().push(JournalEntry {
                at: now,
                event: ev.clone(),
            });
        }
        let accepted = apply_event(&mut st, &ev, expect, now);
        let index = self.index.fetch_add(1, Ordering::AcqRel) + 1;
        Applied { index, accepted }
    }

    pub fn next_job_id(&self) -> JobId {
        JobId(self.state.read().unwrap().next_job_id)
    }
}

#[allow(clippy::too_many_arguments)]
fn place(
    st: &mut State,
    job: JobId,
    vertices: &[VertexId],
    start: u64,
    dur: Option<u64>,
    expect: &[(VertexId, u64)],
    now: u64,
    reserve_only: bool,
) -> bool {
    let (req, d, nodes) = match st.jobs.get(&job) {
        Some(j) if j.state == JobState::Pending => {
            (j.request, dur.unwrap_or(j.walltime.0.as_secs()), j.nodes)
        }
        _ => return false, // already placed / cancelled / unknown: reject
    };
    // A gang must get exactly `nodes` vertices (distinctness is checked by
    // `commit_job`); single-node jobs keep their historical behaviour.
    if nodes > 1 && vertices.len() != nodes as usize {
        return false;
    }
    if !st
        .graph
        .commit_job(vertices, &req, start, d, Some(job.0), expect)
    {
        return false;
    }
    let running = !reserve_only && start <= now;
    let j = st.jobs.get_mut(&job).unwrap();
    j.state = if running {
        JobState::Running
    } else {
        JobState::Reserved
    };
    j.planned_start = Some(start);
    j.planned_dur = Some(d);
    let cmd = j.command.clone();
    let nodelist: Vec<String> = vertices
        .iter()
        .filter_map(|&v| st.graph.vertices.get(v as usize))
        .map(|vx| vx.name.clone())
        .collect();
    if running {
        // Phase 4: enqueue executable work for the owning agents, one
        // assignment per node, ranked in vertex order.
        for (rank, node) in nodelist.iter().enumerate() {
            st.assignments
                .entry(node.clone())
                .or_default()
                .push(Assignment {
                    job,
                    command: cmd.clone(),
                    start,
                    rank: rank as u32,
                    nodelist: nodelist.clone(),
                });
        }
    }
    st.allocations.insert(job, nodelist);
    true
}

/// Aggregate one gang rank's exit. The job completes when every rank has
/// reported: its spans are released at the latest rank's end and it is
/// `Failed` if any rank exited non-zero. Peers are *not* cancelled on the
/// first failure: agents have no kill channel, so the nodes stay booked
/// until each rank has really exited (walltime still bounds them), which
/// keeps the planner consistent with what runs on the hardware.
/// Rejected (no effect) for jobs that are not running, nodes outside the
/// job's allocation, and repeated reports from the same rank.
fn rank_finished(st: &mut State, job: JobId, node: &str, end: u64, code: i32) -> bool {
    if st.jobs.get(&job).map(|j| j.state) != Some(JobState::Running) {
        return false;
    }
    let Some(alloc) = st.allocations.get(&job) else {
        return false;
    };
    let Some(rank) = alloc.iter().position(|n| n == node) else {
        return false;
    };
    let ranks = alloc.len();
    let reports = st.rank_reports.entry(job).or_default();
    if reports.contains_key(&(rank as u32)) {
        return false;
    }
    reports.insert(rank as u32, (end, code));
    if reports.len() < ranks {
        return true;
    }
    let reports = st.rank_reports.remove(&job).unwrap_or_default();
    let last = reports.values().map(|r| r.0).max().unwrap_or(end);
    let failed = reports.values().any(|r| r.1 != 0);
    finish(st, job, last, if failed { 1 } else { 0 });
    true
}

fn finish(st: &mut State, job: JobId, end: u64, code: i32) {
    if let Some(j) = st.jobs.get_mut(&job) {
        j.state = if code == 0 {
            JobState::Completed
        } else {
            JobState::Failed
        };
    }
    st.rank_reports.remove(&job);
    st.graph.release_job(job.0, end);
}

fn apply_event(st: &mut State, ev: &Event, expect: &[(VertexId, u64)], now: u64) -> bool {
    match ev {
        Event::JobSubmitted(job) => {
            let mut job = job.clone();
            st.next_job_id = st.next_job_id.max(job.id.0 + 1);
            job.state = JobState::Pending;
            st.jobs.insert(job.id, job);
        }
        Event::JobCancelled(id) => {
            if let Some(j) = st.jobs.get_mut(id) {
                if matches!(j.state, JobState::Pending | JobState::Reserved) {
                    j.state = JobState::Cancelled;
                    st.graph.release_job(id.0, 0);
                    st.allocations.remove(id);
                }
            }
        }
        Event::JobStarted {
            job,
            vertices,
            start,
            dur,
        } => {
            return place(st, *job, vertices, *start, *dur, expect, now, false);
        }
        Event::JobReserved {
            job,
            vertices,
            start,
            dur,
        } => {
            return place(st, *job, vertices, *start, *dur, expect, now, true);
        }
        Event::JobFinished { job, end, code } => finish(st, *job, *end, *code),
        Event::JobRankFinished {
            job,
            node,
            end,
            code,
        } => return rank_finished(st, *job, node, *end, *code),
        Event::JobExtended { job, new_end } => {
            st.graph.extend_job(job.0, *new_end);
            if let Some(j) = st.jobs.get_mut(job) {
                if let Some(s) = j.planned_start {
                    j.planned_dur = Some(new_end.saturating_sub(s));
                }
            }
        }
        Event::ReservationsCleared { .. } => {
            let reserved: Vec<JobId> = st
                .jobs
                .values()
                .filter(|j| j.state == JobState::Reserved)
                .map(|j| j.id)
                .collect();
            for id in reserved {
                st.graph.release_job(id.0, 0);
                st.allocations.remove(&id);
                let j = st.jobs.get_mut(&id).unwrap();
                j.state = JobState::Pending;
                j.planned_start = None;
                j.planned_dur = None;
            }
        }
        Event::NodeRegistered { name, capacity } => {
            if st.graph.by_name(name).is_none() {
                let root = ensure_cluster_root(&mut st.graph);
                st.graph
                    .add_vertex(VertexKind::Node, name, *capacity, Some(root));
            }
            let info = st.nodes.entry(name.clone()).or_default();
            info.name = name.clone();
            info.capacity = *capacity;
            info.last_heartbeat = now;
        }
        Event::AssignmentsFetched { node, count } => {
            if let Some(q) = st.assignments.get_mut(node) {
                let n = (*count).min(q.len());
                q.drain(..n);
            }
        }
        Event::NodeProfileSet { name, profile } => match st.graph.by_name(name) {
            Some(id) => st.graph.set_profile(id, profile.clone()),
            None => return false,
        },
        Event::Heartbeat { name, allocated } => {
            if let Some(info) = st.nodes.get_mut(name) {
                info.last_heartbeat = now;
                info.allocated = *allocated;
            }
        }
    }
    true
}

fn ensure_cluster_root(g: &mut ResourceGraph) -> VertexId {
    g.by_name("cluster")
        .unwrap_or_else(|| g.add_vertex(VertexKind::Cluster, "cluster", Resources::default(), None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnomon_core::{ManualClock, Walltime};
    use std::time::Duration;

    fn setup() -> (Arc<ManualClock>, Fsm) {
        let clock = Arc::new(ManualClock::new(1000));
        let fsm = Fsm::recording(clock.clone());
        fsm.apply(Event::NodeRegistered {
            name: "n0".into(),
            capacity: Resources {
                cores: 4,
                mem_mb: 0,
                gpus: 0,
            },
        });
        (clock, fsm)
    }

    fn job(id: u64, cores: u32, wall: u64) -> Job {
        Job::new(
            JobId(id),
            "j",
            "u",
            Resources {
                cores,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(Duration::from_secs(wall)),
        )
    }

    #[test]
    fn double_start_of_same_job_is_rejected() {
        let (_c, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(1, 1, 100)));
        let ev = Event::JobStarted {
            job: JobId(1),
            vertices: vec![1],
            start: 1000,
            dur: None,
        };
        assert!(fsm.apply_checked(ev.clone()).accepted);
        assert!(
            !fsm.apply_checked(ev).accepted,
            "a job must never be placed twice"
        );
    }

    #[test]
    fn early_finish_frees_the_rest_of_the_window() {
        let (clock, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(1, 4, 1000)));
        fsm.apply(Event::JobStarted {
            job: JobId(1),
            vertices: vec![1],
            start: 1000,
            dur: None,
        });
        clock.set(1100);
        fsm.apply(Event::JobFinished {
            job: JobId(1),
            end: 1100,
            code: 0,
        });
        let fit = fsm.read(|s| {
            s.graph.vertex(1).earliest_fit(
                &Resources {
                    cores: 4,
                    mem_mb: 0,
                    gpus: 0,
                },
                1100,
                50,
            )
        });
        assert_eq!(fit, Some(1100));
    }

    #[test]
    fn journal_replays_to_identical_state() {
        let (clock, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(1, 2, 100)));
        fsm.apply(Event::JobSubmitted(job(2, 4, 100)));
        fsm.apply(Event::JobStarted {
            job: JobId(1),
            vertices: vec![1],
            start: 1000,
            dur: Some(50),
        });
        fsm.apply(Event::JobReserved {
            job: JobId(2),
            vertices: vec![1],
            start: 1050,
            dur: None,
        });
        clock.set(1040);
        fsm.apply(Event::JobFinished {
            job: JobId(1),
            end: 1040,
            code: 0,
        });
        fsm.apply(Event::ReservationsCleared { now: 1040 });

        // Replay the journal into a fresh FSM driven by the same clock values.
        let c2 = Arc::new(ManualClock::new(1000));
        let replica = Fsm::with_clock(c2.clone());
        for e in fsm.journal() {
            c2.set(e.at);
            replica.apply(e.event);
        }
        let a = fsm.read(|s| (s.jobs.clone(), format!("{:?}", s.graph)));
        let b = replica.read(|s| (s.jobs.clone(), format!("{:?}", s.graph)));
        assert_eq!(a, b);
    }

    #[test]
    fn apply_at_ignores_the_local_clock() {
        // Same event, applied at the stamped time by replicas whose clocks differ.
        let make = |clock_now| {
            let fsm = Fsm::with_clock(Arc::new(ManualClock::new(clock_now)));
            fsm.apply_at(
                Event::NodeRegistered {
                    name: "n0".into(),
                    capacity: Resources::default(),
                },
                1234,
            );
            fsm
        };
        let (a, b) = (make(1), make(999_999));
        assert_eq!(a.read(|s| format!("{s:?}")), b.read(|s| format!("{s:?}")));
        assert_eq!(a.snapshot().node("n0").unwrap().last_heartbeat, 1234);
    }

    #[test]
    fn restore_reproduces_a_serialized_state_image() {
        let (_c, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(7, 1, 100)));
        fsm.apply(Event::JobStarted {
            job: JobId(7),
            vertices: vec![1],
            start: 1000,
            dur: None,
        });
        let snap = fsm.snapshot();
        let bytes = serde_json::to_vec(snap.state()).unwrap();
        let other = Fsm::new();
        other.restore(serde_json::from_slice(&bytes).unwrap(), snap.index);
        assert_eq!(other.index(), fsm.index());
        assert_eq!(
            other.read(|s| format!("{s:?}")),
            fsm.read(|s| format!("{s:?}"))
        );
        // next_job_id travels with the image.
        assert_eq!(other.next_job_id(), JobId(8));
    }

    #[derive(Default)]
    struct VecSink(Mutex<Vec<(StateIndex, JournalEntry)>>);
    impl EventSink for VecSink {
        fn record(&self, index: StateIndex, entry: &JournalEntry) {
            self.0.lock().unwrap().push((index, entry.clone()));
        }
    }

    #[test]
    fn sink_sees_every_event_with_its_resulting_index() {
        let (clock, fsm) = setup();
        let sink = Arc::new(VecSink::default());
        assert!(fsm.set_sink(sink.clone()).is_ok());
        assert!(fsm.set_sink(sink.clone()).is_err());
        clock.set(1500);
        let i1 = fsm.apply(Event::JobSubmitted(job(1, 1, 10)));
        let i2 = fsm.apply_at(Event::JobCancelled(JobId(1)), 1600).index;
        let got = sink.0.lock().unwrap().clone();
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].0, got[0].1.at), (i1, 1500));
        assert_eq!((got[1].0, got[1].1.at), (i2, 1600));
    }

    /// Four 4-core nodes (vertices 1..=4, names n0..n3) and a submitted gang.
    fn gang_setup(nodes: u32) -> (Arc<ManualClock>, Fsm) {
        let clock = Arc::new(ManualClock::new(1000));
        let fsm = Fsm::recording(clock.clone());
        for n in 0..4 {
            fsm.apply(Event::NodeRegistered {
                name: format!("n{n}"),
                capacity: Resources {
                    cores: 4,
                    mem_mb: 0,
                    gpus: 0,
                },
            });
        }
        let mut j = job(1, 4, 100);
        j.nodes = nodes;
        j.command = "mpirun app".into();
        fsm.apply(Event::JobSubmitted(j));
        (clock, fsm)
    }

    fn start(job: u64, vertices: Vec<u32>) -> Event {
        Event::JobStarted {
            job: JobId(job),
            vertices,
            start: 1000,
            dur: None,
        }
    }

    fn rank_done(node: &str, end: u64, code: i32) -> Event {
        Event::JobRankFinished {
            job: JobId(1),
            node: node.into(),
            end,
            code,
        }
    }

    fn state_of(fsm: &Fsm, id: u64) -> JobState {
        fsm.read(|s| s.jobs[&JobId(id)].state)
    }

    #[test]
    fn gang_start_enqueues_one_ranked_assignment_per_node() {
        let (_c, fsm) = gang_setup(3);
        assert!(fsm.apply_checked(start(1, vec![2, 3, 4])).accepted);
        fsm.read(|s| {
            let list = vec!["n1".to_string(), "n2".into(), "n3".into()];
            assert_eq!(s.allocations[&JobId(1)], list);
            for (rank, node) in list.iter().enumerate() {
                let q = &s.assignments[node];
                assert_eq!(q.len(), 1);
                assert_eq!(q[0].rank, rank as u32);
                assert_eq!(q[0].nodelist, list);
                assert_eq!(q[0].command, "mpirun app");
                assert_eq!(q[0].start, 1000);
            }
            assert!(!s.assignments.contains_key("n0"));
            for v in 2..=4 {
                assert_eq!(s.graph.vertex(v).planner.job_span(1), Some((1000, 1100)));
            }
        });
    }

    #[test]
    fn gang_placement_with_wrong_vertex_count_or_duplicates_is_rejected() {
        let (_c, fsm) = gang_setup(3);
        assert!(!fsm.apply_checked(start(1, vec![1, 2])).accepted);
        assert!(!fsm.apply_checked(start(1, vec![1, 2, 3, 4])).accepted);
        assert!(!fsm.apply_checked(start(1, vec![1, 1, 2])).accepted);
        assert_eq!(state_of(&fsm, 1), JobState::Pending);
        // A conflict on one vertex rejects the whole gang.
        fsm.apply(Event::JobSubmitted(job(2, 4, 500)));
        assert!(fsm.apply_checked(start(2, vec![3])).accepted);
        assert!(!fsm.apply_checked(start(1, vec![1, 2, 3])).accepted);
        fsm.read(|s| {
            assert!(s.graph.vertex(1).planner.job_span(1).is_none());
            assert!(s.graph.vertex(2).planner.job_span(1).is_none());
            assert!(!s.allocations.contains_key(&JobId(1)));
        });
        assert!(fsm.apply_checked(start(1, vec![1, 2, 4])).accepted);
    }

    #[test]
    fn gang_completes_only_when_every_rank_reports() {
        let (clock, fsm) = gang_setup(3);
        fsm.apply(start(1, vec![1, 2, 3]));
        clock.set(1040);
        assert!(fsm.apply_checked(rank_done("n2", 1040, 0)).accepted);
        assert!(fsm.apply_checked(rank_done("n0", 1050, 0)).accepted);
        assert_eq!(state_of(&fsm, 1), JobState::Running);
        // Repeated report from a rank and reports from foreign nodes: ignored.
        assert!(!fsm.apply_checked(rank_done("n2", 1060, 1)).accepted);
        assert!(!fsm.apply_checked(rank_done("n3", 1060, 1)).accepted);
        // Spans stay booked on every node until the last rank exits.
        fsm.read(|s| assert_eq!(s.graph.vertex(3).planner.job_span(1), Some((1000, 1100))));
        assert!(fsm.apply_checked(rank_done("n1", 1070, 0)).accepted);
        assert_eq!(state_of(&fsm, 1), JobState::Completed);
        fsm.read(|s| {
            assert!(s.rank_reports.is_empty());
            for v in 1..=3 {
                // Released at the latest rank's end.
                assert_eq!(s.graph.vertex(v).planner.job_span(1), Some((1000, 1070)));
            }
            assert_eq!(s.allocations[&JobId(1)].len(), 3, "kept for listings");
        });
        // Reports after completion have no effect.
        assert!(!fsm.apply_checked(rank_done("n0", 1080, 0)).accepted);
    }

    #[test]
    fn gang_fails_if_any_rank_fails() {
        let (_c, fsm) = gang_setup(2);
        fsm.apply(start(1, vec![1, 2]));
        fsm.apply(rank_done("n0", 1010, 3));
        assert_eq!(state_of(&fsm, 1), JobState::Running, "peers keep running");
        fsm.apply(rank_done("n1", 1020, 0));
        assert_eq!(state_of(&fsm, 1), JobState::Failed);
    }

    #[test]
    fn job_finished_still_ends_a_gang_at_once() {
        let (_c, fsm) = gang_setup(2);
        fsm.apply(start(1, vec![1, 2]));
        fsm.apply(rank_done("n0", 1010, 0));
        fsm.apply(Event::JobFinished {
            job: JobId(1),
            end: 1020,
            code: 0,
        });
        assert_eq!(state_of(&fsm, 1), JobState::Completed);
        fsm.read(|s| assert!(s.rank_reports.is_empty()));
    }

    #[test]
    fn single_node_rank_report_equals_job_finished() {
        let run = |ev: Event| {
            let (_c, fsm) = setup();
            fsm.apply(Event::JobSubmitted(job(1, 2, 100)));
            fsm.apply(Event::JobStarted {
                job: JobId(1),
                vertices: vec![1],
                start: 1000,
                dur: None,
            });
            fsm.apply(ev);
            fsm.read(|s| (s.jobs.clone(), format!("{:?}", s.graph)))
        };
        for code in [0, 2] {
            let a = run(Event::JobFinished {
                job: JobId(1),
                end: 1030,
                code,
            });
            let b = run(Event::JobRankFinished {
                job: JobId(1),
                node: "n0".into(),
                end: 1030,
                code,
            });
            assert_eq!(a, b);
        }
    }

    #[test]
    fn reservation_clear_and_cancel_drop_gang_allocations() {
        let (_c, fsm) = gang_setup(2);
        assert!(
            fsm.apply_checked(Event::JobReserved {
                job: JobId(1),
                vertices: vec![1, 2],
                start: 2000,
                dur: None,
            })
            .accepted
        );
        fsm.read(|s| {
            assert_eq!(s.allocations[&JobId(1)].len(), 2);
            assert!(s.assignments.is_empty(), "no work before the gang starts");
        });
        fsm.apply(Event::ReservationsCleared { now: 1000 });
        fsm.read(|s| assert!(s.allocations.is_empty()));
        fsm.apply(Event::JobReserved {
            job: JobId(1),
            vertices: vec![3, 4],
            start: 2000,
            dur: None,
        });
        fsm.apply(Event::JobCancelled(JobId(1)));
        fsm.read(|s| {
            assert!(s.allocations.is_empty());
            assert!(s.graph.vertex(3).planner.job_span(1).is_none());
        });
    }

    #[test]
    fn gang_journal_replays_to_identical_state() {
        let (clock, fsm) = gang_setup(3);
        fsm.apply(start(1, vec![1, 2, 4]));
        clock.set(1030);
        fsm.apply(rank_done("n3", 1030, 0));
        fsm.apply(rank_done("n0", 1030, 1));
        let mid = fsm.read(|s| format!("{s:?}"));
        clock.set(1050);
        fsm.apply(rank_done("n1", 1050, 0));
        let c2 = Arc::new(ManualClock::new(1000));
        let replica = Fsm::with_clock(c2.clone());
        let journal = fsm.journal();
        for (i, e) in journal.iter().enumerate() {
            c2.set(e.at);
            replica.apply(e.event.clone());
            if i + 2 == journal.len() {
                // Partial aggregation state replays exactly too.
                assert_eq!(replica.read(|s| format!("{s:?}")), mid);
            }
        }
        assert_eq!(
            fsm.read(|s| format!("{s:?}")),
            replica.read(|s| format!("{s:?}"))
        );
        assert_eq!(state_of(&replica, 1), JobState::Failed);
    }
}
