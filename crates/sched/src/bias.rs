//! Bias-corrected runtime prediction (`results/EVALUATION.md` §3b).
//!
//! [`QuantileEstimator`](crate::QuantileEstimator) plans each job at a
//! quantile of its user's runtime/walltime ratios. On real traces it is
//! inconsistent and lets the reservations of wide jobs slip, because the
//! spans of narrow backfilled jobs are systematically too short. Following
//! Tsafrir et al. (2007) — estimation *bias* matters more than accuracy —
//! [`BiasCorrectedEstimator`] separates the two:
//!
//! 1. **Base prediction**: an exponentially weighted mean of
//!    `ln(runtime / walltime)` learned over a feature hierarchy
//!    (global → walltime bucket → walltime × cores bucket → user →
//!    user × walltime bucket), each level shrunk toward its parent so new
//!    users and rare buckets fall back gracefully. The job name is not used
//!    (it is `swf` in archive traces).
//! 2. **Bias correction**: an online quantile tracker on the signed log-error
//!    of the base prediction (per user, shrunk toward a global tracker). The
//!    update is the gradient of the pinball loss at level `tau`, so an
//!    under-prediction costs `tau / (1 - tau)` times an over-prediction and
//!    the long-run share of overrunning jobs converges to `1 - tau`
//!    (cf. adaptive conformal inference, Gibbs & Candès 2021). A second
//!    tracker at the stricter `guard_tau` gives a *cautious* span.
//! 3. **Reservation protection** through [`Spans`]: by default a job starts
//!    on its predicted span but is recorded at its cautious span, so later
//!    reservations rest on a cautious estimate ([`GuardMode::Record`]); an
//!    overrunning job is re-planned to its walltime; and once the
//!    reservation of a *wide* job has slipped (moved later, which walltime
//!    planning never does), new starts must fit at their cautious span and
//!    are recorded at walltime until that job has started. Planning wide
//!    jobs themselves at walltime is available (`wide_at_walltime`) but off:
//!    on the E1 traces it made wide jobs wait longer.
//!
//! Spans are never planned above the walltime nor below `floor_secs`.
//!
//! # Wide-job shield
//!
//! The protection above is reactive: it acts once a wide reservation has
//! slipped, and on the E1 traces wide jobs still wait longer than under
//! walltime planning in some windows. [`WideSafeBackfill`] wraps the
//! predictor policy with a structural guarantee instead. It keeps a second,
//! *conservative* view of the machine (the "shield") next to the optimistic
//! one the predictor plans on:
//!
//! * every running job occupies the shield until `start + walltime` (an
//!   upper bound of when it really ends, since jobs are killed at walltime);
//! * each queued wide job holds a **protected slot** for its walltime span:
//!   its earliest fit in the shield after the higher-priority queued jobs
//!   (narrow ones at their predicted span by default, see [`NarrowAhead`]),
//!   never later than the slot it held in the previous pass (slots are
//!   re-placed one at a time, earlier only);
//! * every start, whatever the predictor says, is **admitted** only if the
//!   job's *walltime* span fits now in the shield next to all protected slots
//!   (a wide job's own slot excluded). An admitted job is booked there at its
//!   walltime.
//!
//! Prediction still decides everything else (which narrow jobs backfill in
//! front of narrow reservations, how long running jobs are planned to take,
//! where narrow jobs are reserved). Consequence (*no-slip*): a wide job never
//! starts later than its protected slot, and its slot never moves later. All
//! occupants of the shield are upper bounds of real occupancy, slots are
//! jointly feasible with them, and anything that could fill a slot must first
//! be admitted around it; so between scheduling passes nothing can take a
//! slot's room, and the pass at (or before) the slot's start finds the wide
//! job fitting at walltime and starts it (see [`WideSafeBackfill`]).

use crate::{BackfillPolicy, Decision, MachineShape, Policy, RuntimeEstimator, Spans};
use gnomon_core::{Job, JobId, JobState, Resources};
use gnomon_graph::{ResourceGraph, VertexId};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// Number of walltime buckets (log2 steps from 10 minutes).
const WALL_BUCKETS: usize = 8;
/// Number of core-count buckets (log2 steps from 1 core).
const CORE_BUCKETS: usize = 8;
/// Smallest runtime/walltime ratio considered (jobs that die at once).
const MIN_RATIO: f64 = 1e-3;

/// Tunables of [`BiasCorrectedEstimator`]; the defaults are the variant
/// selected on the E1 traces (§3b).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiasConfig {
    /// Planning level: target share of jobs that finish within their span.
    pub tau: f64,
    /// Stricter level of the cautious span (see [`GuardMode`]). `None`
    /// makes the cautious span equal to the planned one.
    pub guard_tau: Option<f64>,
    /// How the cautious span is used.
    pub guard_mode: GuardMode,
    /// Step of the pinball-loss quantile tracker, in log units.
    pub lr: f64,
    /// Floor of the EWMA weight of the user-level cells (recency, as in
    /// Tsafrir et al.'s "last two jobs" predictor).
    pub user_alpha: f64,
    /// Floor of the EWMA weight of the feature-level cells.
    pub feature_alpha: f64,
    /// Shrinkage strength: pseudo-observations of the parent level.
    pub prior_n: f64,
    /// A job is *wide* when it requests at least this fraction of the
    /// largest node's cores.
    pub wide_fraction: f64,
    /// Plan wide jobs at walltime.
    pub wide_at_walltime: bool,
    /// Once a wide job's reservation has slipped, new starts must fit at
    /// their cautious span and are recorded at walltime until it starts.
    pub protect_slipped: bool,
    /// A job that outlives its prediction is re-planned to its walltime
    /// (instead of doubling its elapsed time).
    pub overrun_to_walltime: bool,
    /// Never plan below this many seconds (or the walltime, if smaller).
    pub floor_secs: u64,
}

/// Use of the cautious (`guard_tau`) span in [`Spans`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardMode {
    /// A job may start now only if its cautious span also fits now.
    Block,
    /// A job starts on its predicted span but is recorded at its cautious
    /// span when that also fits, so later reservations plan around it.
    Record,
}

impl Default for BiasConfig {
    fn default() -> Self {
        BiasConfig {
            tau: 0.7,
            guard_tau: Some(0.9),
            guard_mode: GuardMode::Record,
            lr: 0.05,
            user_alpha: 0.3,
            feature_alpha: 0.02,
            prior_n: 3.0,
            wide_fraction: 0.25,
            wide_at_walltime: false,
            protect_slipped: true,
            overrun_to_walltime: true,
            floor_secs: 60,
        }
    }
}

