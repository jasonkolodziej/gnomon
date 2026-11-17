//! The Rust example plugin (`examples/priority-plugin`) loaded as a real
//! compiled component. CI builds it and points `GNOMON_EXAMPLE_PLUGIN` at the
//! `.wasm`; without the variable the test does nothing, since building it
//! needs the wasm32 target and `wasm-tools`.

use gnomon_core::{Job, JobId, Resources, Walltime};
use gnomon_policy::{PluginLimits, PriorityPlugin};
use std::time::Duration;

fn job(partition: &str, cores: u32, nodes: u32) -> Job {
    let mut j = Job::new(
        JobId(1),
        "j",
        "alice",
        Resources {
            cores,
            mem_mb: 0,
            gpus: 0,
        },
        Walltime(Duration::from_secs(3600)),
    );
    j.partition = partition.into();
    j.nodes = nodes;
    j.submit_time = 1_000;
    j
}

#[test]
fn example_plugin_computes_multifactor_priority() {
    let Some(path) = std::env::var_os("GNOMON_EXAMPLE_PLUGIN") else {
        eprintln!("GNOMON_EXAMPLE_PLUGIN not set; skipping");
        return;
    };
    // Loading proves the component imports nothing but the WIT types.
    let p = PriorityPlugin::from_file(&path, PluginLimits::default()).unwrap();
    let hour_later = 1_000 + 3_600;
    // age 60 min * 10 + size 10 000 / (8 cores * 2 nodes)
    assert_eq!(p.priority(&job("normal", 8, 2), hour_later).unwrap(), 1_225);
    assert_eq!(p.priority(&job("debug", 8, 2), hour_later).unwrap(), 51_225);
    // Age is capped at 7 days; a single core gets the whole size factor.
    let year = 1_000 + 365 * 86_400;
    assert_eq!(
        p.priority(&job("normal", 1, 1), year).unwrap(),
        100_800 + 10_000
    );
    // Deterministic across calls.
    for _ in 0..50 {
        assert_eq!(p.priority(&job("normal", 8, 2), hour_later).unwrap(), 1_225);
    }
}
