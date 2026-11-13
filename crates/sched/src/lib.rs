//! The scheduling core (report section 4.3).
//!
//! Omega/Nomad pattern, Rust-flavored:
//!
//! * [`EvalBroker`] — priority queue of *evaluations* (one per job) with
//!   idempotence-based coalescing: re-enqueueing a job whose snapshot index
//!   hasn't changed is a no-op (Nomad's 99%-load-cut trick, applied here to
//!   the backfill domain — report 5.2).
//! * [`Policy`] — computes a [`Plan`] as a *pure function* of (snapshot,
//!   now). No locks held while deciding.
//! * [`PlanApplier`] — single-writer commit point with **per-vertex**
//!   optimistic concurrency: a plan conflicts only if a vertex it uses
//!   changed *and* the plan no longer fits there. (The original skeleton
//!   rejected a plan if *any* event — even a heartbeat — landed after its
//!   snapshot.)
//! * [`backfill_pass`] — one conservative-backfill pass with reservation
//!   depth limit, re-planning and overrun handling; shared by the
//!   controller and the simulator so replay runs production code.
//! * [`QuantileEstimator`] — distribution-aware runtime planning with a
//!   risk budget; [`ShadowPolicy`] — run a candidate policy alongside the
//!   live one and record where they disagree.
//! * [`BiasCorrectedEstimator`] — runtime prediction with online bias
//!   correction under an asymmetric loss and wide-job protection.

mod bias;
pub use bias::{BiasConfig, BiasCorrectedEstimator, GuardMode};

use gnomon_core::{Event, Job, JobId, JobState, StateIndex};
use gnomon_fsm::{Fsm, Snapshot};
use gnomon_graph::{ResourceGraph, VertexId};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// Unit of scheduling work (Nomad-style evaluation).
#[derive(Clone, Debug)]
pub struct Eval {
    pub job: JobId,
    pub priority: u64,
    /// FSM index at enqueue time — coalescing key.
    pub enqueued_at: StateIndex,
    pub seq: u64,
}

/// The output of a worker: a pure decision over a snapshot.
#[derive(Clone, Debug)]
pub struct Plan {
    pub job: JobId,
    /// Snapshot index the plan was computed against.
    pub base_index: StateIndex,
    pub decision: Decision,
    /// Versions of the vertices the decision uses (per-vertex OCC read set).
    pub read_set: Vec<(VertexId, u64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// Start immediately on these graph vertices. `dur` = planned span
    /// (`None` = full walltime).
    Start {
        vertices: Vec<u32>,
        start: u64,
        dur: Option<u64>,
    },
    /// Backfill reservation in the future.
    Reserve {
        vertices: Vec<u32>,
        start: u64,
        dur: Option<u64>,
    },
    /// Cannot place even in the future window — leave pending.
    Defer,
}

impl Decision {
    /// Same action, time and vertices (ignores the planned-span length, which
    /// differs by construction between estimators).
    pub fn same_placement(&self, other: &Decision) -> bool {
        match (self, other) {
            (
                Decision::Start {
                    vertices: a,
                    start: s,
                    ..
                },
                Decision::Start {
                    vertices: b,
                    start: t,
                    ..
                },
            )
            | (
                Decision::Reserve {
                    vertices: a,
                    start: s,
                    ..
                },
                Decision::Reserve {
                    vertices: b,
                    start: t,
                    ..
                },
            ) => a == b && s == t,
            (Decision::Defer, Decision::Defer) => true,
            _ => false,
        }
    }

    pub fn vertices(&self) -> &[u32] {
        match self {
            Decision::Start { vertices, .. } | Decision::Reserve { vertices, .. } => vertices,
            Decision::Defer => &[],
        }
    }
    pub fn to_event(&self, job: JobId) -> Option<Event> {
        match self {
            Decision::Start {
                vertices,
                start,
                dur,
            } => Some(Event::JobStarted {
                job,
                vertices: vertices.clone(),
                start: *start,
                dur: *dur,
            }),
            Decision::Reserve {
                vertices,
                start,
                dur,
            } => Some(Event::JobReserved {
                job,
                vertices: vertices.clone(),
                start: *start,
                dur: *dur,
            }),
            Decision::Defer => None,
        }
    }
}

/// Priority-ordered, coalescing evaluation broker.
#[derive(Default)]
pub struct EvalBroker {
    pending: Mutex<BrokerInner>,
    seq: AtomicU64,
    /// Metrics: coalesced-away enqueues (the load-shedding win).
    pub coalesced: AtomicU64,
}

#[derive(Default)]
struct BrokerInner {
    /// job -> latest eval (dedup by job).
    by_job: HashMap<JobId, Eval>,
    /// (priority desc, seq asc) -> job, FIFO within a priority.
    queue: BTreeMap<(std::cmp::Reverse<u64>, u64), JobId>,
}

impl EvalBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue (or coalesce) an evaluation for a job.
    pub fn enqueue(&self, job: &Job, index: StateIndex) {
        let mut inner = self.pending.lock().unwrap();
        if let Some(old) = inner.by_job.get(&job.id) {
            // Idempotence: same job, state unchanged since its pending eval
            // was queued -> the new eval would produce the same plan.
            if old.enqueued_at >= index {
                self.coalesced.fetch_add(1, Ordering::Relaxed);
                return;
            }
            let old_key = (std::cmp::Reverse(old.priority), old.seq);
            inner.queue.remove(&old_key);
        }
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let eval = Eval {
            job: job.id,
            priority: job.priority,
            enqueued_at: index,
            seq,
        };
        inner.by_job.insert(job.id, eval.clone());
        inner
            .queue
            .insert((std::cmp::Reverse(eval.priority), seq), job.id);
    }