/// One cell of the feature hierarchy: EWMA of the log-ratio and two
/// quantile-tracked offsets of the residual (plan level, guard level).
#[derive(Clone, Copy, Debug, Default)]
struct Cell {
    n: f64,
    mean: f64,
    off: [f64; 2],
}

impl Cell {
    fn effective_n(&self, alpha: f64) -> f64 {
        self.n.min(1.0 / alpha)
    }

    /// Shrink this cell's mean toward `parent` (James–Stein style).
    fn shrink(&self, parent: f64, alpha: f64, k: f64) -> f64 {
        let n = self.effective_n(alpha);
        (n * self.mean + k * parent) / (n + k)
    }

    fn update_mean(&mut self, y: f64, alpha: f64) {
        self.n += 1.0;
        let a = (1.0 / self.n).max(alpha);
        self.mean += a * (y - self.mean);
    }
}

#[derive(Clone, Debug, Default)]
struct UserCells {
    all: Cell,
    by_wall: [Cell; WALL_BUCKETS],
}

#[derive(Debug, Default)]
struct State {
    global: Cell,
    by_wall: [Cell; WALL_BUCKETS],
    by_wall_cores: [[Cell; CORE_BUCKETS]; WALL_BUCKETS],
    users: HashMap<String, UserCells>,
}

/// Online, bias-corrected runtime predictor with reservation protection.
#[derive(Debug)]
pub struct BiasCorrectedEstimator {
    pub cfg: BiasConfig,
    state: RwLock<State>,
    /// Set by `begin_pass` while a queued wide job's reservation has slipped.
    protect: AtomicBool,
    /// Last seen reservation start of each queued wide job, and whether it
    /// has slipped since it was first reserved.
    wide_resv: Mutex<HashMap<JobId, (u64, bool)>>,
}

fn log2_bucket(x: f64, n: usize) -> usize {
    if x <= 1.0 {
        0
    } else {
        (x.log2().floor() as usize).min(n - 1)
    }
}

fn wall_bucket(job: &Job) -> usize {
    log2_bucket(job.walltime.secs() as f64 / 600.0, WALL_BUCKETS)
}

/// Bucket of the whole job's cores (a gang's request is per node).
fn core_bucket(job: &Job) -> usize {
    let cores = u64::from(job.request.cores.max(1)) * u64::from(job.node_count());
    log2_bucket(cores as f64, CORE_BUCKETS)
}

fn log_ratio(job: &Job, actual: u64) -> f64 {
    let wall = job.walltime.secs().max(1) as f64;
    (actual as f64 / wall).clamp(MIN_RATIO, 1.0).ln()
}

impl BiasCorrectedEstimator {
    pub fn new(cfg: BiasConfig) -> Self {
        BiasCorrectedEstimator {
            cfg,
            state: RwLock::new(State::default()),
            protect: AtomicBool::new(false),
            wide_resv: Mutex::new(HashMap::new()),
        }
    }

    /// Base prediction of `ln(runtime / walltime)` for `job`.
    fn base(&self, st: &State, job: &Job) -> f64 {
        let (wb, cb) = (wall_bucket(job), core_bucket(job));
        let (fa, ua, k) = (
            self.cfg.feature_alpha,
            self.cfg.user_alpha,
            self.cfg.prior_n,
        );
        let m0 = st.global.mean;
        let m1 = st.by_wall[wb].shrink(m0, fa, k);
        let m2 = st.by_wall_cores[wb][cb].shrink(m1, fa, k);
        match st.users.get(&job.user) {
            None => m2,
            Some(u) => {
                let m3 = u.all.shrink(m2, ua, k);
                u.by_wall[wb].shrink(m3, ua, k)
            }
        }
    }

    /// Bias offset at level `i` (0 = plan, 1 = cautious): the user's tracked
    /// residual quantile shrunk toward the global one.
    fn offset(&self, st: &State, user: &str, i: usize) -> f64 {
        let g = st.global.off[i];
        match st.users.get(user) {
            None => g,
            Some(u) => {
                let n = u.all.effective_n(self.cfg.user_alpha);
                (n * u.all.off[i] + self.cfg.prior_n * g) / (n + self.cfg.prior_n)
            }
        }
    }

    /// Predicted `ln(runtime / walltime)` at level `i`, `None` before any
    /// observation (plan at walltime).
    fn predicted_log_ratio(&self, job: &Job, i: usize) -> Option<f64> {
        let st = self.state.read().unwrap_or_else(|e| e.into_inner());
        if st.global.n == 0.0 {
            return None;
        }
        Some(self.base(&st, job) + self.offset(&st, &job.user, i))
    }

    fn span(&self, job: &Job, log_ratio: Option<f64>) -> u64 {
        let wall = job.walltime.secs();
        let Some(lr) = log_ratio else {
            return wall.max(1);
        };
        let p = (wall as f64 * lr.min(0.0).exp()).ceil() as u64;
        p.max(self.cfg.floor_secs.min(wall)).min(wall).max(1)
    }

    /// Wide: a multi-node gang, or a job taking at least `wide_fraction` of
    /// the largest node (on a single-vertex machine, of the whole machine).
    fn is_wide(&self, job: &Job, shape: MachineShape) -> bool {
        job.node_count() > 1
            || (shape.largest_node_cores > 0
                && job.request.cores as f64
                    >= self.cfg.wide_fraction * shape.largest_node_cores as f64)
    }

    /// Current planning-level bias offset for `user`, in log units.
    pub fn bias_offset(&self, user: &str) -> f64 {
        let st = self.state.read().unwrap_or_else(|e| e.into_inner());
        self.offset(&st, user, 0)
    }

    /// Whether reservation protection is active (a wide job has slipped).
    pub fn protecting(&self) -> bool {
        self.protect.load(Ordering::Relaxed)
    }

    /// Span at the planning level, ignoring machine shape.
    pub fn planned_secs(&self, job: &Job) -> u64 {
        self.span(job, self.predicted_log_ratio(job, 0))
    }

    /// Span at the cautious level (never below the planned span).
    pub fn cautious_secs(&self, job: &Job) -> u64 {
        let plan = self.planned_secs(job);
        match self.cfg.guard_tau {
            Some(_) => self.span(job, self.predicted_log_ratio(job, 1)).max(plan),
            None => plan,
        }
    }

