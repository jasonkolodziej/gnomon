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

use crate::{MachineShape, RuntimeEstimator, Spans};
use gnomon_core::{Job, JobId, JobState};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};

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

fn core_bucket(job: &Job) -> usize {
    log2_bucket(job.request.cores.max(1) as f64, CORE_BUCKETS)
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

    fn is_wide(&self, job: &Job, shape: MachineShape) -> bool {
        shape.largest_node_cores > 0
            && job.request.cores as f64 >= self.cfg.wide_fraction * shape.largest_node_cores as f64
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
}
