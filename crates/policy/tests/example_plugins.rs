//! The Rust example plugins (`examples/policy-plugins`) loaded as real
//! compiled components. CI builds one component per world and points
//! `GNOMON_EXAMPLE_<WORLD>_PLUGIN` at each `.wasm`; a test whose variable
//! is unset does nothing, since building needs the wasm32 target and
//! `wasm-tools`.

use gnomon_core::{Job, JobId, Resources, Walltime};
use gnomon_policy::*;
use std::time::Duration;

fn job(partition: &str, cores: u32, gpus: u32, nodes: u32, wall: u64) -> Job {
    let mut j = Job::new(
        JobId(1),
        "j",
        "alice",
        Resources {
            cores,
            mem_mb: 0,
            gpus,
        },
        Walltime(Duration::from_secs(wall)),
    );
    j.partition = partition.into();
    j.nodes = nodes;
    j.submit_time = 1_000;
    j
}

/// The component at `$GNOMON_EXAMPLE_<world>_PLUGIN`, if set.
fn component(world: &str) -> Option<std::ffi::OsString> {
    let var = format!("GNOMON_EXAMPLE_{world}_PLUGIN");
    let path = std::env::var_os(&var);
    if path.is_none() {
        eprintln!("{var} not set; skipping");
    }
    path
}

#[test]
fn priority_example_is_multifactor() {
    let Some(path) = component("PRIORITY") else {
        return;
    };
    // Loading proves the component imports nothing but the WIT types.
    let p = PriorityPlugin::from_file(&path, PluginLimits::default()).unwrap();
    let hour_later = 1_000 + 3_600;
    // age 60 min * 10 + size 10 000 / (8 cores * 2 nodes)
    assert_eq!(
        p.priority(&job("normal", 8, 0, 2, 60), hour_later).unwrap(),
        1_225
    );
    assert_eq!(
        p.priority(&job("debug", 8, 0, 2, 60), hour_later).unwrap(),
        51_225
    );
    // Age is capped at 7 days; a single core gets the whole size factor.
    let year = 1_000 + 365 * 86_400;
    assert_eq!(
        p.priority(&job("normal", 1, 0, 1, 60), year).unwrap(),
        110_800
    );
    for _ in 0..50 {
        assert_eq!(
            p.priority(&job("normal", 8, 0, 2, 60), hour_later).unwrap(),
            1_225
        );
    }
}

#[test]
fn admission_example_applies_site_rules() {
    let Some(path) = component("ADMISSION") else {
        return;
    };
    let p = AdmissionPlugin::from_file(&path, PluginLimits::default()).unwrap();
    assert_eq!(
        p.admit(&job("normal", 4, 0, 1, 8 * 86_400), 0).unwrap(),
        Verdict::Reject("walltime over 7 days".into())
    );
    assert_eq!(
        p.admit(&job("normal", 4, 0, 2, 7_200), 0).unwrap(),
        Verdict::Accept
    );
    let rewrite = |partition: &str, gpus, nodes, wall| {
        Verdict::Rewrite(Rewrite {
            partition: partition.into(),
            request: Resources {
                cores: 4,
                mem_mb: 0,
                gpus,
            },
            nodes,
            walltime_secs: wall,
        })
    };
    assert_eq!(
        p.admit(&job("debug", 4, 0, 2, 7_200), 0).unwrap(),
        rewrite("debug", 0, 1, 3_600)
    );
    assert_eq!(
        p.admit(&job("normal", 4, 1, 2, 7_200), 0).unwrap(),
        rewrite("gpu", 1, 2, 7_200)
    );
}

#[test]
fn estimator_example_plans_long_jobs_at_60_percent() {
    let Some(path) = component("ESTIMATOR") else {
        return;
    };
    let p = EstimatorPlugin::from_file(&path, PluginLimits::default()).unwrap();
    assert_eq!(
        p.estimate(&job("normal", 1, 0, 1, 7_200)).unwrap(),
        Some(4_320)
    );
    assert_eq!(p.estimate(&job("normal", 1, 0, 1, 3_600)).unwrap(), None);
}

#[test]
fn node_score_example_is_best_fit_off_gpu_nodes() {
    let Some(path) = component("NODE_SCORE") else {
        return;
    };
    let p = NodeScorePlugin::from_file(&path, PluginLimits::default()).unwrap();
    let node = |name: &str, free, gpus| Candidate {
        name: name.into(),
        capacity: Resources {
            cores: 32,
            mem_mb: 0,
            gpus,
        },
        free: Resources {
            cores: free,
            mem_mb: 0,
            gpus,
        },
    };
    let cands = [node("tight", 4, 0), node("roomy", 16, 0), node("gpu", 8, 4)];
    let scores = p.score(&job("normal", 4, 0, 1, 60), &cands, 0).unwrap();
    assert_eq!(scores, [0, -12, -1_000_004]);
}