    /// Learn from a completed job: first score the prediction it would have
    /// had (its signed log-error drives the pinball update), then fold the
    /// outcome into the base hierarchy.
    pub fn observe(&self, job: &Job, actual: u64) {
        let y = log_ratio(job, actual);
        let (wb, cb) = (wall_bucket(job), core_bucket(job));
        let (fa, ua, lr) = (self.cfg.feature_alpha, self.cfg.user_alpha, self.cfg.lr);
        let taus = [self.cfg.tau, self.cfg.guard_tau.unwrap_or(self.cfg.tau)];
        let mut st = self.state.write().unwrap_or_else(|e| e.into_inner());
        let first = st.global.n == 0.0;
        let resid = y - self.base(&st, job);
        let global_off = st.global.off;
        let pinball = |off: &mut [f64; 2]| {
            for (o, tau) in off.iter_mut().zip(taus) {
                *o += lr * if resid < *o { tau - 1.0 } else { tau };
            }
        };
        if !first {
            pinball(&mut st.global.off);
        }
        let user = st.users.entry(job.user.clone()).or_insert_with(|| {
            let mut u = UserCells::default();
            u.all.off = global_off; // a new user starts from the global bias
            u
        });
        if !first {
            pinball(&mut user.all.off);
        }
        user.all.update_mean(y, ua);
        user.by_wall[wb].update_mean(y, ua);
        st.global.update_mean(y, fa);
        st.by_wall[wb].update_mean(y, fa);
        st.by_wall_cores[wb][cb].update_mean(y, fa);
    }

    /// Track the reservations of queued wide jobs; returns whether any of
    /// them has slipped since it was first reserved.
    fn track_wide_reservations(&self, queue: &[&Job], shape: MachineShape) -> bool {
        let mut seen = self.wide_resv.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = HashMap::new();
        for j in queue.iter().filter(|j| self.is_wide(j, shape)) {
            let prev = seen.get(&j.id).copied();
            let entry = match (j.state, j.planned_start, prev) {
                (JobState::Reserved, Some(r), Some((last, slipped))) => (r, slipped || r > last),
                (JobState::Reserved, Some(r), None) => (r, false),
                (_, _, Some(p)) => p,
                _ => continue,
            };
            next.insert(j.id, entry);
        }
        *seen = next;
        seen.values().any(|&(_, slipped)| slipped)
    }
}

impl RuntimeEstimator for BiasCorrectedEstimator {
    fn name(&self) -> String {
        let mut s = format!("bc{:.2}", self.cfg.tau);
        if let Some(g) = self.cfg.guard_tau {
            let mode = match self.cfg.guard_mode {
                GuardMode::Block => "b",
                GuardMode::Record => "r",
            };
            s += &format!("-{mode}{g:.2}");
        }
        if self.cfg.wide_at_walltime {
            s += &format!("-w{:.2}", self.cfg.wide_fraction);
        }
        if self.cfg.protect_slipped {
            s += "-p";
        }
        if !self.cfg.overrun_to_walltime {
            s += "-x2";
        }
        s
    }

    fn planned_secs(&self, job: &Job) -> u64 {
        BiasCorrectedEstimator::planned_secs(self, job)
    }

    fn spans_on(&self, job: &Job, shape: MachineShape) -> Spans {
        let wall = job.walltime.secs().max(1);
        if self.cfg.wide_at_walltime && self.is_wide(job, shape) {
            return Spans::uniform(wall);
        }
        let plan = self.planned_secs(job);
        let cautious = self.cautious_secs(job);
        if self.protecting() {
            return Spans {
                plan,
                guard: cautious,
                record: wall,
            };
        }
        match self.cfg.guard_mode {
            GuardMode::Block => Spans {
                plan,
                guard: cautious,
                record: plan,
            },
            GuardMode::Record => Spans {
                plan,
                guard: plan,
                record: cautious,
            },
        }
    }

    fn begin_pass(&self, queue: &[&Job], shape: MachineShape, _now: u64) {
        let slipped = self.cfg.protect_slipped && self.track_wide_reservations(queue, shape);
        self.protect.store(slipped, Ordering::Relaxed);
    }

    fn observe(&self, job: &Job, actual: u64) {
        BiasCorrectedEstimator::observe(self, job, actual)
    }

    fn extend_secs(&self, job: &Job, elapsed: u64) -> u64 {
        let wall = job.walltime.secs();
        let next = if self.cfg.overrun_to_walltime {
            wall
        } else {
            elapsed.saturating_mul(2).max(elapsed + 60).min(wall)
        };
        next.max(elapsed + 1)
    }
}

// ---------------------------------------------------------------------------
// Wide-job shield: predictor backfill that never pushes back a wide job's
// walltime-planned reservation.
// ---------------------------------------------------------------------------

/// Reservation depth of the shield's slot plan (as `PassOptions`).
const SHIELD_DEPTH: usize = 32;

/// Counters of [`WideSafeBackfill`] (cumulative over its lifetime).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShieldStats {
    /// Wide jobs that started holding a protected slot.
    pub protected_starts: u64,
    /// Of those, jobs that started after their slot (the no-slip guarantee
    /// says this stays 0 when the machine runs jobs within their walltime).
    pub late_starts: u64,
    /// Wide jobs that started without ever getting a slot (beyond the
    /// reservation depth, or fitting at once).
    pub unprotected_starts: u64,
    /// Starts the predictor proposed that the shield refused.
    pub refused: u64,
    /// Wide starts the shield made although the predictor's own plan had
    /// not started them (their slot was due).
    pub forced: u64,
    /// Slots that no longer fitted and were dropped (only when capacity
    /// shrinks under them, e.g. a node leaves).
    pub dropped: u64,
}

#[derive(Clone, Debug)]
struct Slot {
    vertices: Vec<VertexId>,
    start: u64,
    dur: u64,
    req: Resources,
}

#[derive(Debug, Default)]
struct Shield {
    /// Walltime of every job seen queued that may still occupy the machine.
    walls: HashMap<JobId, u64>,
    /// Protected slot of each queued wide job (ordered: deterministic).
    slots: BTreeMap<JobId, Slot>,
    /// This pass's conservative view: running jobs at walltime, protected
    /// slots, and the jobs admitted so far in this pass at walltime.
    view: Option<ResourceGraph>,
    /// Wide jobs that fit now at walltime and are still to be placed in
    /// this pass; while any is pending, reservations of other jobs are
    /// deferred so that the optimistic plan cannot block their start.
    due: HashSet<JobId>,
    stats: ShieldStats,
}

fn release(g: &mut ResourceGraph, job: JobId) {
    let leaves: Vec<VertexId> = g.leaves().collect();
    for v in leaves {
        g.vertex_mut(v).planner.release_job(job.0, 0);
    }
}

fn book(
    g: &mut ResourceGraph,
    vs: &[VertexId],
    req: &Resources,
    start: u64,
    dur: u64,
    job: JobId,
) -> bool {
    g.commit_job(vs, req, start, dur, Some(job.0), &[])
}

fn fits_at(g: &ResourceGraph, vs: &[VertexId], req: &Resources, start: u64, dur: u64) -> bool {
    vs.iter()
        .all(|&v| g.vertex(v).earliest_fit(req, start, dur) == Some(start))
}

