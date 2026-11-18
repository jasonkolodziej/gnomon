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
    NODE_LOST_EXIT,
};
use gnomon_graph::{ResourceGraph, VertexId, VertexKind};
use std::collections::{BTreeMap, BTreeSet};
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
    /// Per-rank exit reports of running gang jobs, `rank -> (end, code)`,
    /// aggregated into the job's completion once every rank has reported.
    pub rank_reports: BTreeMap<JobId, BTreeMap<u32, (u64, i32)>>,
    /// Ranks of each running gang whose assignment an agent has fetched
    /// (from replicated `AssignmentsFetched` events, so replay is exact).
    /// The gang's release barrier opens once every rank is in here.
    #[serde(default)]
    pub gang_fetched: BTreeMap<JobId, BTreeSet<u32>>,
    /// Kill channel: per node, the running jobs whose rank on that node must
    /// be killed (a gang peer failed, or the job was cancelled while
    /// running). Agents receive these with `FetchWork`; an entry is cleared
    /// when that node's rank reports its exit, so delivery is retried until
    /// the agent acts on it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kills: BTreeMap<String, BTreeSet<JobId>>,
    /// Running jobs cancelled by the user: they end `Cancelled` (not
    /// `Failed`) once every rank has reported.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub cancelling: BTreeSet<JobId>,
    /// Ranks ended by [`Event::NodeLost`] that may still run on that node
    /// (the agent was only partitioned away). Delivered on the node's kill
    /// list so a returning agent stops them; cleared by the late report
    /// the agent then sends.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lost: BTreeMap<String, BTreeSet<JobId>>,
    next_job_id: u64,
}

/// Whether a placed job's ranks may start (see [`State::gang_release`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GangRelease {
    /// Every rank has been fetched (always the case for single-node jobs).
    Released,
    /// Still waiting for the remaining ranks to be fetched.
    Waiting { fetched: u32, nodes: u32 },
    /// The job is not running (finished, failed, cancelled or unknown).
    NotRunning,
}

impl State {
    pub fn pending_jobs(&self) -> impl Iterator<Item = &Job> {
        self.jobs.values().filter(|j| j.state == JobState::Pending)
    }

    /// Release barrier for a running job's ranks: a gang is released once
    /// every rank's assignment has been fetched, so ranks start together
    /// instead of one agent poll interval apart.
    pub fn gang_release(&self, job: JobId) -> GangRelease {
        let Some(j) = self.jobs.get(&job).filter(|j| j.state == JobState::Running) else {
            return GangRelease::NotRunning;
        };
        let nodes = j.node_count();
        let fetched = self.gang_fetched.get(&job).map_or(0, |r| r.len() as u32);
        if nodes <= 1 || fetched >= nodes {
            GangRelease::Released
        } else {
            GangRelease::Waiting { fetched, nodes }
        }
    }

    /// Jobs whose rank on `node` must be killed (see [`State::kills`]).
    pub fn kills_for(&self, node: &str) -> impl Iterator<Item = JobId> + '_ {
        let lost = self.lost.get(node).into_iter().flatten();
        self.kills
            .get(node)
            .into_iter()
            .flatten()
            .chain(lost)
            .copied()
    }

    /// Nodes that are up but have not heartbeated for more than `timeout`
    /// seconds at `now`. Time before `since` (when this controller started
    /// leading) does not count, so a controller restart or failover does not
    /// declare every node lost at once.
    /// Each with the `last_heartbeat` observed, which [`Event::NodeLost`]
    /// carries so a heartbeat committed meanwhile invalidates it.
    pub fn stale_nodes(&self, now: u64, timeout: u64, since: u64) -> Vec<(String, u64)> {
        self.nodes
            .values()
            .filter(|n| !n.down && now.saturating_sub(n.last_heartbeat.max(since)) > timeout)
            .map(|n| (n.name.clone(), n.last_heartbeat))
            .collect()
    }

    /// Whether `node`'s rank of `job` has been told to die: its queued
    /// assignment must not be launched.
    pub fn is_killed(&self, job: JobId, node: &str) -> bool {
        self.kills.get(node).is_some_and(|k| k.contains(&job))
    }

    /// Names of the nodes `job` holds, in rank order (empty until placed).
    pub fn job_nodes(&self, job: &Job) -> Vec<String> {
        match (&job.allocation, job.placed_on) {
            (Some(gang), _) => gang.to_vec(),
            (None, Some(v)) => self
                .graph
                .vertices
                .get(v as usize)
                .map(|vx| vec![vx.name.clone()])
                .unwrap_or_default(),
            (None, None) => Vec::new(),
        }
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

/// Routes writes through consensus. With a proposer attached
/// ([`Fsm::set_proposer`]), every `apply*` call that is not itself the
/// committed-entry path ([`Fsm::apply_at`], [`Fsm::apply_at_expect`])
/// proposes the event instead (`client_write` on the Raft leader, forwarded
/// from followers) and returns once it has been committed and applied
/// locally. The call sites (API handlers, plan applier, backfill pass) stay
/// unchanged; the event and its per-vertex read set travel in the log, so
/// every replica revalidates the plan identically.
pub trait Proposer: Send + Sync {
    fn propose(&self, ev: Event, expect: &[(VertexId, u64)]) -> Result<Applied, ProposeError>;
}

/// A write that could not be committed (no leader, lost leadership,
/// timeout). The event may or may not take effect later; callers report it
/// as unavailable rather than as rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposeError(pub String);

impl std::fmt::Display for ProposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProposeError {}

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
    proposer: OnceLock<Arc<dyn Proposer>>,
    tail: OnceLock<Mutex<Tail>>,
}

/// One applied event as kept by the in-memory tail ([`Fsm::enable_tail`]).
#[derive(Clone, Debug, PartialEq)]
pub struct TailEntry {
    pub index: StateIndex,
    pub entry: JournalEntry,
    pub accepted: bool,
}

