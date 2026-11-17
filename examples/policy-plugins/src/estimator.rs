//! Runtime estimator plugin (ADR 0002, step 5).
//!
//! A rule-of-thumb stand-in for a trained model (whose weights could be
//! compiled in just as well): users overestimate long jobs most, so plan
//! jobs over an hour at 60% of their walltime and short ones at their
//! walltime (`none`). The host clamps the estimate to `[1, walltime]`, and
//! the walltime stays the kill limit: a job outliving its estimate is
//! extended, never stopped early.

wit_bindgen::generate!({
    path: "../../crates/policy/wit",
    world: "estimator",
});

struct Fraction;

impl Guest for Fraction {
    fn estimate(job: Job) -> Option<u64> {
        (job.walltime_secs > 3600).then(|| job.walltime_secs / 10 * 6)
    }
}

export!(Fraction);