impl Shield {
    fn wall(&self, job: &Job) -> u64 {
        self.walls
            .get(&job.id)
            .copied()
            .unwrap_or_else(|| job.walltime.secs().max(1))
    }

    /// Lift `job`'s own slot out of the view (it does not block itself).
    fn lift(&mut self, job: JobId) -> Option<Slot> {
        let slot = self.slots.get(&job).cloned()?;
        if let Some(view) = self.view.as_mut() {
            release(view, job);
        }
        Some(slot)
    }

    fn restore(&mut self, job: JobId, slot: Option<Slot>) {
        if let (Some(s), Some(view)) = (slot, self.view.as_mut()) {
            if !book(view, &s.vertices, &s.req, s.start, s.dur, job) {
                self.slots.remove(&job);
                self.stats.dropped += 1;
            }
        }
    }

    /// Admit a start of `job` on `vs` now: its walltime span must fit in the
    /// view beside every other protected slot. Books it on success.
    fn admit(&mut self, job: &Job, vs: &[VertexId], now: u64, wide: bool) -> bool {
        let wall = self.wall(job);
        let own = self.lift(job.id);
        let Some(view) = self.view.as_mut() else {
            return true;
        };
        if fits_at(view, vs, &job.request, now, wall)
            && book(view, vs, &job.request, now, wall, job.id)
        {
            match &own {
                Some(s) => {
                    self.stats.protected_starts += 1;
                    self.stats.late_starts += u64::from(now > s.start);
                }
                None if wide => self.stats.unprotected_starts += 1,
                None => {}
            }
            self.slots.remove(&job.id);
            return true;
        }
        self.restore(job.id, own);
        false
    }

    /// Where `job`'s walltime span fits in the view from `now` (its own slot
    /// excluded).
    fn earliest(&mut self, job: &Job, now: u64) -> Option<(Vec<VertexId>, u64)> {
        let wall = self.wall(job);
        let own = self.lift(job.id);
        let fit =
            self.view
                .as_ref()?
                .match_gang(&job.request, job.node_count(), now, wall, job.id.0);
        self.restore(job.id, own);
        fit
    }
}

/// Span at which [`WideSafeBackfill`] plans queued narrow jobs of higher
/// priority ahead of a wide job when it computes the wide job's slot. These
/// placements are not protected (they only decide where a slot first goes),
/// so the choice trades wide-job wait against narrow-job wait; the no-slip
/// guarantee holds for all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NarrowAhead {
    /// At walltime: a new slot is the reservation walltime planning would
    /// give the wide job from the current state.
    Walltime,
    /// At the predictor's cautious span.
    Cautious,
    /// At the predictor's planned span (default): a new slot is where the
    /// predictor's own plan would put the wide job if running jobs ran to
    /// walltime.
    Planned,
    /// Not at all: wide jobs are planned before every narrow job.
    Skip,
}

impl NarrowAhead {
    fn label(self) -> &'static str {
        match self {
            NarrowAhead::Walltime => "wall",
            NarrowAhead::Cautious => "caut",
            NarrowAhead::Planned => "plan",
            NarrowAhead::Skip => "first",
        }
    }
}

/// Backfill with a [`BiasCorrectedEstimator`] in which no prediction-driven
/// start can delay a wide job past its *protected slot* (module docs,
/// "Wide-job shield"). Opt-in; the default policy is untouched.
///
/// **Guarantee (no-slip).** Assume running jobs end by `start + walltime`
/// and a pass runs whenever a job finishes or arrives (as in the controller
/// loop and the simulator). Then a wide job that holds a slot starting at
/// `s` starts at or before `s`, and its slot never moves later. Sketch: the
/// view's occupants (running jobs at walltime, admitted starts at walltime,
/// slots) only bound real occupancy from above, the slots are feasible
/// together with them, and nothing enters the machine without fitting
/// around them. Let `t0 <= s` be the last pass before `s`. No job finishes
/// in `(t0, s]` (that would be a pass) and, by induction on slot starts, no
/// other unstarted slot begins there; so over `[t0, s]` the view holds only
/// what runs at `t0`, which leaves the slot's room free from `t0` on: the
/// wide job is admitted at `t0`. The shield starts it even if the optimistic
/// plan would not (reservations ahead of it are deferred in that pass).
///
/// What it does **not** (and cannot) guarantee: that wide jobs wait no
/// longer than under walltime planning on every workload. Any policy that
/// ever starts a job the walltime policy would not start at that moment (or
/// holds back one it would start) runs a different schedule from then on,
/// and an adversary can append a wide job that fits exactly the room the
/// walltime schedule has at that instant and the other does not; that job
/// alone makes the wide-job mean worse. So "never worse than the baseline"
/// holds for every input only for the baseline itself. The shield bounds
/// the part prediction controls: once a wide job holds a slot, prediction
/// errors cannot delay it. Its *first* slot depends on the state the
/// predictor has produced, and wide jobs beyond the reservation depth hold
/// no slot until they reach it.
pub struct WideSafeBackfill {
    inner: BackfillPolicy,
    est: Arc<BiasCorrectedEstimator>,
    /// Reservation depth of the plan that assigns new slots.
    pub depth: usize,
    /// How queued narrow jobs of higher priority are planned ahead of a
    /// wide job when its slot is (re)computed.
    pub ahead: NarrowAhead,
    shield: Mutex<Shield>,
}

impl WideSafeBackfill {
    pub fn new(window_secs: u64, est: Arc<BiasCorrectedEstimator>) -> Self {
        WideSafeBackfill {
            inner: BackfillPolicy::with_estimator(window_secs, est.clone()),
            est,
            depth: SHIELD_DEPTH,
            ahead: NarrowAhead::Planned,
            shield: Mutex::new(Shield::default()),
        }
    }

    pub fn with_ahead(mut self, ahead: NarrowAhead) -> Self {
        self.ahead = ahead;
        self
    }

    pub fn estimator(&self) -> &BiasCorrectedEstimator {
        &self.est
    }

    pub fn stats(&self) -> ShieldStats {
        self.lock().stats
    }