/// The last `cap` applied events, for live views (the API's `ListEvents`).
#[derive(Debug)]
struct Tail {
    cap: usize,
    entries: std::collections::VecDeque<TailEntry>,
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
            proposer: OnceLock::new(),
            tail: OnceLock::new(),
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

    /// Route every later write through `proposer` (once; later calls are
    /// rejected and return it back). See [`Proposer`].
    pub fn set_proposer(&self, proposer: Arc<dyn Proposer>) -> Result<(), Arc<dyn Proposer>> {
        self.proposer.set(proposer)
    }

    /// Keep the last `cap` applied events in memory for live views
    /// ([`Fsm::tail_after`]). Off by default, so the simulator and replay pay
    /// nothing; the controller turns it on. Returns false if already on.
    pub fn enable_tail(&self, cap: usize) -> bool {
        self.tail
            .set(Mutex::new(Tail {
                cap: cap.max(1),
                entries: std::collections::VecDeque::new(),
            }))
            .is_ok()
    }

    /// Up to `limit` tail entries with an index above `after`, oldest first,
    /// and the oldest index the tail still holds (`None` when the tail is
    /// off or empty).
    pub fn tail_after(
        &self,
        after: StateIndex,
        limit: usize,
    ) -> (Vec<TailEntry>, Option<StateIndex>) {
        let Some(t) = self.tail.get() else {
            return (Vec::new(), None);
        };
        let t = t.lock().unwrap();
        let first = t.entries.front().map(|e| e.index);
        let start = t.entries.partition_point(|e| e.index <= after);
        let out = t.entries.iter().skip(start).take(limit).cloned().collect();
        (out, first)
    }

    /// Whether writes go through a [`Proposer`] (replicated controller).
    pub fn is_replicated(&self) -> bool {
        self.proposer.get().is_some()
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

    /// Apply with the plan's per-vertex read set. Replicated, the read set
    /// is carried in the log entry and every replica revalidates it.
    ///
    /// A write the [`Proposer`] could not commit is logged and reported as
    /// not accepted; use [`Fsm::try_apply`] to tell the two apart.
    pub fn apply_with_expect(&self, ev: Event, expect: &[(VertexId, u64)]) -> Applied {
        self.try_apply_with_expect(ev, expect).unwrap_or_else(|e| {
            eprintln!("gnomon: write not committed: {e}");
            Applied {
                index: self.index(),
                accepted: false,
            }
        })
    }

    /// Apply (or, replicated, propose) one event; `Err` when it could not be
    /// committed.
    pub fn try_apply(&self, ev: Event) -> Result<Applied, ProposeError> {
        self.try_apply_with_expect(ev, &[])
    }

    pub fn try_apply_with_expect(
        &self,
        ev: Event,
        expect: &[(VertexId, u64)],
    ) -> Result<Applied, ProposeError> {
        match self.proposer.get() {
            Some(p) => p.propose(ev, expect),
            None => Ok(self.apply_inner(ev, expect, None)),
        }
    }

    /// Apply one event as if the clock read `now`: the deterministic path
    /// for replicated logs and recovery, where every replica (and every
    /// restart) must see the timestamp the leader stamped, not its own clock.
    /// Never goes through a [`Proposer`]: this *is* the committed path.
    pub fn apply_at(&self, ev: Event, now: u64) -> Applied {
        self.apply_inner(ev, &[], Some(now))
    }

    /// [`Fsm::apply_at`] with a plan's per-vertex read set (a replicated
    /// plan commit).
    pub fn apply_at_expect(&self, ev: Event, now: u64, expect: &[(VertexId, u64)]) -> Applied {
        self.apply_inner(ev, expect, Some(now))
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
        if let Some(t) = self.tail.get() {
            // Under the state write lock, so the tail is in index order.
            let mut t = t.lock().unwrap();
            if t.entries.len() == t.cap {
                t.entries.pop_front();
            }
            t.entries.push_back(TailEntry {
                index,
                entry: JournalEntry { at: now, event: ev },
                accepted,
            });
        }
        Applied { index, accepted }
    }

    pub fn next_job_id(&self) -> JobId {
        JobId(self.state.read().unwrap().next_job_id)
    }
}

/// Whether starting `req` on `vertices` at `start` for `d` seconds leaves
/// room for the running jobs there that have outlived their planned span.
///
/// With runtime estimates, a running job's booking can end while the job is
/// still running, until the backfill pass extends it (`JobExtended`). The
/// graph then shows its resources as free, and the per-vertex version check
/// cannot notice, since nothing changed. A start in that window would put
/// two jobs on resources only one fits. So the overrunning jobs count as
/// using their request for the whole new span: the extension is at least
/// that long, up to their walltime. Reservations need no check: they are
/// re-planned every pass, after the extensions.
fn clear_of_overruns(
    st: &State,
    vertices: &[VertexId],
    req: &Resources,
    start: u64,
    d: u64,
) -> bool {
    let mut extra: Vec<Resources> = vec![Resources::default(); vertices.len()];
    let mut any = false;
    for j in st.jobs.values().filter(|j| j.state == JobState::Running) {
        let Some(end) = j.planned_start.zip(j.planned_dur).map(|(s, d)| s + d) else {
            continue;
        };
        if end > start {
            continue; // still booked at `start`: the planner already saw it
        }
        let held: Vec<VertexId> = match &j.allocation {
            Some(names) => names.iter().filter_map(|n| st.graph.by_name(n)).collect(),
            None => j.placed_on.into_iter().collect(),
        };
        for (k, v) in vertices.iter().enumerate() {
            if held.contains(v) {
                extra[k] = extra[k].add(&j.request);
                any = true;
            }
        }
    }
    !any || vertices
        .iter()
        .zip(&extra)
        .all(|(&v, x)| st.graph.vertex(v).earliest_fit(&req.add(x), start, d) == Some(start))
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
    let running = !reserve_only && start <= now;
    if running && !clear_of_overruns(st, vertices, &req, start, d) {
        return false;
    }
    if !st
        .graph
        .commit_job(vertices, &req, start, d, Some(job.0), expect)
    {
        return false;
    }
    let j = st.jobs.get_mut(&job).unwrap();
    j.state = if running {
        JobState::Running
    } else {
        JobState::Reserved
    };
    j.planned_start = Some(start);
    j.planned_dur = Some(d);
    j.placed_on = vertices.first().copied();
    j.allocation = (vertices.len() > 1).then(|| {
        vertices
            .iter()
            .filter_map(|&v| st.graph.vertices.get(v as usize))
            .map(|vx| vx.name.clone())
            .collect()
    });
    if running {
        // Phase 4: enqueue executable work for the owning agents, one
        // assignment per node, ranked in vertex order.
        let cmd = j.command.clone();
        for (rank, &v) in vertices.iter().enumerate() {
            if let Some(vx) = st.graph.vertices.get(v as usize) {
                st.assignments
                    .entry(vx.name.clone())
                    .or_default()
                    .push(Assignment {
                        job,
                        command: cmd.clone(),
                        start,
                        rank: rank as u32,
                    });
            }
        }
    }
    true
}

/// Aggregate one gang rank's exit. The job completes when every rank has
/// reported: its spans are released at the latest rank's end and it is
/// `Failed` if any rank exited non-zero. The first non-zero exit puts every
/// rank that has not reported yet on the kill channel ([`State::kills`]), so
/// agents stop the peers instead of letting them run to the walltime; the
/// nodes stay booked until each killed rank has really exited and reported,
/// which keeps the planner consistent with what runs on the hardware.
/// Rejected (no effect) for jobs that are not running, nodes outside the
/// job's allocation, and repeated reports from the same rank.
fn rank_finished(st: &mut State, job: JobId, node: &str, end: u64, code: i32) -> bool {
    if st.jobs.get(&job).map(|j| j.state) != Some(JobState::Running) {
        return false;
    }
    let alloc = st
        .jobs
        .get(&job)
        .map(|j| st.job_nodes(j))
        .unwrap_or_default();
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
        let reported: BTreeSet<u32> = reports.keys().copied().collect();
        unqueue_kill(st, job, node);
        if code != 0 {
            kill_ranks(st, job, &alloc, &reported);
        }
        return true;
    }
    let reports = st.rank_reports.remove(&job).unwrap_or_default();
    let last = reports.values().map(|r| r.0).max().unwrap_or(end);
    let failed = reports.values().any(|r| r.1 != 0);
    finish(st, job, last, if failed { 1 } else { 0 });
    true
}

