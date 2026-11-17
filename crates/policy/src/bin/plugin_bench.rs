//! Cost of WASM plugins on the scheduling path (ADR 0004).
//!
//! Builds a controller FSM with `nodes` 32-core nodes and `4 x nodes`
//! pending jobs of mixed shapes (a quarter of them 2-node gangs), then times
//! one full `backfill_pass` (the production scheduling pass) under:
//!
//! * `walltime`: `BackfillPolicy` alone, the baseline;
//! * `estimator`: with an estimator plugin (one call per job, then cached);
//! * `node-score-all`: with a node-score plugin offered every feasible node
//!   (one call per placement that has a choice);
//! * `node-score`: the same, capped at the default
//!   `DEFAULT_MAX_CANDIDATES` other candidates per call;
//! * `both`: estimator and capped node-score.
//!
//! Plugins are the hand-written fixtures in `gnomon_policy::fixtures`, so
//! the numbers measure the host's per-call overhead (store, instance,
//! lowering the arguments), which dominates for small policies.
//!
//! usage: `plugin_bench [nodes...]` (default 64 256 1024). Output CSV:
//! `variant,nodes,jobs,started,reserved,plugin_calls,millis,us_per_job`.

use gnomon_core::{Event, Job, JobId, Resources, Walltime};
use gnomon_fsm::Fsm;
use gnomon_policy::{
    fixtures, EstimatorPlugin, NodeScorePlugin, PluginLimits, ScoredPolicy, WasmEstimator,
    DEFAULT_MAX_CANDIDATES,
};
use gnomon_sched::{backfill_pass, BackfillPolicy, PlanApplier, Policy};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

const WINDOW: u64 = 48 * 3600;

fn cluster(nodes: u32) -> (Fsm, u64) {
    let fsm = Fsm::new();
    for n in 0..nodes {
        fsm.apply(Event::NodeRegistered {
            name: format!("node{n:04}"),
            capacity: Resources {
                cores: 32,
                mem_mb: 131_072,
                gpus: 0,
            },
        });
    }
    let jobs = 4 * u64::from(nodes);
    for i in 0..jobs {
        let cores = [16, 8, 4, 4][(i % 4) as usize];
        let mut job = Job::new(
            JobId(i + 1),
            "b",
            "bench",
            Resources {
                cores,
                mem_mb: 1024,
                gpus: 0,
            },
            Walltime(Duration::from_secs(600 + 60 * (i % 50))),
        );
        job.nodes = if i % 4 == 0 { 2 } else { 1 };
        job.priority = jobs - i;
        fsm.apply(Event::JobSubmitted(job));
    }
    (fsm, jobs)
}

fn main() {
    let sizes: Vec<u32> = std::env::args()
        .skip(1)
        .map(|a| a.parse().expect("node counts"))
        .collect();
    let sizes = if sizes.is_empty() {
        vec![64, 256, 1024]
    } else {
        sizes
    };
    let limits = PluginLimits::default();
    let est_wat = fixtures::half_walltime();
    let score_wat = fixtures::most_free_cores();
    println!("variant,nodes,jobs,started,reserved,plugin_calls,millis,us_per_job");
    for &nodes in &sizes {
        for variant in [
            "walltime",
            "estimator",
            "node-score-all",
            "node-score",
            "both",
        ] {
            let est = matches!(variant, "estimator" | "both").then(|| {
                let p = EstimatorPlugin::from_bytes(est_wat.as_bytes(), limits).unwrap();
                Arc::new(WasmEstimator::new(p))
            });
            let backfill = match &est {
                Some(e) => BackfillPolicy::with_estimator(WINDOW, e.clone()),
                None => BackfillPolicy::new(WINDOW),
            };
            let scored = variant.starts_with("node-score") || variant == "both";
            let scored = scored.then(|| {
                let p = NodeScorePlugin::from_bytes(score_wat.as_bytes(), limits).unwrap();
                let s = ScoredPolicy::new(Arc::new(backfill), p);
                let cap = (variant != "node-score-all").then_some(DEFAULT_MAX_CANDIDATES);
                Arc::new(s.with_max_candidates(cap))
            });
            let policy: Arc<dyn Policy> = match &scored {
                Some(s) => s.clone(),
                None => Arc::new(match &est {
                    Some(e) => BackfillPolicy::with_estimator(WINDOW, e.clone()),
                    None => BackfillPolicy::new(WINDOW),
                }),
            };
            let (fsm, jobs) = cluster(nodes);
            let applier = PlanApplier::new();
            let t = Instant::now();
            let stats = backfill_pass(&fsm, policy.as_ref(), &applier, Default::default());
            let elapsed = t.elapsed();
            let calls = est.as_ref().map_or(0, |e| e.calls.load(Ordering::Relaxed))
                + scored
                    .as_ref()
                    .map_or(0, |s| s.calls.load(Ordering::Relaxed));
            println!(
                "{variant},{nodes},{jobs},{},{},{calls},{},{:.1}",
                stats.started,
                stats.reserved,
                elapsed.as_millis(),
                elapsed.as_secs_f64() * 1e6 / jobs as f64
            );
        }
    }
}