    /// Start of `job`'s protected slot, if it holds one.
    pub fn protected_start(&self, job: JobId) -> Option<u64> {
        self.lock().slots.get(&job).map(|s| s.start)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shield> {
        self.shield.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Conservative view of the machine: last pass's reservations dropped,
    /// running jobs stretched to their walltime.
    fn conservative_view(
        sh: &mut Shield,
        queue: &[&Job],
        graph: &ResourceGraph,
        now: u64,
    ) -> ResourceGraph {
        let mut view = graph.clone();
        let queued: HashSet<JobId> = queue.iter().map(|j| j.id).collect();
        for j in queue.iter().filter(|j| j.state == JobState::Reserved) {
            release(&mut view, j.id);
        }
        let leaves: Vec<VertexId> = view.leaves().collect();
        // Finished jobs keep their (past) spans in the planner: drop them, so
        // that only jobs still running are stretched to their walltime.
        for &v in &leaves {
            view.vertex_mut(v).planner.release_before(now);
        }
        sh.walls.retain(|id, wall| {
            if queued.contains(id) {
                return true;
            }
            let mut live = false;
            for &v in &leaves {
                let p = &mut view.vertex_mut(v).planner;
                if let Some((start, _)) = p.job_span(id.0) {
                    p.extend_job(id.0, start + *wall);
                    live = true;
                }
            }
            live
        });
        sh.slots.retain(|id, _| queued.contains(id));
        view
    }

    /// Re-plan the protected slots: walk the queue in priority order as
    /// conservative backfill would (up to `depth` reservations), wide jobs at
    /// walltime and narrow ones as `ahead` says; each wide job takes the
    /// earlier of its old slot and its new fit.
    fn replan_slots(&self, sh: &mut Shield, queue: &[&Job], view: &ResourceGraph, now: u64) {
        let shape = MachineShape::of(view);
        let mut plan = view.clone();
        let old: Vec<JobId> = sh.slots.keys().copied().collect();
        for id in old {
            let s = sh.slots[&id].clone();
            if !book(&mut plan, &s.vertices, &s.req, s.start, s.dur, id) {
                sh.slots.remove(&id);
                sh.stats.dropped += 1;
            }
        }
        let mut order: Vec<&Job> = queue.to_vec();
        order.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));
        let mut reserved = 0usize;
        for j in order {
            if reserved >= self.depth {
                break;
            }
            let wall = sh.wall(j);
            let wide = self.est.is_wide(j, shape);
            let dur = match (wide, self.ahead) {
                (true, _) => wall,
                (false, NarrowAhead::Skip) => continue,
                (false, NarrowAhead::Walltime) => wall,
                (false, NarrowAhead::Cautious) => self.est.cautious_secs(j),
                (false, NarrowAhead::Planned) => self.est.planned_secs(j),
            };
            let old = if wide {
                sh.slots.get(&j.id).cloned()
            } else {
                None
            };
            if old.is_some() {
                release(&mut plan, j.id);
            }
            let fit = plan.match_gang(&j.request, j.node_count(), now, dur, j.id.0);
            let (vs, start) = match (fit, &old) {
                (Some((vs, t)), Some(o)) if t < o.start => (vs, t),
                (_, Some(o)) => (o.vertices.clone(), o.start),
                (Some(f), None) => f,
                (None, None) => continue,
            };
            if !book(&mut plan, &vs, &j.request, start, dur, j.id) {
                // Only an old slot that capacity shrank under can fail here.
                sh.slots.remove(&j.id);
                sh.stats.dropped += 1;
                continue;
            }
            reserved += usize::from(start > now);
            if wide {
                let slot = Slot {
                    vertices: vs,
                    start,
                    dur: wall,
                    req: j.request,
                };
                sh.slots.insert(j.id, slot);
            }
        }
    }
}

impl Policy for WideSafeBackfill {
    fn name(&self) -> String {
        format!("{}-ws.{}", self.inner.name(), self.ahead.label())
    }

    fn planned_secs(&self, job: &Job) -> u64 {
        self.inner.planned_secs(job)
    }

    fn observe(&self, job: &Job, actual: u64) {
        self.inner.observe(job, actual)
    }

    fn extend_secs(&self, job: &Job, elapsed: u64) -> u64 {
        self.inner.extend_secs(job, elapsed)
    }

    fn begin_pass(&self, queue: &[&Job], graph: &ResourceGraph, now: u64) {
        self.inner.begin_pass(queue, graph, now);
        let mut sh = self.lock();
        for j in queue {
            sh.walls.insert(j.id, j.walltime.secs().max(1));
        }
        let mut view = Self::conservative_view(&mut sh, queue, graph, now);
        self.replan_slots(&mut sh, queue, &view, now);
        let ids: Vec<JobId> = sh.slots.keys().copied().collect();
        for id in ids {
            let s = sh.slots[&id].clone();
            if !book(&mut view, &s.vertices, &s.req, s.start, s.dur, id) {
                sh.slots.remove(&id);
                sh.stats.dropped += 1;
            }
        }
        sh.view = Some(view);
        let shape = MachineShape::of(graph);
        sh.due.clear();
        for j in queue.iter().filter(|j| self.est.is_wide(j, shape)) {
            if sh.earliest(j, now).is_some_and(|(_, t)| t <= now) {
                sh.due.insert(j.id);
            }
        }
    }