/// Queue a kill for every rank of `job` on `alloc` that has not reported.
fn kill_ranks(st: &mut State, job: JobId, alloc: &[String], reported: &BTreeSet<u32>) {
    for (r, n) in alloc.iter().enumerate() {
        if !reported.contains(&(r as u32)) {
            st.kills.entry(n.clone()).or_default().insert(job);
        }
    }
}

fn unqueue_kill(st: &mut State, job: JobId, node: &str) {
    if let Some(k) = st.kills.get_mut(node) {
        k.remove(&job);
        if k.is_empty() {
            st.kills.remove(node);
        }
    }
}

/// Cancel a running job: every rank that has not reported yet is put on the
/// kill channel. The job stays `Running` (its nodes booked) until the ranks
/// report, then ends `Cancelled`.
fn cancel_running(st: &mut State, job: JobId) {
    let Some(j) = st.jobs.get(&job) else { return };
    let alloc = st.job_nodes(j);
    let reported: BTreeSet<u32> = st
        .rank_reports
        .get(&job)
        .map(|r| r.keys().copied().collect())
        .unwrap_or_default();
    st.cancelling.insert(job);
    kill_ranks(st, job, &alloc, &reported);
}

fn finish(st: &mut State, job: JobId, end: u64, code: i32) {
    let cancelled = st.cancelling.remove(&job);
    if let Some(j) = st.jobs.get_mut(&job) {
        j.state = if cancelled {
            JobState::Cancelled
        } else if code == 0 {
            JobState::Completed
        } else {
            JobState::Failed
        };
    }
    st.rank_reports.remove(&job);
    st.gang_fetched.remove(&job);
    if !st.kills.is_empty() {
        st.kills.retain(|_, k| {
            k.remove(&job);
            !k.is_empty()
        });
    }
    st.graph.release_job(job.0, end);
}

fn apply_event(st: &mut State, ev: &Event, expect: &[(VertexId, u64)], now: u64) -> bool {
    match ev {
        Event::JobSubmitted(job) => {
            // Ids are chosen before the event is committed (API: the next
            // free id of the replica that took the request), so two
            // concurrent submissions can pick the same one; the later one
            // is rejected and the API retries with a fresh id.
            if st.jobs.contains_key(&job.id) {
                return false;
            }
            let mut job = job.clone();
            st.next_job_id = st.next_job_id.max(job.id.0 + 1);
            job.state = JobState::Pending;
            job.placed_on = None;
            job.allocation = None;
            st.jobs.insert(job.id, job);
        }
        Event::JobCancelled(id) => match st.jobs.get(id).map(|j| j.state) {
            Some(JobState::Pending | JobState::Reserved) => {
                let j = st.jobs.get_mut(id).unwrap();
                j.state = JobState::Cancelled;
                st.graph.release_job(id.0, 0);
                j.placed_on = None;
                j.allocation = None;
            }
            Some(JobState::Running) => cancel_running(st, *id),
            _ => return false,
        },
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
        Event::JobFinished { job, end, code } => {
            // A late report (the job already ended, e.g. its node was lost)
            // only clears the lost-rank bookkeeping; it never changes the
            // job's outcome.
            if st.jobs.get(job).map(|j| j.state) != Some(JobState::Running) {
                return clear_lost(st, *job, None);
            }
            finish(st, *job, *end, *code)
        }
        Event::JobRankFinished {
            job,
            node,
            end,
            code,
        } => {
            if st.jobs.get(job).map(|j| j.state) != Some(JobState::Running) {
                return clear_lost(st, *job, Some(node));
            }
            return rank_finished(st, *job, node, *end, *code);
        }
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
                let j = st.jobs.get_mut(&id).unwrap();
                j.placed_on = None;
                j.allocation = None;
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
            revive(st, name);
        }
        Event::AssignmentsFetched { node, count, items } => {
            // A lost node's queue was handed back to the planner: a fetch
            // racing with the loss claims nothing.
            if st.nodes.get(node).is_some_and(|n| n.down) {
                return false;
            }
            if let Some(q) = st.assignments.get_mut(node) {
                let taken: Vec<Assignment> = if items.is_empty() {
                    let n = (*count).min(q.len());
                    q.drain(..n).collect()
                } else {
                    let mut taken = Vec::new();
                    q.retain(|a| {
                        let hit = items.contains(&(a.job, a.rank));
                        if hit {
                            taken.push(a.clone());
                        }
                        !hit
                    });
                    taken
                };
                for a in &taken {
                    let gang = st
                        .jobs
                        .get(&a.job)
                        .is_some_and(|j| j.state == JobState::Running && j.node_count() > 1);
                    if gang {
                        st.gang_fetched.entry(a.job).or_default().insert(a.rank);
                    }
                }
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
                // A partitioned agent comes back by heartbeating to the same
                // controller, without registering again.
                revive(st, name);
            }
        }
        Event::NodeLost {
            name,
            last_heartbeat,
        } => return node_lost(st, name, *last_heartbeat, now),
    }
    true
}

