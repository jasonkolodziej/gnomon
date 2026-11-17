//! Node-score plugin (ADR 0004): best fit, keeping GPU nodes for GPU jobs.
//!
//! * Prefer the node the job fills most tightly (fewest cores left free
//!   after placement), which leaves whole nodes for wide jobs and gangs.
//! * Jobs without GPUs avoid GPU nodes, so those stay free for GPU jobs.

wit_bindgen::generate!({
    path: "../../crates/policy/wit",
    world: "node-score",
});

const GPU_NODE_PENALTY: i64 = 1_000_000;

struct BestFit;

impl Guest for BestFit {
    fn score(job: Job, candidates: Vec<Node>, _now: u64) -> Vec<i64> {
        candidates
            .iter()
            .map(|n| {
                let left = i64::from(n.free_cores) - i64::from(job.cores);
                let gpu_penalty = if job.gpus == 0 && n.gpus > 0 {
                    GPU_NODE_PENALTY
                } else {
                    0
                };
                -left - gpu_penalty
            })
            .collect()
    }
}

export!(BestFit);