    fn place(&self, graph: &ResourceGraph, job: &Job, now: u64, allow_reserve: bool) -> Decision {
        let decision = self.inner.place(graph, job, now, allow_reserve);
        let mut sh = self.lock();
        if sh.view.is_none() {
            return decision; // no pass context (snapshot API): unshielded
        }
        let shape = MachineShape::of(graph);
        let wide = self.est.is_wide(job, shape);
        sh.due.remove(&job.id);
        let others_due = !sh.due.is_empty();
        if let Decision::Start { vertices, .. } = &decision {
            if sh.admit(job, vertices, now, wide) {
                return decision;
            }
            sh.stats.refused += 1;
        }
        // A wide job whose walltime span fits now in the shield starts now,
        // whatever the optimistic plan says (its slot is due).
        if wide {
            if let Some((vs, t)) = sh.earliest(job, now) {
                let spans = self.est.spans_on(job, shape);
                let wall = sh.wall(job);
                let dur = [spans.record.max(spans.plan), spans.plan, wall]
                    .into_iter()
                    .find(|&d| fits_at(graph, &vs, &job.request, now, d));
                if let (true, Some(d)) = (t <= now, dur) {
                    if sh.admit(job, &vs, now, wide) {
                        if !matches!(decision, Decision::Start { .. }) {
                            sh.stats.forced += 1;
                        }
                        return Decision::Start {
                            vertices: vs,
                            start: now,
                            dur: Some(d),
                        };
                    }
                }
            }
        }
        let refused = matches!(decision, Decision::Start { .. });
        if others_due || !allow_reserve {
            return Decision::Defer;
        }
        if !refused {
            return decision;
        }
        // Refused start: reserve where the walltime span would be admitted.
        let Some((_, from)) = sh.earliest(job, now) else {
            return Decision::Defer;
        };
        let plan = self.est.spans_on(job, shape).plan;
        let from = from.max(now + 1);
        match self
            .inner
            .place_span(graph, job, from, true, plan, Some(plan))
        {
            Decision::Start {
                vertices,
                start,
                dur,
            }
            | Decision::Reserve {
                vertices,
                start,
                dur,
            } if start <= now + self.inner.window_secs => Decision::Reserve {
                vertices,
                start,
                dur,
            },
            _ => Decision::Defer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnomon_core::{Resources, Walltime};
    use std::time::Duration;

    fn job(user: &str, cores: u32, wall: u64) -> Job {
        Job::new(
            JobId(1),
            "swf",
            user,
            Resources {
                cores,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(Duration::from_secs(wall)),
        )
    }

    /// Deterministic xorshift in [0, 1).
    fn rng(seed: u64) -> impl FnMut() -> f64 {
        let mut x = seed;
        move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    const SHAPE: MachineShape = MachineShape {
        total_cores: 128,
        largest_node_cores: 128,
    };

    fn all_spans(s: Spans) -> [u64; 3] {
        [s.plan, s.guard, s.record]
    }

    #[test]
    fn plans_walltime_without_history() {
        let est = BiasCorrectedEstimator::new(BiasConfig::default());
        let j = job("alice", 1, 7200);
        assert_eq!(est.planned_secs(&j), 7200);
        assert_eq!(est.spans_on(&j, SHAPE), Spans::uniform(7200));
    }

    #[test]
    fn spans_stay_between_floor_and_walltime() {
        for mode in [GuardMode::Block, GuardMode::Record] {
            let est = BiasCorrectedEstimator::new(BiasConfig {
                guard_mode: mode,
                ..BiasConfig::default()
            });
            let mut r = rng(7);
            for i in 0..2000u64 {
                let wall = 30 + (r() * 50_000.0) as u64;
                let j = job(&format!("u{}", i % 13), 1 + (r() * 64.0) as u32, wall);
                // Mostly instant failures drive predictions toward the floor.
                let actual = if r() < 0.9 { 0 } else { wall };
                let s = est.spans_on(&j, SHAPE);
                for span in all_spans(s) {
                    assert!(span <= wall, "{span} > walltime {wall}");
                    assert!(span >= 60.min(wall), "{span} below floor (wall {wall})");
                }
                assert!(s.guard >= s.plan && s.record >= s.plan, "{s:?}");
                assert!(est.cautious_secs(&j) >= est.planned_secs(&j));
                est.observe(&j, actual);
            }
        }
    }

    #[test]
    fn bias_correction_converges_to_target_coverage() {
        // Known bias: runtimes are walltime * U(0.2, 0.6), so the plain
        // (geometric) mean under-predicts the 0.8-quantile, 0.52 * walltime.
        // With tau = 0.8 about 80% of jobs must fit their span.
        let est = BiasCorrectedEstimator::new(BiasConfig {
            tau: 0.8,
            guard_tau: None,
            ..BiasConfig::default()
        });
        let mut r = rng(11);
        let wall = 10_000u64;
        let j = job("bob", 4, wall);
        let (mut fit, mut n, mut span) = (0u32, 0u32, 0u64);
        for i in 0..6000 {
            let actual = (wall as f64 * (0.2 + 0.4 * r())) as u64;
            if i >= 3000 {
                let p = est.planned_secs(&j);
                n += 1;
                span += p;
                fit += u32::from(actual <= p);
            }
            est.observe(&j, actual);
        }
        let cover = fit as f64 / n as f64;
        assert!((0.75..=0.85).contains(&cover), "coverage {cover}");
        let mean_span = span / n as u64;
        assert!(
            (4800..=6000).contains(&mean_span),
            "mean span {mean_span} vs q0.8 = 5200"
        );
        // The correction is a clearly positive multiplicative offset.
        assert!(est.bias_offset("bob") > 0.1, "{}", est.bias_offset("bob"));
    }

    #[test]
    fn bias_correction_is_per_user_with_global_fallback() {
        let est = BiasCorrectedEstimator::new(BiasConfig::default());
        let wall = 10_000;
        for _ in 0..400 {
            est.observe(&job("short", 1, wall), 1000);
            est.observe(&job("long", 1, wall), 9000);
        }
        let s = est.planned_secs(&job("short", 1, wall));
        let l = est.planned_secs(&job("long", 1, wall));
        let new = est.planned_secs(&job("newcomer", 1, wall));
        assert!(s < 2000, "short user span {s}");
        assert!(l > 8000, "long user span {l}");
        assert!(s < new && new <= wall, "newcomer falls back: {new}");
    }

    #[test]
    fn wide_jobs_get_walltime_planning_when_enabled() {
        let cfg = BiasConfig {
            wide_at_walltime: true,
            ..BiasConfig::default()
        };
        let est = BiasCorrectedEstimator::new(cfg);
        for _ in 0..100 {
            est.observe(&job("carol", 64, 10_000), 500);
        }
        let wide = job("carol", 32, 10_000); // 25% of 128 cores
        let narrow = job("carol", 31, 10_000);
        assert_eq!(est.spans_on(&wide, SHAPE), Spans::uniform(10_000));
        assert!(est.spans_on(&narrow, SHAPE).plan < 2000);
        // Off by default: the wide job is planned at its prediction.
        let off = BiasCorrectedEstimator::new(BiasConfig::default());
        for _ in 0..100 {
            off.observe(&job("carol", 64, 10_000), 500);
        }
        assert!(off.spans_on(&wide, SHAPE).plan < 2000);
        // A gang is wide however small its per-node slice is.
        let mut gang = job("carol", 4, 10_000);
        gang.nodes = 4;
        assert_eq!(est.spans_on(&gang, SHAPE), Spans::uniform(10_000));
    }

    #[test]
    fn guard_modes_shape_the_spans() {
        let mk = |guard_mode| {
            let est = BiasCorrectedEstimator::new(BiasConfig {
                guard_mode,
                ..BiasConfig::default()
            });
            let mut r = rng(3);
            for _ in 0..500 {
                est.observe(&job("dan", 1, 10_000), (1000.0 + 4000.0 * r()) as u64);
            }
            est
        };
        let j = job("dan", 1, 10_000);
        let rec = mk(GuardMode::Record).spans_on(&j, SHAPE);
        assert_eq!(rec.guard, rec.plan);
        assert!(rec.record > rec.plan, "{rec:?}");
        let blk = mk(GuardMode::Block).spans_on(&j, SHAPE);
        assert_eq!(blk.record, blk.plan);
        assert!(blk.guard > blk.plan, "{blk:?}");
    }

    #[test]
    fn slipped_wide_reservation_turns_on_protection_until_it_starts() {
        let est = BiasCorrectedEstimator::new(BiasConfig::default());
        for _ in 0..50 {
            est.observe(&job("erin", 1, 10_000), 1000);
        }
        let narrow = job("erin", 1, 10_000);
        let mut wide = job("frank", 64, 10_000);
        wide.id = JobId(2);
        wide.state = JobState::Reserved;
        let pass = |w: &Job, start: u64| {
            let mut w = w.clone();
            w.planned_start = Some(start);
            est.begin_pass(&[&w], SHAPE, 0);
        };
        pass(&wide, 5000);
        pass(&wide, 4000); // moved earlier: no slip
        assert!(!est.protecting());
        let normal = est.spans_on(&narrow, SHAPE);
        pass(&wide, 6000); // moved later: slipped
        assert!(est.protecting());
        let guarded = est.spans_on(&narrow, SHAPE);
        assert_eq!(guarded.record, 10_000);
        assert_eq!(guarded.guard, est.cautious_secs(&narrow));
        assert_eq!(guarded.plan, normal.plan);
        pass(&wide, 6000); // still slipped while queued
        assert!(est.protecting());
        est.begin_pass(&[], SHAPE, 0); // the wide job started
        assert!(!est.protecting());
        assert_eq!(est.spans_on(&narrow, SHAPE), normal);
    }

    // ----- wide-job shield --------------------------------------------------

    use crate::{backfill_pass, PassOptions, PlanApplier};
    use gnomon_core::{Event, ManualClock};
    use gnomon_fsm::Fsm;

    /// Shield tests count jobs of at least half the machine as wide.
    fn shield_cfg() -> BiasConfig {
        BiasConfig {
            wide_fraction: 0.5,
            ..BiasConfig::default()
        }
    }

    fn machine(cores: u32) -> (Arc<ManualClock>, Fsm) {
        let clock = Arc::new(ManualClock::new(0));
        let fsm = Fsm::with_clock(clock.clone());
        fsm.apply(Event::NodeRegistered {
            name: "n0".into(),
            capacity: Resources {
                cores,
                mem_mb: 0,
                gpus: 0,
            },
        });
        (clock, fsm)
    }

    fn submit(fsm: &Fsm, id: u64, user: &str, cores: u32, wall: u64, at: u64) {
        let mut j = job(user, cores, wall);
        j.id = JobId(id);
        j.priority = 0;
        j.submit_time = at;
        fsm.apply(Event::JobSubmitted(j));
    }

    fn pass(fsm: &Fsm, policy: &dyn Policy) {
        backfill_pass(fsm, policy, &PlanApplier::new(), PassOptions::default());
    }

    fn state(fsm: &Fsm, id: u64) -> (JobState, Option<u64>) {
        fsm.read(|st| {
            let j = &st.jobs[&JobId(id)];
            (j.state, j.planned_start)
        })
    }

    /// User `slow` runs half its walltime, user `quick` a twentieth.
    fn trained() -> Arc<BiasCorrectedEstimator> {
        let est = Arc::new(BiasCorrectedEstimator::new(shield_cfg()));
        for _ in 0..300 {
            est.observe(&job("slow", 1, 10_000), 5_000);
            est.observe(&job("quick", 1, 20_000), 1_000);
        }
        est
    }

    /// 100 cores. Job 1 (`slow`, 60 cores, walltime 10 000 s) runs from 0.
    /// At 0 the wide job 2 (100 cores) and the narrow job 3 (`quick`, 40
    /// cores, walltime `wall3`, predicted ~1 000 s) are queued. Walltime
    /// planning reserves job 2 at 10 000.
    fn wide_behind_running_job(policy: &dyn Policy, wall3: u64) -> (Arc<ManualClock>, Fsm) {
        let (clock, fsm) = machine(100);
        submit(&fsm, 1, "slow", 60, 10_000, 0);
        pass(&fsm, policy);
        assert_eq!(state(&fsm, 1).0, JobState::Running);
        submit(&fsm, 2, "slow", 100, 10_000, 0);
        submit(&fsm, 3, "quick", 40, wall3, 0);
        pass(&fsm, policy);
        (clock, fsm)
    }

    /// Job `id` finishes at `end`; a pass follows.
    fn finish(clock: &ManualClock, fsm: &Fsm, policy: &dyn Policy, id: u64, end: u64) {
        clock.set(end);
        fsm.apply(Event::JobFinished {
            job: JobId(id),
            end,
            code: 0,
        });
        pass(fsm, policy);
    }

    #[test]
    fn unshielded_predictor_lets_an_overrunning_backfill_delay_the_wide_job() {
        let policy = BackfillPolicy::with_estimator(48 * 3600, trained());
        let (clock, fsm) = wide_behind_running_job(&policy, 20_000);
        assert_eq!(
            state(&fsm, 3).0,
            JobState::Running,
            "backfilled on prediction"
        );
        finish(&clock, &fsm, &policy, 1, 10_000);
        assert_ne!(state(&fsm, 2).0, JobState::Running);
        finish(&clock, &fsm, &policy, 3, 20_000);
        assert_eq!(state(&fsm, 2), (JobState::Running, Some(20_000)));
    }

    #[test]
    fn shield_refuses_a_backfill_whose_walltime_covers_a_wide_slot() {
        let policy = WideSafeBackfill::new(48 * 3600, trained());
        let (clock, fsm) = wide_behind_running_job(&policy, 20_000);
        assert_eq!(policy.protected_start(JobId(2)), Some(10_000));
        assert_ne!(state(&fsm, 3).0, JobState::Running, "refused by the shield");
        assert!(policy.stats().refused >= 1);
        // Job 1 overruns its prediction up to its walltime: the wide job
        // still starts exactly at its slot.
        finish(&clock, &fsm, &policy, 1, 10_000);
        assert_eq!(state(&fsm, 2), (JobState::Running, Some(10_000)));
        // Job 1 (60 of 100 cores) is wide as well and started on its slot.
        let s = policy.stats();
        assert_eq!((s.protected_starts, s.late_starts), (2, 0));
    }

    #[test]
    fn shield_admits_a_backfill_whose_walltime_ends_before_the_wide_slot() {
        let policy = WideSafeBackfill::new(48 * 3600, trained());
        let (_, fsm) = wide_behind_running_job(&policy, 9_000);
        assert_eq!(state(&fsm, 3).0, JobState::Running);
        assert_eq!(policy.protected_start(JobId(2)), Some(10_000));
        assert_eq!(policy.stats().refused, 0);
    }

    #[test]
    fn shield_moves_a_wide_slot_earlier_when_a_running_job_ends_early() {
        let policy = WideSafeBackfill::new(48 * 3600, trained());
        let (clock, fsm) = wide_behind_running_job(&policy, 20_000);
        assert_eq!(policy.protected_start(JobId(2)), Some(10_000));
        finish(&clock, &fsm, &policy, 1, 4_000);
        assert_eq!(state(&fsm, 2), (JobState::Running, Some(4_000)));
    }

    #[test]
    fn shield_keeps_the_predictor_name_and_spans() {
        let est = trained();
        let policy = WideSafeBackfill::new(48 * 3600, est.clone());
        let name = RuntimeEstimator::name(&*est);
        assert_eq!(policy.name(), format!("backfill/{name}-ws.plan"));
        let wall = WideSafeBackfill::new(48 * 3600, est.clone()).with_ahead(NarrowAhead::Walltime);
        assert_eq!(wall.name(), format!("backfill/{name}-ws.wall"));
        let j = job("quick", 1, 20_000);
        assert_eq!(policy.planned_secs(&j), est.planned_secs(&j));
        assert!(policy.planned_secs(&j) < 5_000);
    }

    type WideRecord = (Vec<u64>, u64);

    /// Event-driven run on a random workload: passes only at arrivals and
    /// finishes, jobs end within their walltime. Returns, per wide job that
    /// held a slot, its slot after each pass and its start.
    fn random_run(seed: u64, n: u64, ahead: NarrowAhead) -> (WideSafeBackfill, Vec<WideRecord>) {
        let mut r = rng(seed);
        let est = Arc::new(BiasCorrectedEstimator::new(shield_cfg()));
        let policy = WideSafeBackfill::new(48 * 3600, est).with_ahead(ahead);
        let (clock, fsm) = machine(64);
        let mut arrivals = Vec::new();
        let mut t = 0u64;
        for id in 1..=n {
            t += (r() * 900.0) as u64;
            let wide = r() < 0.2;
            let cores = if wide {
                32 + (r() * 32.0) as u32
            } else {
                1 + (r() * 24.0) as u32
            };
            let wall = 600 + (r() * 20_000.0) as u64;
            // Mostly short runs (optimistic predictions), some to walltime.
            let actual = if r() < 0.3 {
                wall
            } else {
                (wall as f64 * r() * 0.3) as u64 + 1
            };
            arrivals.push((t, id, cores, wall, actual));
        }
        let actual: HashMap<u64, u64> = arrivals.iter().map(|a| (a.1, a.4)).collect();
        let wide_ids: Vec<u64> = arrivals.iter().filter(|a| a.2 >= 32).map(|a| a.1).collect();
        let mut ai = 0;
        let mut running: Vec<(u64, u64, u64)> = Vec::new(); // (end, id, start)
        let mut slots: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut started: HashMap<u64, u64> = HashMap::new();
        loop {
            let next_finish = running.iter().map(|x| x.0).min();
            let next_arrival = arrivals.get(ai).map(|a| a.0);
            let Some(now) = [next_finish, next_arrival].into_iter().flatten().min() else {
                break;
            };
            clock.set(now);
            running.sort_unstable();
            while running.first().is_some_and(|x| x.0 <= now) {
                let (end, id, start) = running.remove(0);
                fsm.apply(Event::JobFinished {
                    job: JobId(id),
                    end,
                    code: 0,
                });
                let j = fsm.read(|st| st.jobs[&JobId(id)].clone());
                policy.observe(&j, end - start);
            }
            while arrivals.get(ai).is_some_and(|a| a.0 <= now) {
                let (at, id, cores, wall, _) = arrivals[ai];
                submit(&fsm, id, &format!("u{}", id % 5), cores, wall, at);
                ai += 1;
            }
            pass(&fsm, &policy);
            for &id in &wide_ids {
                if let Some(s) = policy.protected_start(JobId(id)) {
                    slots.entry(id).or_default().push(s);
                }
            }
            let now_running: Vec<u64> = fsm.read(|st| {
                st.jobs
                    .values()
                    .filter(|j| j.state == JobState::Running)
                    .map(|j| j.id.0)
                    .collect()
            });
            for id in now_running {
                if let std::collections::hash_map::Entry::Vacant(e) = started.entry(id) {
                    e.insert(now);
                    running.push((now + actual[&id], id, now));
                }
            }
        }
        assert_eq!(started.len() as u64, n, "every job ran");
        let wide = wide_ids
            .iter()
            .filter_map(|id| Some((slots.remove(id)?, started[id])))
            .collect();
        (policy, wide)
    }

    #[test]
    fn wide_slots_never_move_later_and_wide_jobs_start_by_them_on_random_workloads() {
        let modes = [
            NarrowAhead::Walltime,
            NarrowAhead::Cautious,
            NarrowAhead::Planned,
            NarrowAhead::Skip,
        ];
        for (seed, ahead) in (1..=8).zip(modes.into_iter().cycle()) {
            let (policy, wide) = random_run(seed, 400, ahead);
            assert!(
                wide.len() > 20,
                "seed {seed}: {} slotted wide jobs",
                wide.len()
            );
            for (slots, start) in &wide {
                assert!(
                    slots.windows(2).all(|w| w[1] <= w[0]),
                    "seed {seed}: slot moved later: {slots:?}"
                );
                let last = *slots.last().unwrap();
                assert!(
                    *start <= last,
                    "seed {seed}: started {start} after {slots:?}"
                );
            }
            let s = policy.stats();
            assert_eq!(s.late_starts, 0, "seed {seed}: {s:?}");
            assert_eq!(s.dropped, 0, "seed {seed}: {s:?}");
            assert!(s.refused > 0, "seed {seed}: the shield was never exercised");
        }
    }

    #[test]
    fn narrow_ahead_mode_decides_where_a_new_slot_goes() {
        // 100 cores; job 1 (60 cores, walltime 10 000) runs. Queued in
        // priority order: narrow job 2 (`quick`, 45 cores, walltime 20 000,
        // predicted ~1 000), which cannot start before job 1 ends, then the
        // wide job 3 (100 cores).
        let slot = |ahead| {
            let policy = WideSafeBackfill::new(48 * 3600, trained()).with_ahead(ahead);
            let (_, fsm) = machine(100);
            submit(&fsm, 1, "slow", 60, 10_000, 0);
            pass(&fsm, &policy);
            submit(&fsm, 2, "quick", 45, 20_000, 0);
            submit(&fsm, 3, "slow", 100, 10_000, 0);
            pass(&fsm, &policy);
            assert_ne!(state(&fsm, 2).0, JobState::Running);
            policy
                .protected_start(JobId(3))
                .expect("wide job holds a slot")
        };
        // Wide first: right after job 1's walltime.
        assert_eq!(slot(NarrowAhead::Skip), 10_000);
        // Walltime: after job 2's walltime span, which starts at 10 000.
        assert_eq!(slot(NarrowAhead::Walltime), 30_000);
        // Planned: after job 2's predicted span only.
        let planned = slot(NarrowAhead::Planned);
        assert!((10_000..30_000).contains(&planned), "{planned}");
        assert!(slot(NarrowAhead::Cautious) >= planned);
    }
}
