//! `estimator` world: a job's expected runtime, used by the backfill planner
//! for its span instead of the walltime. The leader runs it while building a
//! plan, and the plan event records the resulting span, so replicas and
//! replay never run it (ADR 0002, Rule 1).

use super::bindings::estimator::EstimatorPre;
use super::{plugin_loaders, wit_job, Host, PluginError, Sandbox};
use gnomon_core::{Job, JobId};
use gnomon_sched::{MachineShape, RuntimeEstimator};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// A loaded estimator plugin.
pub struct EstimatorPlugin(Sandbox<EstimatorPre<Host>>);

plugin_loaders!(EstimatorPlugin, EstimatorPre<super::Host>);

impl EstimatorPlugin {
    /// The plugin's raw estimate for `job` (unclamped; `None` = walltime).
    pub fn estimate(&self, job: &Job) -> Result<Option<u64>, PluginError> {
        self.0.call(|store, pre| {
            pre.instantiate(&mut *store)?
                .call_estimate(&mut *store, &wit_job(job))
        })
    }
}

/// [`RuntimeEstimator`] backed by an estimator plugin. The estimate is a
/// pure function of the job, so it is computed once per job and cached for
/// as long as the job is queued. A failed call, or `none`, plans with the
/// walltime (what [`gnomon_sched::BackfillPolicy`] does without an
/// estimator); estimates are clamped to `[1, walltime]`.
pub struct WasmEstimator {
    plugin: EstimatorPlugin,
    cache: Mutex<HashMap<JobId, u64>>,
    pub calls: AtomicU64,
    pub failures: AtomicU64,
}

impl std::fmt::Debug for WasmEstimator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmEstimator")
            .field("calls", &self.calls)
            .field("failures", &self.failures)
            .finish_non_exhaustive()
    }
}

impl WasmEstimator {
    pub fn new(plugin: EstimatorPlugin) -> Self {
        WasmEstimator {
            plugin,
            cache: Mutex::new(HashMap::new()),
            calls: AtomicU64::new(0),
            failures: AtomicU64::new(0),
        }
    }

    fn compute(&self, job: &Job) -> u64 {
        let wall = job.walltime.secs();
        self.calls.fetch_add(1, Ordering::Relaxed);
        match self.plugin.estimate(job) {
            Ok(Some(secs)) => secs.clamp(1, wall.max(1)),
            Ok(None) => wall,
            Err(e) => {
                let n = self.failures.fetch_add(1, Ordering::Relaxed) + 1;
                eprintln!(
                    "gnomon: estimator plugin failed for job {} ({n} failures); \
                     planning with the walltime: {e}",
                    job.id.0
                );
                wall
            }
        }
    }
}

impl RuntimeEstimator for WasmEstimator {
    fn name(&self) -> String {
        "wasm".into()
    }

    fn planned_secs(&self, job: &Job) -> u64 {
        if let Some(&s) = self.cache.lock().unwrap().get(&job.id) {
            return s;
        }
        // Computed outside the lock: other jobs' estimates need not wait.
        let s = self.compute(job);
        self.cache.lock().unwrap().insert(job.id, s);
        s
    }

    /// Drop estimates of jobs that left the queue (started, finished or
    /// cancelled), keeping the cache as large as the queue.
    fn begin_pass(&self, queue: &[&Job], _shape: MachineShape, _now: u64) {
        let mut cache = self.cache.lock().unwrap();
        if cache.len() > queue.len() {
            let queued: std::collections::HashSet<JobId> = queue.iter().map(|j| j.id).collect();
            cache.retain(|id, _| queued.contains(id));
        }
    }
}