    pub fn dequeue(&self) -> Option<Eval> {
        let mut inner = self.pending.lock().unwrap();
        let key = inner.queue.keys().next().copied()?;
        let id = inner.queue.remove(&key)?;
        inner.by_job.remove(&id);
        Some(Eval {
            job: id,
            priority: key.0 .0,
            enqueued_at: 0,
            seq: key.1,
        })
    }

    pub fn len(&self) -> usize {
        self.pending.lock().unwrap().queue.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Pure scheduling policy: (graph, job, now) in, decision out. Replaceable
/// per-partition or as a WASM plugin (report section 4.5).
pub trait Policy: Send + Sync {
    /// Decide for one job. `allow_reserve = false` asks only "can it start
    /// now?" (reservation-depth limit, like Slurm's `bf_max_job_test`).
    fn place(&self, graph: &ResourceGraph, job: &Job, now: u64, allow_reserve: bool) -> Decision;

    fn name(&self) -> String {
        "policy".into()
    }

    /// Planned span for a job (walltime unless a runtime model is attached).
    fn planned_secs(&self, job: &Job) -> u64 {
        job.walltime.secs()
    }

    /// Called when a job completes, so learning policies can update.
    fn observe(&self, _job: &Job, _actual_secs: u64) {}

    /// Called once per [`backfill_pass`] with the queued (pending and
    /// reserved) jobs, before the previous pass's reservations are cleared
    /// and before any [`place`](Self::place) of this pass (default no-op).
    fn begin_pass(&self, _queue: &[&Job], _graph: &ResourceGraph, _now: u64) {}

    /// For a running job that outlived its planned span: new span length.
    fn extend_secs(&self, job: &Job, elapsed: u64) -> u64 {
        job.walltime.secs().max(elapsed)
    }

    /// Snapshot-level entry point used by worker threads.
    fn schedule(&self, snap: &Snapshot, job: &Job) -> Plan {
        let decision = self.place(snap.graph(), job, snap.now, true);
        let read_set = snap.graph().versions(decision.vertices());
        Plan {
            job: job.id,
            base_index: snap.index,
            decision,
            read_set,
        }
    }
}

/// Default policy: earliest-fit over the resource graph's per-vertex
/// planners — backfill as a pure function (the "when" separated from the
/// "where", Fluxion's qmanager/resource split). With an `estimator`, spans
/// are planned at a runtime quantile instead of the user's walltime.
/// Gang jobs (`nodes > 1`) are placed all-or-nothing on `nodes` distinct
/// node leaves with a common start ([`ResourceGraph::match_gang`]), in both
/// the start-now path and the reservation path.
pub struct BackfillPolicy {
    /// How far ahead reservations may look (Slurm's bf_window, in seconds).
    pub window_secs: u64,
    pub estimator: Option<Arc<dyn RuntimeEstimator>>,
}

impl BackfillPolicy {
    pub fn new(window_secs: u64) -> Self {
        BackfillPolicy {
            window_secs,
            estimator: None,
        }
    }
    pub fn with_estimator(window_secs: u64, est: Arc<dyn RuntimeEstimator>) -> Self {
        BackfillPolicy {
            window_secs,
            estimator: Some(est),
        }
    }
}

impl Default for BackfillPolicy {
    fn default() -> Self {
        Self::new(48 * 3600)
    }
}

impl Policy for BackfillPolicy {
    fn name(&self) -> String {
        match &self.estimator {
            None => "backfill/walltime".into(),
            Some(e) => format!("backfill/{}", e.name()),
        }
    }

    fn planned_secs(&self, job: &Job) -> u64 {
        match &self.estimator {
            Some(e) => e.planned_secs(job),
            None => job.walltime.secs(),
        }
    }

    fn observe(&self, job: &Job, actual: u64) {
        if let Some(e) = &self.estimator {
            e.observe(job, actual);
        }
    }

    fn extend_secs(&self, job: &Job, elapsed: u64) -> u64 {
        match &self.estimator {
            Some(e) => e.extend_secs(job, elapsed),
            None => job.walltime.secs().max(elapsed),
        }
    }

    fn place(&self, graph: &ResourceGraph, job: &Job, now: u64, allow_reserve: bool) -> Decision {
        let Some(est) = &self.estimator else {
            return self.place_span(graph, job, now, allow_reserve, job.walltime.secs(), None);
        };
        let spans = est.spans_on(job, MachineShape::of(graph));
        let plan = spans.plan;
        let decision = self.place_span(graph, job, now, allow_reserve, plan, Some(plan));
        if !matches!(decision, Decision::Start { .. }) {
            return decision;
        }
        // Starts now at the predicted span. If it would not at the stricter
        // guard span, the prediction squeezes it in front of a reservation:
        // place it as the guard span allows (recording the prediction).
        if spans.guard > plan {
            match self.place_span(graph, job, now, allow_reserve, spans.guard, Some(plan)) {
                Decision::Start { .. } => {}
                guarded => return guarded,
            }
        }
        // Record the more conservative span when it also starts now, so the
        // reservations of later jobs do not rest on an optimistic span.
        if spans.record > plan {
            let rec = Some(spans.record);
            if let d @ Decision::Start { .. } =
                self.place_span(graph, job, now, allow_reserve, spans.record, rec)
            {
                return d;
            }
        }
        decision
    }

    fn begin_pass(&self, queue: &[&Job], graph: &ResourceGraph, now: u64) {
        if let Some(e) = &self.estimator {
            e.begin_pass(queue, MachineShape::of(graph), now);
        }
    }
}

impl BackfillPolicy {
    /// Earliest-fit placement of `job` for a span of `dur` seconds;
    /// `dur_field` is the planned span recorded in the decision.
    fn place_span(
        &self,
        graph: &ResourceGraph,
        job: &Job,
        now: u64,
        allow_reserve: bool,
        dur: u64,
        dur_field: Option<u64>,
    ) -> Decision {
        let nodes = job.node_count();
        if !allow_reserve && nodes > 1 {
            return match gang_start_now(graph, job, now, dur) {
                Some(vertices) => Decision::Start {
                    vertices,
                    start: now,
                    dur: dur_field,
                },
                None => Decision::Defer,
            };
        }
        if !allow_reserve {
            // Cheap "start now?" check: skip vertices without free capacity now.
            for id in graph.leaves() {
                let v = graph.vertex(id);
                if v.name.starts_with("granted:") {
                    continue;
                }
                let free = v.cap().base; // quick reject on current usage
                if !v.planner.usage_at(now).add(&job.request).fits_in(&free) && v.profile.is_none()
                {
                    continue;
                }
                if v.earliest_fit(&job.request, now, dur) == Some(now) {
                    return Decision::Start {
                        vertices: vec![id],
                        start: now,
                        dur: dur_field,
                    };
                }
            }
            return Decision::Defer;
        }
        match graph.match_gang(&job.request, nodes, now, dur, job.id.0) {
            Some((vertices, start)) if start <= now => Decision::Start {
                vertices,
                start,
                dur: dur_field,
            },
            Some((vertices, start)) if start <= now + self.window_secs => Decision::Reserve {
                vertices,
                start,
                dur: dur_field,
            },
            _ => Decision::Defer,
        }
    }
}

/// "Start now?" for a gang: the first `nodes` schedulable node leaves (in
/// vertex order) that can each hold the per-node request for `[now,
/// now+dur)`, or `None` if fewer are free. One `earliest_fit` per leaf at
/// most, stopping as soon as the gang is complete.
fn gang_start_now(graph: &ResourceGraph, job: &Job, now: u64, dur: u64) -> Option<Vec<VertexId>> {
    let need = job.node_count() as usize;
    let mut chosen = Vec::with_capacity(need);
    for id in graph.schedulable_nodes() {
        if graph.vertex(id).earliest_fit(&job.request, now, dur) == Some(now) {
            chosen.push(id);
            if chosen.len() == need {
                return Some(chosen);
            }
        }
    }
    None
}

/// How the applier decides that an optimistic plan conflicted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictMode {
    /// Original skeleton: reject if *any* event was applied since the plan's
    /// snapshot (heartbeats included). Kept for A/B benchmarking.
    GlobalIndex,
    /// Reject only if the job is no longer pending or the plan no longer fits
    /// on the vertices it uses; unchanged vertices commit on a fast path.
    PerVertex,
}

/// Single-writer commit point (Raft leader side).
pub struct PlanApplier {
    pub mode: ConflictMode,
    pub committed: AtomicU64,
    pub conflicts: AtomicU64,
}

impl Default for PlanApplier {
    fn default() -> Self {
        Self::new()
    }
}

impl PlanApplier {
    pub fn new() -> Self {
        Self::with_mode(ConflictMode::PerVertex)
    }

