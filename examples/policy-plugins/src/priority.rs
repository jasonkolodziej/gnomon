//! Priority plugin (ADR 0002): a multifactor priority in the style of
//! Slurm's `priority/multifactor`:
//!
//! * **age**: 10 points per minute waited, capped at 7 days, so old jobs
//!   cannot starve;
//! * **size**: small jobs first, up to 10 000 points for a single core,
//!   falling with the total core count (every node of a gang);
//! * **partition**: `debug` jobs get a flat 50 000 boost.

wit_bindgen::generate!({
    path: "../../crates/policy/wit",
    world: "priority",
});

const AGE_POINTS_PER_MIN: u64 = 10;
const AGE_CAP_SECS: u64 = 7 * 24 * 3600;
const SIZE_POINTS: u64 = 10_000;
const DEBUG_BOOST: u64 = 50_000;

struct Multifactor;

impl Guest for Multifactor {
    fn priority(job: Job, now: u64) -> u64 {
        let waited = now.saturating_sub(job.submit_time).min(AGE_CAP_SECS);
        let age = waited / 60 * AGE_POINTS_PER_MIN;
        let cores = u64::from(job.cores.max(1)) * u64::from(job.nodes.max(1));
        let size = SIZE_POINTS / cores;
        let partition = if job.partition == "debug" {
            DEBUG_BOOST
        } else {
            0
        };
        age + size + partition
    }
}

export!(Multifactor);