/// [`Event::NodeLost`]: see the event's documentation.
fn node_lost(st: &mut State, name: &str, seen: u64, now: u64) -> bool {
    match st.nodes.get_mut(name) {
        Some(info) if !info.down && info.last_heartbeat == seen => info.down = true,
        _ => return false,
    }
    let unfetched: BTreeSet<JobId> = st
        .assignments
        .remove(name)
        .unwrap_or_default()
        .iter()
        .map(|a| a.job)
        .collect();
    // (job, state, gang, this node's rank already reported)
    let affected: Vec<(JobId, JobState, bool, bool)> = st
        .jobs
        .values()
        .filter(|j| matches!(j.state, JobState::Running | JobState::Reserved))
        .filter_map(|j| {
            let rank = st.job_nodes(j).iter().position(|n| n == name)? as u32;
            let reported = st
                .rank_reports
                .get(&j.id)
                .is_some_and(|r| r.contains_key(&rank));
            Some((j.id, j.state, j.is_gang(), reported))
        })
        .collect();
    for (job, state, gang, reported) in affected {
        match state {
            // Never started here: plan it again elsewhere.
            JobState::Reserved => requeue(st, job),
            JobState::Running if !gang && unfetched.contains(&job) => requeue(st, job),
            // This rank already ended; the gang goes on without the node.
            _ if reported => {}
            _ => {
                if !unfetched.contains(&job) {
                    st.lost.entry(name.to_string()).or_default().insert(job);
                }
                if gang {
                    rank_finished(st, job, name, now, NODE_LOST_EXIT);
                } else {
                    finish(st, job, now, NODE_LOST_EXIT);
                }
            }
        }
    }
    refresh_placement(st, name);
    true
}

/// A heartbeat or registration from a node marked down brings it back. It
/// stays out of placement until its agent has stopped and reported every
/// rank failed while it was lost: their spans are already released, so
/// placing new work beside them would overcommit the node.
fn revive(st: &mut State, name: &str) {
    if let Some(info) = st.nodes.get_mut(name) {
        info.down = false;
    }
    refresh_placement(st, name);
}

/// A node is placeable when it is up and no lost rank may still run on it.
fn refresh_placement(st: &mut State, name: &str) {
    let blocked = st.nodes.get(name).is_some_and(|n| n.down)
        || st.lost.get(name).is_some_and(|jobs| !jobs.is_empty());
    if let Some(v) = st.graph.by_name(name) {
        st.graph.set_down(v, blocked);
    }
}

/// Return a placed job to Pending, releasing its planner spans.
fn requeue(st: &mut State, job: JobId) {
    st.graph.release_job(job.0, 0);
    st.rank_reports.remove(&job);
    st.gang_fetched.remove(&job);
    if let Some(j) = st.jobs.get_mut(&job) {
        j.state = JobState::Pending;
        j.placed_on = None;
        j.allocation = None;
        j.planned_start = None;
        j.planned_dur = None;
    }
}