    pub fn with_mode(mode: ConflictMode) -> Self {
        PlanApplier {
            mode,
            committed: AtomicU64::new(0),
            conflicts: AtomicU64::new(0),
        }
    }

    /// Optimistic commit. Returns `Ok(Some(event))` on commit, `Ok(None)` for
    /// a deferral, `Err(())` on conflict (caller re-enqueues the job).
    /// `committed` counts only plans the state machine actually accepted.
    #[allow(clippy::result_unit_err)]
    pub fn apply(&self, fsm: &Fsm, plan: &Plan) -> Result<Option<Event>, ()> {
        let Some(ev) = plan.decision.to_event(plan.job) else {
            return Ok(None);
        };
        if self.mode == ConflictMode::GlobalIndex && fsm.index() != plan.base_index {
            self.conflicts.fetch_add(1, Ordering::Relaxed);
            return Err(());
        }
        let expect: &[(VertexId, u64)] = match self.mode {
            ConflictMode::PerVertex => &plan.read_set,
            ConflictMode::GlobalIndex => &[],
        };
        if fsm.apply_with_expect(ev.clone(), expect).accepted {
            self.committed.fetch_add(1, Ordering::Relaxed);
            Ok(Some(ev))
        } else {
            self.conflicts.fetch_add(1, Ordering::Relaxed);
            Err(())
        }
    }
}

// ---------------------------------------------------------------------------
// Distribution-aware runtime planning.
// ---------------------------------------------------------------------------

/// Size of the machine a job is being placed on, derived from the resource
/// graph so estimators can treat jobs relative to it (e.g. wide jobs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MachineShape {
    /// Cores over all schedulable leaves.
    pub total_cores: u32,
    /// Cores of the largest single leaf (the widest single-vertex request).
    pub largest_node_cores: u32,
}

impl MachineShape {
    pub fn of(graph: &ResourceGraph) -> Self {
        graph
            .leaves()
            .map(|id| graph.vertex(id).capacity.cores)
            .fold(MachineShape::default(), |s, c| MachineShape {
                total_cores: s.total_cores.saturating_add(c),
                largest_node_cores: s.largest_node_cores.max(c),
            })
    }
}

/// The spans [`BackfillPolicy`] uses to place a job with an estimator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spans {
    /// Predicted span: decides whether and when the job can run.
    pub plan: u64,
    /// A job that starts now at `plan` must also start now at `guard`, or it
    /// is placed as the guard span allows (protects reservations it would
    /// otherwise be squeezed in front of). `guard <= plan` disables it.
    pub guard: u64,
    /// Span recorded for a job that starts now, when it also fits now; later
    /// jobs plan around it. `record <= plan` records `plan`.
    pub record: u64,
}

impl Spans {
    pub fn uniform(secs: u64) -> Self {
        Spans {
            plan: secs,
            guard: secs,
            record: secs,
        }
    }
}

/// Predicts how long a job will occupy its resources (never above walltime).
pub trait RuntimeEstimator: Send + Sync + std::fmt::Debug {
    fn name(&self) -> String;
    fn planned_secs(&self, job: &Job) -> u64;
    /// Spans for placing `job` on a machine of the given shape; defaults to
    /// [`planned_secs`](Self::planned_secs) for all three.
    fn spans_on(&self, job: &Job, _shape: MachineShape) -> Spans {
        Spans::uniform(self.planned_secs(job))
    }
    /// Called once per scheduling pass with the queued jobs, as
    /// [`Policy::begin_pass`] (default no-op).
    fn begin_pass(&self, _queue: &[&Job], _shape: MachineShape, _now: u64) {}
    fn observe(&self, _job: &Job, _actual_secs: u64) {}
    /// New span length for a job that has run `elapsed` s past its plan.
    fn extend_secs(&self, job: &Job, elapsed: u64) -> u64 {
        (elapsed.saturating_mul(2))
            .max(elapsed + 60)
            .min(job.walltime.secs())
            .max(elapsed + 1)
    }
}

impl RuntimeEstimator for QuantileEstimator {
    fn name(&self) -> String {
        format!("q{:.2}", self.theta)
    }
    fn planned_secs(&self, job: &Job) -> u64 {
        QuantileEstimator::planned_secs(self, job)
    }
    fn observe(&self, job: &Job, a: u64) {
        QuantileEstimator::observe(self, job, a)
    }
}

/// Learns each user's runtime/walltime ratio from completed jobs and plans
/// spans at the `theta` quantile (risk budget = 1 - theta: the chance a job
/// outlives its planned span). Overruns are extended, never killed early —
/// the walltime remains the hard limit, exactly as today.
///
/// Builds on Nissimov & Feitelson's probabilistic backfilling (2008) and
/// Tsafrir et al.'s system-generated predictions (2007), but lives inside
/// every vertex's planner and is learned online from the event stream.
#[derive(Debug)]
pub struct QuantileEstimator {
    pub theta: f64,
    pub window: usize,
    pub min_samples: usize,
    /// Never plan below this many seconds (or the walltime, if smaller).
    pub floor_secs: u64,
    users: RwLock<HashMap<String, VecDeque<f64>>>,
    global: RwLock<VecDeque<f64>>,
}

impl QuantileEstimator {
    pub fn new(theta: f64) -> Self {
        QuantileEstimator {
            theta,
            window: 32,
            min_samples: 4,
            floor_secs: 60,
            users: RwLock::new(HashMap::new()),
            global: RwLock::new(VecDeque::new()),
        }
    }

    fn quantile(v: &VecDeque<f64>, q: f64) -> f64 {
        let mut s: Vec<f64> = v.iter().copied().collect();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((s.len() as f64 - 1.0) * q).ceil() as usize;
        s[idx.min(s.len() - 1)]
    }

    /// Ratio quantile for a user (falls back to the global history, then 1.0).
    pub fn ratio(&self, user: &str) -> f64 {
        if let Some(h) = self.users.read().unwrap().get(user) {
            if h.len() >= self.min_samples {
                return Self::quantile(h, self.theta);
            }
        }
        let g = self.global.read().unwrap();
        if g.len() >= self.min_samples * 4 {
            Self::quantile(&g, self.theta)
        } else {
            1.0
        }
    }

    pub fn planned_secs(&self, job: &Job) -> u64 {
        let wall = job.walltime.secs();
        let p = (wall as f64 * self.ratio(&job.user)).ceil() as u64;
        p.max(self.floor_secs.min(wall)).min(wall).max(1)
    }

    pub fn observe(&self, job: &Job, actual: u64) {
        let wall = job.walltime.secs().max(1);
        let r = (actual as f64 / wall as f64).clamp(0.0, 1.0);
        let push = |q: &mut VecDeque<f64>, cap: usize| {
            q.push_back(r);
            while q.len() > cap {
                q.pop_front();
            }
        };
        push(
            self.users
                .write()
                .unwrap()
                .entry(job.user.clone())
                .or_default(),
            self.window,
        );
        push(&mut self.global.write().unwrap(), self.window * 16);
    }