/// Forget that `job` may still run on a lost node (`node`, or any node),
/// once that node's agent has reported it. Returns whether anything changed.
fn clear_lost(st: &mut State, job: JobId, node: Option<&str>) -> bool {
    let mut cleared = Vec::new();
    st.lost.retain(|n, jobs| {
        if node.is_none_or(|want| want == n) && jobs.remove(&job) {
            cleared.push(n.clone());
        }
        !jobs.is_empty()
    });
    for n in &cleared {
        refresh_placement(st, n);
    }
    !cleared.is_empty()
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
    fn a_start_never_overlaps_a_running_job_past_its_planned_span() {
        let (clock, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(1, 4, 1000)));
        fsm.apply(Event::JobSubmitted(job(2, 4, 1000)));
        // Job 1 was planned for an estimated 10 s of its 1000 s walltime.
        assert!(
            fsm.apply_checked(Event::JobStarted {
                job: JobId(1),
                vertices: vec![1],
                start: 1000,
                dur: Some(10),
            })
            .accepted
        );
        clock.set(1020);
        // Its booking has ended but it still runs, unextended. The graph
        // shows the node free, and the version is unchanged, so the plan's
        // read set matches: only the overrun check can refuse this.
        let version = fsm.snapshot().graph().versions(&[1]);
        let start2 = Event::JobStarted {
            job: JobId(2),
            vertices: vec![1],
            start: 1020,
            dur: Some(10),
        };
        assert!(!fsm.apply_with_expect(start2.clone(), &version).accepted);
        // Extended, the booking itself refuses it.
        fsm.apply(Event::JobExtended {
            job: JobId(1),
            new_end: 1100,
        });
        assert!(!fsm.apply_checked(start2.clone()).accepted);
        // Finished, the node is free again.
        fsm.apply(Event::JobFinished {
            job: JobId(1),
            end: 1020,
            code: 0,
        });
        assert!(fsm.apply_checked(start2).accepted);
    }

    #[test]
    fn a_start_beside_an_overrunning_job_is_fine_when_both_fit() {
        let (clock, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(1, 2, 1000)));
        fsm.apply(Event::JobSubmitted(job(2, 2, 1000)));
        fsm.apply(Event::JobStarted {
            job: JobId(1),
            vertices: vec![1],
            start: 1000,
            dur: Some(10),
        });
        clock.set(1020);
        assert!(
            fsm.apply_checked(Event::JobStarted {
                job: JobId(2),
                vertices: vec![1],
                start: 1020,
                dur: Some(10),
            })
            .accepted
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

    #[test]
    fn a_second_submission_with_a_taken_id_is_rejected() {
        let (_c, fsm) = setup();
        assert!(
            fsm.apply_checked(Event::JobSubmitted(job(1, 1, 100)))
                .accepted
        );
        let mut other = job(1, 4, 900);
        other.name = "other".into();
        assert!(!fsm.apply_checked(Event::JobSubmitted(other)).accepted);
        fsm.read(|s| assert_eq!(s.jobs[&JobId(1)].request.cores, 1, "first one kept"));
    }

    type Proposal = (Event, Vec<(VertexId, u64)>);

    /// Records proposals and applies them locally, as a one-node log would.
    struct LoopbackProposer {
        target: Arc<Fsm>,
        seen: Mutex<Vec<Proposal>>,
        fail: bool,
    }

    impl Proposer for LoopbackProposer {
        fn propose(&self, ev: Event, expect: &[(VertexId, u64)]) -> Result<Applied, ProposeError> {
            if self.fail {
                return Err(ProposeError("no leader".into()));
            }
            self.seen
                .lock()
                .unwrap()
                .push((ev.clone(), expect.to_vec()));
            Ok(self.target.apply_at_expect(ev, 1234, expect))
        }
    }

    #[test]
    fn writes_go_through_the_proposer_but_committed_entries_do_not() {
        let fsm = Arc::new(Fsm::with_clock(Arc::new(ManualClock::new(0))));
        let p = Arc::new(LoopbackProposer {
            target: fsm.clone(),
            seen: Mutex::default(),
            fail: false,
        });
        assert!(!fsm.is_replicated());
        assert!(fsm.set_proposer(p.clone()).is_ok());
        assert!(fsm.is_replicated());
        let i = fsm.apply(Event::NodeRegistered {
            name: "n0".into(),
            capacity: Resources::default(),
        });
        assert_eq!(i, 1);
        fsm.apply_with_expect(Event::JobSubmitted(job(1, 0, 10)), &[(1, 5)]);
        assert_eq!(p.seen.lock().unwrap().len(), 2);
        assert_eq!(p.seen.lock().unwrap()[1].1, [(1, 5)], "read set travels");
        // The committed path (what Raft calls) applies directly.
        fsm.apply_at(Event::JobCancelled(JobId(1)), 99);
        assert_eq!(p.seen.lock().unwrap().len(), 2);
        assert_eq!(fsm.index(), 3);
    }

    #[test]
    fn an_uncommitted_write_is_an_error_not_a_state_change() {
        let fsm = Fsm::with_clock(Arc::new(ManualClock::new(0)));
        let target = Arc::new(Fsm::new());
        let _ = fsm.set_proposer(Arc::new(LoopbackProposer {
            target,
            seen: Mutex::default(),
            fail: true,
        }));
        let ev = Event::JobSubmitted(job(1, 1, 10));
        assert!(fsm.try_apply(ev.clone()).is_err());
        assert!(!fsm.apply_checked(ev).accepted);
        assert_eq!(fsm.index(), 0);
    }

    #[test]
    fn tail_keeps_the_last_events_in_index_order_when_enabled() {
        let (_c, fsm) = setup();
        assert_eq!(fsm.tail_after(0, 10), (vec![], None), "off by default");
        assert!(fsm.enable_tail(3));
        assert!(!fsm.enable_tail(3));
        for id in 1..=4 {
            fsm.apply(Event::JobSubmitted(job(id, 1, 10)));
        }
        fsm.apply(Event::JobSubmitted(job(1, 1, 10))); // duplicate id: rejected
        let (all, first) = fsm.tail_after(0, 10);
        let idx: Vec<_> = all.iter().map(|e| e.index).collect();
        assert_eq!(idx, [4, 5, 6], "capped at 3, oldest dropped");
        assert_eq!(first, Some(4));
        assert!(!all[2].accepted);
        let (after5, _) = fsm.tail_after(5, 10);
        assert_eq!(after5.len(), 1);
        assert_eq!(fsm.tail_after(4, 1).0[0].index, 5, "limit");
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
            assert_eq!(s.job_nodes(&s.jobs[&JobId(1)]), list);
            for (rank, node) in list.iter().enumerate() {
                let q = &s.assignments[node];
                assert_eq!(q.len(), 1);
                assert_eq!(q[0].rank, rank as u32);
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
            assert!(s.job_nodes(&s.jobs[&JobId(1)]).is_empty());
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
            assert_eq!(
                s.job_nodes(&s.jobs[&JobId(1)]).len(),
                3,
                "kept for listings"
            );
        });
        // Reports after completion have no effect.
        assert!(!fsm.apply_checked(rank_done("n0", 1080, 0)).accepted);
    }

    #[test]
    fn gang_fails_if_any_rank_fails() {
        let (_c, fsm) = gang_setup(2);
        fsm.apply(start(1, vec![1, 2]));
        fsm.apply(rank_done("n0", 1010, 3));
        assert_eq!(
            state_of(&fsm, 1),
            JobState::Running,
            "nodes stay booked until the killed peer exits"
        );
        fsm.apply(rank_done("n1", 1020, 0));
        assert_eq!(state_of(&fsm, 1), JobState::Failed);
    }

    fn kills(fsm: &Fsm) -> Vec<(String, u64)> {
        fsm.read(|s| {
            s.kills
                .iter()
                .flat_map(|(n, js)| js.iter().map(move |j| (n.clone(), j.0)))
                .collect()
        })
    }

    #[test]
    fn failed_rank_puts_every_unreported_peer_on_the_kill_channel() {
        let (_c, fsm) = gang_setup(3);
        fsm.apply(start(1, vec![1, 2, 3]));
        fsm.apply(rank_done("n2", 1005, 0)); // already finished: not killed
        assert!(kills(&fsm).is_empty(), "a clean exit kills nobody");
        fsm.apply(rank_done("n0", 1010, 9));
        assert_eq!(kills(&fsm), [("n1".into(), 1)]);
        fsm.read(|s| {
            assert!(s.is_killed(JobId(1), "n1"));
            assert_eq!(s.kills_for("n1").collect::<Vec<_>>(), [JobId(1)]);
            assert_eq!(s.kills_for("n0").count(), 0);
        });
        // The killed peer's report clears its entry and completes the job.
        fsm.apply(rank_done("n1", 1012, 143));
        assert!(kills(&fsm).is_empty());
        assert_eq!(state_of(&fsm, 1), JobState::Failed);
        let freed = fsm.read(|s| {
            s.graph.vertex(2).earliest_fit(
                &Resources {
                    cores: 4,
                    mem_mb: 0,
                    gpus: 0,
                },
                1012,
                50,
            )
        });
        assert_eq!(freed, Some(1012), "nodes free once the last rank exits");
    }

    #[test]
    fn kill_entries_are_dropped_when_the_job_ends_another_way() {
        let (_c, fsm) = gang_setup(2);
        fsm.apply(start(1, vec![1, 2]));
        fsm.apply(rank_done("n0", 1010, 1));
        assert_eq!(kills(&fsm).len(), 1);
        fsm.apply(Event::JobFinished {
            job: JobId(1),
            end: 1011,
            code: 1,
        });
        assert!(kills(&fsm).is_empty());
    }

    #[test]
    fn cancelling_a_running_gang_kills_every_rank_and_ends_cancelled() {
        let (_c, fsm) = gang_setup(2);
        fsm.apply(start(1, vec![1, 2]));
        assert!(fsm.apply_checked(Event::JobCancelled(JobId(1))).accepted);
        assert_eq!(state_of(&fsm, 1), JobState::Running, "until ranks exit");
        assert_eq!(kills(&fsm), [("n0".into(), 1), ("n1".into(), 1)]);
        fsm.apply(rank_done("n0", 1010, 143));
        fsm.apply(rank_done("n1", 1011, 143));
        assert_eq!(state_of(&fsm, 1), JobState::Cancelled);
        fsm.read(|s| assert!(s.kills.is_empty() && s.cancelling.is_empty()));
    }

    #[test]
    fn cancelling_a_running_single_node_job_kills_it() {
        let (_c, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(1, 1, 100)));
        fsm.apply(start(1, vec![1]));
        fsm.apply(Event::JobCancelled(JobId(1)));
        assert_eq!(kills(&fsm), [("n0".into(), 1)]);
        fsm.apply(Event::JobFinished {
            job: JobId(1),
            end: 1010,
            code: 143,
        });
        assert_eq!(state_of(&fsm, 1), JobState::Cancelled);
        assert!(kills(&fsm).is_empty());
        assert!(
            !fsm.apply_checked(Event::JobCancelled(JobId(1))).accepted,
            "a finished job cannot be cancelled"
        );
    }

    #[test]
    fn kill_channel_replays_exactly() {
        let (clock, fsm) = gang_setup(3);
        fsm.apply(start(1, vec![1, 2, 3]));
        clock.set(1010);
        fsm.apply(rank_done("n0", 1010, 2));
        let c2 = Arc::new(ManualClock::new(1000));
        let replica = Fsm::with_clock(c2.clone());
        for e in fsm.journal() {
            c2.set(e.at);
            replica.apply(e.event);
        }
        assert_eq!(
            fsm.read(|s| format!("{s:?}")),
            replica.read(|s| format!("{s:?}"))
        );
        assert_eq!(kills(&replica).len(), 2);
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
            assert_eq!(s.job_nodes(&s.jobs[&JobId(1)]).len(), 2);
            assert!(s.assignments.is_empty(), "no work before the gang starts");
        });
        fsm.apply(Event::ReservationsCleared { now: 1000 });
        fsm.read(|s| assert!(s.jobs.values().all(|j| s.job_nodes(j).is_empty())));
        fsm.apply(Event::JobReserved {
            job: JobId(1),
            vertices: vec![3, 4],
            start: 2000,
            dur: None,
        });
        fsm.apply(Event::JobCancelled(JobId(1)));
        fsm.read(|s| {
            assert!(s.jobs.values().all(|j| s.job_nodes(j).is_empty()));
            assert!(s.graph.vertex(3).planner.job_span(1).is_none());
        });
    }

    fn fetched(node: &str) -> Event {
        Event::AssignmentsFetched {
            node: node.into(),
            count: 1,
            items: vec![],
        }
    }

    #[test]
    fn fetching_by_identity_is_idempotent() {
        let (_c, fsm) = setup();
        for id in 1..=2 {
            fsm.apply(Event::JobSubmitted(job(id, 1, 100)));
            fsm.apply(start(id, vec![1]));
        }
        let first = Event::AssignmentsFetched {
            node: "n0".into(),
            count: 1,
            items: vec![(JobId(1), 0)],
        };
        fsm.apply(first.clone());
        fsm.apply(first); // replayed by a retried write: a no-op
        let left: Vec<JobId> = fsm.read(|s| s.assignments["n0"].iter().map(|a| a.job).collect());
        assert_eq!(left, [JobId(2)], "job 2's work must not be dropped");
    }

    #[test]
    fn gang_is_released_only_once_every_rank_is_fetched() {
        let (clock, fsm) = gang_setup(3);
        let release = |f: &Fsm| f.read(|s| s.gang_release(JobId(1)));
        assert_eq!(release(&fsm), GangRelease::NotRunning, "not started yet");
        fsm.apply(start(1, vec![1, 2, 3]));
        assert_eq!(
            release(&fsm),
            GangRelease::Waiting {
                fetched: 0,
                nodes: 3
            }
        );
        fsm.apply(fetched("n0"));
        fsm.apply(fetched("n2"));
        fsm.apply(fetched("n2")); // nothing left to fetch on n2: no double count
        assert_eq!(
            release(&fsm),
            GangRelease::Waiting {
                fetched: 2,
                nodes: 3
            }
        );
        fsm.apply(fetched("n1"));
        assert_eq!(release(&fsm), GangRelease::Released);
        // Completion clears the barrier state with the job.
        clock.set(1050);
        for n in ["n0", "n1", "n2"] {
            fsm.apply(rank_done(n, 1050, 0));
        }
        assert_eq!(release(&fsm), GangRelease::NotRunning);
        fsm.read(|s| assert!(s.gang_fetched.is_empty()));
        // The barrier is replicated state: replay reproduces it exactly.
        let replica = Fsm::with_clock(Arc::new(ManualClock::new(1000)));
        for e in fsm.journal() {
            replica.apply_at(e.event, e.at);
        }
        assert_eq!(
            replica.read(|s| format!("{s:?}")),
            fsm.read(|s| format!("{s:?}"))
        );
    }

    #[test]
    fn single_node_jobs_are_released_immediately_and_not_tracked() {
        let (_c, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(1, 1, 100)));
        fsm.apply(Event::JobStarted {
            job: JobId(1),
            vertices: vec![1],
            start: 1000,
            dur: None,
        });
        assert_eq!(
            fsm.read(|s| s.gang_release(JobId(1))),
            GangRelease::Released
        );
        fsm.apply(fetched("n0"));
        fsm.read(|s| assert!(s.gang_fetched.is_empty()));
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

    fn fetch_rank(node: &str, job: u64, rank: u32) -> Event {
        Event::AssignmentsFetched {
            node: node.into(),
            count: 1,
            items: vec![(JobId(job), rank)],
        }
    }

    /// The leader's loss proposal for `node` as of now.
    fn lost(fsm: &Fsm, node: &str) -> Event {
        Event::NodeLost {
            name: node.into(),
            last_heartbeat: fsm.read(|s| s.nodes[node].last_heartbeat),
        }
    }

    #[test]
    fn stale_nodes_ignore_down_nodes_and_time_before_leadership() {
        let (_c, fsm) = setup(); // n0 heartbeat at 1000
        fsm.read(|s| {
            assert!(
                s.stale_nodes(1030, 30, 0).is_empty(),
                "exactly the timeout is not stale"
            );
            assert_eq!(s.stale_nodes(1031, 30, 0), [("n0".to_string(), 1000)]);
            assert!(
                s.stale_nodes(1031, 30, 1020).is_empty(),
                "a new leader waits a full timeout before declaring nodes lost"
            );
        });
        fsm.apply(lost(&fsm, "n0"));
        assert!(fsm.read(|s| s.stale_nodes(5000, 30, 0).is_empty()));
    }

    #[test]
    fn node_lost_fails_a_running_job_and_takes_the_node_out_of_placement() {
        let (clock, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(1, 4, 1000)));
        fsm.apply(start(1, vec![1]));
        fsm.apply(fetch_rank("n0", 1, 0));
        clock.set(1100);
        assert!(fsm.apply_checked(lost(&fsm, "n0")).accepted);
        assert!(
            !fsm.apply_checked(lost(&fsm, "n0")).accepted,
            "already down"
        );
        assert!(
            !fsm.apply_checked(Event::NodeLost {
                name: "nope".into(),
                last_heartbeat: 0
            })
            .accepted,
            "unknown node"
        );
        assert_eq!(state_of(&fsm, 1), JobState::Failed);
        fsm.read(|s| {
            assert!(s.nodes["n0"].down && s.graph.vertex(1).down);
            assert!(s.graph.schedulable_nodes().is_empty());
            assert_eq!(
                s.graph.vertex(1).planner.job_span(1),
                Some((1000, 1100)),
                "the rest of the window is released at the loss"
            );
            // The agent may only be partitioned away: tell it to stop the rank.
            assert_eq!(s.kills_for("n0").collect::<Vec<_>>(), [JobId(1)]);
        });
        fsm.apply(Event::JobSubmitted(job(2, 1, 100)));
        assert!(
            !fsm.apply_checked(start(2, vec![1])).accepted,
            "a plan made before the loss must not book the down node"
        );
    }

    #[test]
    fn node_lost_requeues_work_that_never_started_there() {
        let (_c, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(1, 2, 100)));
        fsm.apply(Event::JobSubmitted(job(2, 4, 100)));
        fsm.apply(start(1, vec![1])); // queued, never fetched
        fsm.apply(Event::JobReserved {
            job: JobId(2),
            vertices: vec![1],
            start: 1100,
            dur: None,
        });
        fsm.apply(lost(&fsm, "n0"));
        assert_eq!(state_of(&fsm, 1), JobState::Pending);
        assert_eq!(state_of(&fsm, 2), JobState::Pending);
        fsm.read(|s| {
            assert!(s.assignments.get("n0").is_none_or(Vec::is_empty));
            assert!(s.lost.is_empty() && s.kills_for("n0").next().is_none());
            assert!(s.jobs.values().all(|j| j.placed_on.is_none()));
            assert_eq!(s.graph.vertex(1).planner.job_span(1), None);
        });
    }

    #[test]
    fn a_returning_node_is_placeable_and_its_late_report_changes_nothing() {
        let (clock, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(1, 4, 1000)));
        fsm.apply(start(1, vec![1]));
        fsm.apply(fetch_rank("n0", 1, 0));
        clock.set(1100);
        fsm.apply(lost(&fsm, "n0"));
        // The partition heals: the agent heartbeats the same controller.
        clock.set(1200);
        fsm.apply(Event::Heartbeat {
            name: "n0".into(),
            allocated: Resources::default(),
        });
        fsm.read(|s| {
            assert!(!s.nodes["n0"].down, "alive again");
            assert!(
                s.graph.schedulable_nodes().is_empty(),
                "not placeable while the failed rank may still run there"
            );
        });
        // It stops the rank (kill list) and reports it; the outcome stays.
        let late = Event::JobFinished {
            job: JobId(1),
            end: 1200,
            code: 0,
        };
        assert!(
            fsm.apply_checked(late.clone()).accepted,
            "clears the lost rank"
        );
        assert_eq!(state_of(&fsm, 1), JobState::Failed);
        assert!(fsm.read(|s| s.lost.is_empty() && s.kills_for("n0").next().is_none()));
        assert_eq!(
            fsm.read(|s| s.graph.schedulable_nodes()),
            [1],
            "placeable again"
        );
        assert!(!fsm.apply_checked(late).accepted, "nothing left to clear");
        assert_eq!(state_of(&fsm, 1), JobState::Failed);
    }

    #[test]
    fn node_lost_fails_a_gang_and_kills_its_peers() {
        let (clock, fsm) = gang_setup(3);
        fsm.apply(start(1, vec![1, 2, 3]));
        for (rank, node) in ["n0", "n1", "n2"].iter().enumerate() {
            fsm.apply(fetch_rank(node, 1, rank as u32));
        }
        clock.set(1020);
        fsm.apply(lost(&fsm, "n1"));
        assert_eq!(
            state_of(&fsm, 1),
            JobState::Running,
            "peers still hold their nodes until they report"
        );
        fsm.read(|s| {
            for node in ["n0", "n2", "n1"] {
                assert!(s.kills_for(node).any(|j| j == JobId(1)), "{node}");
            }
        });
        clock.set(1022);
        fsm.apply(rank_done("n0", 1022, 143));
        fsm.apply(rank_done("n2", 1022, 143));
        assert_eq!(state_of(&fsm, 1), JobState::Failed);
        fsm.read(|s| {
            assert!(s.kills_for("n0").next().is_none() && s.kills_for("n2").next().is_none());
            assert_eq!(
                s.kills_for("n1").collect::<Vec<_>>(),
                [JobId(1)],
                "lost rank"
            );
        });
    }

    #[test]
    fn losing_a_node_whose_gang_rank_already_reported_leaves_the_gang_alone() {
        let (clock, fsm) = gang_setup(2);
        fsm.apply(start(1, vec![1, 2]));
        fsm.apply(fetch_rank("n0", 1, 0));
        fsm.apply(fetch_rank("n1", 1, 1));
        clock.set(1010);
        fsm.apply(rank_done("n0", 1010, 0));
        fsm.apply(lost(&fsm, "n0"));
        assert_eq!(state_of(&fsm, 1), JobState::Running);
        assert!(fsm.read(|s| s.lost.is_empty() && s.kills_for("n1").next().is_none()));
        fsm.apply(rank_done("n1", 1020, 0));
        assert_eq!(state_of(&fsm, 1), JobState::Completed);
    }

    #[test]
    fn node_loss_journal_replays_to_identical_state() {
        let (clock, fsm) = gang_setup(2);
        fsm.apply(Event::JobSubmitted(job(2, 1, 100)));
        fsm.apply(start(1, vec![1, 2]));
        fsm.apply(fetch_rank("n0", 1, 0));
        fsm.apply(Event::JobStarted {
            job: JobId(2),
            vertices: vec![3],
            start: 1000,
            dur: None,
        });
        clock.set(1040);
        fsm.apply(lost(&fsm, "n1")); // gang rank 1 never fetched; job 2 on n2 untouched
        fsm.apply(lost(&fsm, "n2")); // job 2 never fetched: back to Pending
        fsm.apply(rank_done("n0", 1041, 143));
        clock.set(1060);
        fsm.apply(Event::NodeRegistered {
            name: "n1".into(),
            capacity: Resources {
                cores: 4,
                mem_mb: 0,
                gpus: 0,
            },
        });
        let c2 = Arc::new(ManualClock::new(1000));
        let replica = Fsm::with_clock(c2.clone());
        for e in fsm.journal() {
            c2.set(e.at);
            replica.apply(e.event);
        }
        assert_eq!(
            fsm.read(|s| format!("{s:?}")),
            replica.read(|s| format!("{s:?}"))
        );
        assert_eq!(state_of(&replica, 1), JobState::Failed);
        assert_eq!(state_of(&replica, 2), JobState::Pending);
    }

    #[test]
    fn a_loss_proposal_older_than_the_last_heartbeat_is_rejected() {
        let (clock, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(1, 4, 1000)));
        fsm.apply(start(1, vec![1]));
        fsm.apply(fetch_rank("n0", 1, 0));
        clock.set(1040);
        let stale = lost(&fsm, "n0"); // the leader's check saw the 1000 heartbeat
        fsm.apply(Event::Heartbeat {
            name: "n0".into(),
            allocated: Resources::default(),
        });
        assert!(
            !fsm.apply_checked(stale).accepted,
            "a heartbeat committed after the check proves the node alive"
        );
        assert_eq!(state_of(&fsm, 1), JobState::Running);
        assert!(!fsm.read(|s| s.nodes["n0"].down));
    }

    #[test]
    fn a_fetch_racing_with_the_loss_claims_nothing() {
        let (_c, fsm) = setup();
        fsm.apply(Event::JobSubmitted(job(1, 4, 1000)));
        fsm.apply(start(1, vec![1]));
        // FetchWork took its snapshot (assignment queued), then the loss commits.
        fsm.apply(lost(&fsm, "n0"));
        assert_eq!(
            state_of(&fsm, 1),
            JobState::Pending,
            "handed back to the planner"
        );
        assert!(
            !fsm.apply_checked(fetch_rank("n0", 1, 0)).accepted,
            "the stale claim must not be honoured"
        );
    }
}