    /// Overrun: double the elapsed time (at least +60s), capped at walltime.
    pub fn extend_secs(&self, job: &Job, elapsed: u64) -> u64 {
        (elapsed.saturating_mul(2))
            .max(elapsed + 60)
            .min(job.walltime.secs())
            .max(elapsed + 1)
    }
}

// ---------------------------------------------------------------------------
// One scheduling pass (controller loop and simulator share this code).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct PassOptions {
    /// Max reservations per pass (jobs beyond it may only start now).
    pub max_reservations: usize,
}

impl Default for PassOptions {
    fn default() -> Self {
        PassOptions {
            max_reservations: 32,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PassStats {
    pub started: u64,
    pub reserved: u64,
    pub extended: u64,
    pub conflicts: u64,
}

/// Conservative backfill with compression: extend overrunning predictions,
/// clear reservations, then place pending jobs in priority order (the first
/// `max_reservations` that can't start now get reservations). All changes
/// go through FSM events, so a pass is replayable.
pub fn backfill_pass(
    fsm: &Fsm,
    policy: &dyn Policy,
    applier: &PlanApplier,
    opts: PassOptions,
) -> PassStats {
    let now = fsm.now();
    let mut stats = PassStats::default();

    // 1. Overruns: running jobs past their planned span get extended.
    let overruns: Vec<(JobId, u64)> = fsm.read(|st| {
        st.jobs
            .values()
            .filter(|j| j.state == JobState::Running)
            .filter_map(|j| {
                let (s, d) = (j.planned_start?, j.planned_dur?);
                (s + d <= now).then(|| (j.id, s + policy.extend_secs(j, now - s)))
            })
            .collect()
    });
    for (job, new_end) in overruns {
        fsm.apply(Event::JobExtended { job, new_end });
        stats.extended += 1;
    }

    // 2. Re-plan: show the policy the queue with last pass's reservations,
    // then drop them (they are recomputed below).
    fsm.read(|st| {
        let queue: Vec<&Job> = st
            .jobs
            .values()
            .filter(|j| matches!(j.state, JobState::Pending | JobState::Reserved))
            .collect();
        policy.begin_pass(&queue, &st.graph, now);
    });
    let any_reserved = fsm.read(|st| st.jobs.values().any(|j| j.state == JobState::Reserved));
    if any_reserved {
        fsm.apply(Event::ReservationsCleared { now });
    }

    // 3. Place pending jobs in priority order.
    let mut queue: Vec<(u64, JobId)> =
        fsm.read(|st| st.pending_jobs().map(|j| (j.priority, j.id)).collect());
    queue.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut reservations = 0usize;
    for (_, id) in queue {
        let allow = reservations < opts.max_reservations;
        let plan = fsm.read(|st| {
            let job = st.jobs.get(&id)?;
            let decision = policy.place(&st.graph, job, now, allow);
            let read_set = st.graph.versions(decision.vertices());
            Some(Plan {
                job: id,
                base_index: 0,
                decision,
                read_set,
            })
        });
        let Some(plan) = plan else { continue };
        match applier.apply(fsm, &plan) {
            Ok(Some(Event::JobStarted { .. })) => stats.started += 1,
            Ok(Some(_)) => {
                stats.reserved += 1;
                reservations += 1;
            }
            Ok(None) => {}
            Err(()) => stats.conflicts += 1,
        }
    }
    stats
}

// ---------------------------------------------------------------------------
// Shadow mode: evaluate a candidate policy on live traffic, apply nothing.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ShadowDiff {
    pub job: JobId,
    pub now: u64,
    pub primary: Decision,
    pub candidate: Decision,
}

/// Wraps the live policy; every decision is also computed by `candidate`
/// against the same graph and time, and disagreements are recorded.
pub struct ShadowPolicy {
    pub primary: Arc<dyn Policy>,
    pub candidate: Arc<dyn Policy>,
    pub decisions: AtomicU64,
    pub diffs: Mutex<Vec<ShadowDiff>>,
}

impl ShadowPolicy {
    pub fn new(primary: Arc<dyn Policy>, candidate: Arc<dyn Policy>) -> Self {
        ShadowPolicy {
            primary,
            candidate,
            decisions: AtomicU64::new(0),
            diffs: Mutex::new(Vec::new()),
        }
    }
    pub fn agreement(&self) -> f64 {
        let n = self.decisions.load(Ordering::Relaxed).max(1) as f64;
        1.0 - self.diffs.lock().unwrap().len() as f64 / n
    }
}

impl Policy for ShadowPolicy {
    fn name(&self) -> String {
        format!(
            "shadow({} vs {})",
            self.primary.name(),
            self.candidate.name()
        )
    }
    fn planned_secs(&self, job: &Job) -> u64 {
        self.primary.planned_secs(job)
    }
    fn observe(&self, job: &Job, a: u64) {
        self.primary.observe(job, a);
        self.candidate.observe(job, a);
    }
    fn begin_pass(&self, queue: &[&Job], graph: &ResourceGraph, now: u64) {
        self.primary.begin_pass(queue, graph, now);
        self.candidate.begin_pass(queue, graph, now);
    }
    fn extend_secs(&self, job: &Job, e: u64) -> u64 {
        self.primary.extend_secs(job, e)
    }
    fn place(&self, graph: &ResourceGraph, job: &Job, now: u64, allow: bool) -> Decision {
        let p = self.primary.place(graph, job, now, allow);
        let c = self.candidate.place(graph, job, now, allow);
        self.decisions.fetch_add(1, Ordering::Relaxed);
        if !p.same_placement(&c) {
            self.diffs.lock().unwrap().push(ShadowDiff {
                job: job.id,
                now,
                primary: p.clone(),
                candidate: c,
            });
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnomon_core::{Job, Resources, Walltime};
    use std::time::Duration;

    #[test]
    fn broker_coalesces_duplicate_evals() {
        let broker = EvalBroker::new();
        let job = Job::new(
            JobId(1),
            "a",
            "u",
            Resources {
                cores: 1,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(Duration::from_secs(60)),
        );
        broker.enqueue(&job, 5);
        broker.enqueue(&job, 5); // same index -> coalesced
        broker.enqueue(&job, 4); // older index -> coalesced
        assert_eq!(broker.len(), 1);
        assert_eq!(broker.coalesced.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn optimistic_commit_detects_conflict() {
        let fsm = Fsm::new();
        fsm.apply(Event::NodeRegistered {
            name: "n0".into(),
            capacity: Resources {
                cores: 4,
                mem_mb: 1024,
                gpus: 0,
            },
        });
        let job = Job::new(
            JobId(1),
            "a",
            "u",
            Resources {
                cores: 1,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(Duration::from_secs(60)),
        );
        fsm.apply(Event::JobSubmitted(job.clone()));
        let snap = fsm.snapshot();
        let plan = Plan {
            job: JobId(1),
            base_index: snap.index,
            decision: Decision::Start {
                vertices: vec![1],
                start: gnomon_core::now_secs(),
                dur: None,
            },
            read_set: snap.graph().versions(&[1]),
        };
        for mode in [ConflictMode::GlobalIndex, ConflictMode::PerVertex] {
            let fsm2 = Fsm::new();
            for ev in [
                Event::NodeRegistered {
                    name: "n0".into(),
                    capacity: Resources {
                        cores: 4,
                        mem_mb: 1024,
                        gpus: 0,
                    },
                },
                Event::JobSubmitted(job.clone()),
            ] {
                fsm2.apply(ev);
            }
            let applier = PlanApplier::with_mode(mode);
            assert!(applier.apply(&fsm2, &plan).is_ok());
            // Same plan again: the job is no longer pending -> conflict in
            // both modes (the old applier counted this as committed).
            assert!(applier.apply(&fsm2, &plan).is_err());
            assert_eq!(applier.committed.load(Ordering::Relaxed), 1);
            assert_eq!(applier.conflicts.load(Ordering::Relaxed), 1);
        }
    }

    #[test]
    fn heartbeat_does_not_invalidate_plans_in_per_vertex_mode() {
        let mk = || {
            let fsm = Fsm::new();
            fsm.apply(Event::NodeRegistered {
                name: "n0".into(),
                capacity: Resources {
                    cores: 4,
                    mem_mb: 1024,
                    gpus: 0,
                },
            });
            fsm.apply(Event::JobSubmitted(Job::new(
                JobId(1),
                "a",
                "u",
                Resources {
                    cores: 1,
                    mem_mb: 0,
                    gpus: 0,
                },
                Walltime(Duration::from_secs(60)),
            )));
            fsm
        };
        let policy = BackfillPolicy::default();
        for (mode, ok) in [
            (ConflictMode::GlobalIndex, false),
            (ConflictMode::PerVertex, true),
        ] {
            let fsm = mk();
            let snap = fsm.snapshot();
            let plan = policy.schedule(&snap, snap.job(JobId(1)).unwrap());
            fsm.apply(Event::Heartbeat {
                name: "n0".into(),
                allocated: Resources::default(),
            });
            assert_eq!(
                PlanApplier::with_mode(mode).apply(&fsm, &plan).is_ok(),
                ok,
                "{mode:?}"
            );
        }
    }

    #[test]
    fn estimator_learns_user_ratio_and_respects_walltime() {
        let est = QuantileEstimator::new(0.9);
        let j = Job::new(
            JobId(1),
            "a",
            "alice",
            Resources {
                cores: 1,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(Duration::from_secs(10_000)),
        );
        assert_eq!(est.planned_secs(&j), 10_000); // no history -> walltime
        for k in 0..10 {
            est.observe(&j, 1000 + k * 100);
        } // ratios 0.10..0.19
        let p = est.planned_secs(&j);
        assert!((1800..=1900).contains(&p), "p90 of ratios ~0.19 -> {p}");
        assert!(est.extend_secs(&j, 9_000) <= 10_000);
    }

    #[test]
    fn shadow_records_disagreements_without_acting() {
        let fsm = Fsm::new();
        fsm.apply(Event::NodeRegistered {
            name: "n0".into(),
            capacity: Resources {
                cores: 4,
                mem_mb: 0,
                gpus: 0,
            },
        });
        let est = Arc::new(QuantileEstimator::new(0.5));
        let user_job = |id| {
            Job::new(
                JobId(id),
                "a",
                "bob",
                Resources {
                    cores: 4,
                    mem_mb: 0,
                    gpus: 0,
                },
                Walltime(Duration::from_secs(1000)),
            )
        };
        for k in 0..8 {
            est.observe(&user_job(100 + k), 100);
        } // bob uses ~10% of walltime
        let shadow = ShadowPolicy::new(
            Arc::new(BackfillPolicy::default()),
            Arc::new(BackfillPolicy::with_estimator(48 * 3600, est)),
        );
        // Node is reserved from now+500: a full-walltime plan cannot fit before
        // it, a 100 s prediction can.
        let now = gnomon_core::now_secs();
        fsm.apply(Event::JobSubmitted(user_job(9)));
        assert!(
            fsm.apply_checked(Event::JobReserved {
                job: JobId(9),
                vertices: vec![1],
                start: now + 500,
                dur: None
            })
            .accepted
        );
        fsm.apply(Event::JobSubmitted(user_job(1)));
        let snap = fsm.snapshot();
        let d = shadow.place(snap.graph(), snap.job(JobId(1)).unwrap(), snap.now, true);
        assert!(
            matches!(d, Decision::Reserve { .. }),
            "primary decision is what gets applied"
        );
        let diffs = shadow.diffs.lock().unwrap();
        assert_eq!(diffs.len(), 1);
        assert!(
            matches!(diffs[0].candidate, Decision::Start { .. }),
            "candidate would backfill now"
        );
    }

    fn cluster(nodes: u32, cores: u32, now: u64) -> Fsm {
        let fsm = Fsm::with_clock(Arc::new(gnomon_core::ManualClock::new(now)));
        for n in 0..nodes {
            fsm.apply(Event::NodeRegistered {
                name: format!("n{n}"),
                capacity: Resources {
                    cores,
                    mem_mb: 0,
                    gpus: 0,
                },
            });
        }
        fsm
    }

    fn sized_job(id: u64, cores: u32, nodes: u32, wall: u64) -> Job {
        let mut j = Job::new(
            JobId(id),
            "j",
            "u",
            Resources {
                cores,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(Duration::from_secs(wall)),
        );
        j.nodes = nodes;
        j.submit_time = 0;
        j
    }

    #[test]
    fn gang_starts_now_or_reserves_in_both_policy_paths() {
        let fsm = cluster(4, 4, 1000);
        // n0 (vertex 1) is busy until 1500.
        fsm.apply(Event::JobSubmitted(sized_job(1, 4, 1, 500)));
        assert!(
            fsm.apply_checked(Event::JobStarted {
                job: JobId(1),
                vertices: vec![1],
                start: 1000,
                dur: None
            })
            .accepted
        );
        let policy = BackfillPolicy::default();
        let three = sized_job(2, 4, 3, 100);
        let four = sized_job(3, 4, 4, 100);
        fsm.read(|st| {
            for allow in [true, false] {
                assert_eq!(
                    policy.place(&st.graph, &three, 1000, allow),
                    Decision::Start {
                        vertices: vec![2, 3, 4],
                        start: 1000,
                        dur: None
                    },
                    "allow_reserve={allow}"
                );
            }
            // Four nodes only once n0 frees up.
            assert_eq!(
                policy.place(&st.graph, &four, 1000, true),
                Decision::Reserve {
                    vertices: vec![1, 2, 3, 4],
                    start: 1500,
                    dur: None
                }
            );
            assert_eq!(policy.place(&st.graph, &four, 1000, false), Decision::Defer);
            // Beyond the reservation window: defer.
            let short = BackfillPolicy::new(100);
            assert_eq!(short.place(&st.graph, &four, 1000, true), Decision::Defer);
        });
    }

    #[test]
    fn backfill_pass_starts_gang_and_counts_it_once() {
        let fsm = cluster(4, 4, 1000);
        fsm.apply(Event::JobSubmitted(sized_job(1, 2, 4, 100)));
        let stats = backfill_pass(
            &fsm,
            &BackfillPolicy::default(),
            &PlanApplier::new(),
            PassOptions::default(),
        );
        assert_eq!(stats.started, 1);
        fsm.read(|st| {
            assert_eq!(st.jobs[&JobId(1)].state, JobState::Running);
            assert_eq!(st.allocations[&JobId(1)].len(), 4);
            assert_eq!(st.assignments.len(), 4);
        });
    }

    #[test]
    fn conservative_backfill_protects_gang_reservation() {
        // n0 is busy until 1500; a 4-node gang (higher priority) must keep its
        // reservation at 1500 while single-node jobs backfill around it, and
        // a backfill candidate that would delay it must wait.
        let fsm = cluster(4, 4, 1000);
        fsm.apply(Event::JobSubmitted(sized_job(1, 4, 1, 500)));
        fsm.apply(Event::JobStarted {
            job: JobId(1),
            vertices: vec![1],
            start: 1000,
            dur: None,
        });
        let mut gang = sized_job(2, 4, 4, 1000);
        gang.priority = 10;
        fsm.apply(Event::JobSubmitted(gang));
        fsm.apply(Event::JobSubmitted(sized_job(3, 4, 1, 400))); // fits before 1500
        fsm.apply(Event::JobSubmitted(sized_job(4, 4, 1, 900))); // would delay the gang
        let stats = backfill_pass(
            &fsm,
            &BackfillPolicy::default(),
            &PlanApplier::new(),
            PassOptions::default(),
        );
        assert_eq!(stats.conflicts, 0);
        fsm.read(|st| {
            let j = |id| &st.jobs[&JobId(id)];
            assert_eq!(j(2).state, JobState::Reserved);
            assert_eq!(j(2).planned_start, Some(1500));
            assert_eq!(j(3).state, JobState::Running);
            assert_eq!(j(4).state, JobState::Reserved);
            assert_eq!(j(4).planned_start, Some(2500));
        });
    }

    #[test]
    fn shadow_policy_compares_gang_placements() {
        let fsm = cluster(4, 4, 1000);
        let shadow = ShadowPolicy::new(
            Arc::new(BackfillPolicy::default()),
            Arc::new(BackfillPolicy::new(10)),
        );
        let gang = sized_job(1, 4, 2, 100);
        let d = fsm.read(|st| shadow.place(&st.graph, &gang, 1000, true));
        assert_eq!(d.vertices().len(), 2);
        assert_eq!(shadow.agreement(), 1.0);
    }

    /// Estimator with fixed spans, to test how `BackfillPolicy` uses them.
    #[derive(Debug)]
    struct FixedSpans(Spans);
    impl RuntimeEstimator for FixedSpans {
        fn name(&self) -> String {
            "fixed".into()
        }
        fn planned_secs(&self, _job: &Job) -> u64 {
            self.0.plan
        }
        fn spans_on(&self, _job: &Job, _shape: MachineShape) -> Spans {
            self.0
        }
    }

    /// One 4-core node, fully reserved from t = 1500 by job 9; job 1
    /// (4 cores, walltime 10 000 s) is pending at t = 1000.
    fn reserved_node() -> Fsm {
        let fsm = Fsm::with_clock(Arc::new(gnomon_core::ManualClock::new(1000)));
        fsm.apply(Event::NodeRegistered {
            name: "n0".into(),
            capacity: Resources {
                cores: 4,
                mem_mb: 0,
                gpus: 0,
            },
        });
        for id in [9, 1] {
            fsm.apply(Event::JobSubmitted(Job::new(
                JobId(id),
                "a",
                "bob",
                Resources {
                    cores: 4,
                    mem_mb: 0,
                    gpus: 0,
                },
                Walltime(Duration::from_secs(10_000)),
            )));
        }
        assert!(
            fsm.apply_checked(Event::JobReserved {
                job: JobId(9),
                vertices: vec![1],
                start: 1500,
                dur: None
            })
            .accepted
        );
        fsm
    }

    fn place_with(spans: Spans) -> Decision {
        let fsm = reserved_node();
        let pol = BackfillPolicy::with_estimator(48 * 3600, Arc::new(FixedSpans(spans)));
        let snap = fsm.snapshot();
        pol.place(snap.graph(), snap.job(JobId(1)).unwrap(), snap.now, true)
    }

    #[test]
    fn machine_shape_reads_the_graph() {
        let fsm = reserved_node();
        fsm.apply(Event::NodeRegistered {
            name: "n1".into(),
            capacity: Resources {
                cores: 16,
                mem_mb: 0,
                gpus: 0,
            },
        });
        let shape = fsm.read(|st| MachineShape::of(&st.graph));
        assert_eq!(
            shape,
            MachineShape {
                total_cores: 20,
                largest_node_cores: 16
            }
        );
    }

    #[test]
    fn guard_span_blocks_backfill_in_front_of_a_reservation() {
        let plain = Spans::uniform(100);
        assert!(matches!(
            place_with(plain),
            Decision::Start {
                start: 1000,
                dur: Some(100),
                ..
            }
        ));
        // 100 s fits before the reservation at 1500, a 600 s guard does not:
        // the job is not squeezed in, and keeps its predicted span.
        let d = place_with(Spans {
            guard: 600,
            ..plain
        });
        assert!(
            matches!(d, Decision::Reserve { dur: Some(100), .. }),
            "{d:?}"
        );
        // A guard that also fits lets it start.
        let d = place_with(Spans {
            guard: 400,
            ..plain
        });
        assert!(matches!(d, Decision::Start { dur: Some(100), .. }), "{d:?}");
    }

    #[test]
    fn record_span_is_used_only_when_it_fits() {
        let plain = Spans::uniform(100);
        let d = place_with(Spans {
            record: 400,
            ..plain
        });
        assert!(matches!(d, Decision::Start { dur: Some(400), .. }), "{d:?}");
        // Recording 800 s would overlap the reservation: keep the prediction.
        let d = place_with(Spans {
            record: 800,
            ..plain
        });
        assert!(matches!(d, Decision::Start { dur: Some(100), .. }), "{d:?}");
    }

    #[test]
    fn backfill_pass_shows_the_queue_before_clearing_reservations() {
        #[derive(Default)]
        struct Spy(Mutex<Vec<(JobId, JobState, Option<u64>)>>);
        impl Policy for Spy {
            fn place(&self, _: &ResourceGraph, _: &Job, _: u64, _: bool) -> Decision {
                Decision::Defer
            }
            fn begin_pass(&self, queue: &[&Job], _: &ResourceGraph, _: u64) {
                let mut seen = self.0.lock().unwrap();
                seen.extend(queue.iter().map(|j| (j.id, j.state, j.planned_start)));
                seen.sort_by_key(|x| x.0);
            }
        }
        let fsm = reserved_node();
        let spy = Spy::default();
        backfill_pass(&fsm, &spy, &PlanApplier::new(), PassOptions::default());
        assert_eq!(
            *spy.0.lock().unwrap(),
            vec![
                (JobId(1), JobState::Pending, None),
                (JobId(9), JobState::Reserved, Some(1500)),
            ]
        );
    }
}
