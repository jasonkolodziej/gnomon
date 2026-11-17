//! Admission plugin (ADR 0003), the equivalent of a site's `job_submit.lua`:
//!
//! * jobs asking for more than 7 days are rejected;
//! * `debug` jobs are capped at one hour and one node (rewritten, not
//!   rejected, as sites commonly do);
//! * GPU jobs submitted to `normal` move to the `gpu` partition.

wit_bindgen::generate!({
    path: "../../crates/policy/wit",
    world: "admission",
});

use gnomon::policy::types::Request;

const MAX_WALLTIME_SECS: u64 = 7 * 24 * 3600;
const DEBUG_MAX_SECS: u64 = 3600;

struct SiteRules;

impl Guest for SiteRules {
    fn admit(job: Job, _now: u64) -> Verdict {
        if job.walltime_secs > MAX_WALLTIME_SECS {
            return Verdict::Reject("walltime over 7 days".into());
        }
        let mut req = Request {
            partition: job.partition.clone(),
            cores: job.cores,
            mem_mb: job.mem_mb,
            gpus: job.gpus,
            nodes: job.nodes,
            walltime_secs: job.walltime_secs,
        };
        if job.partition == "debug" {
            req.walltime_secs = req.walltime_secs.min(DEBUG_MAX_SECS);
            req.nodes = 1;
        }
        if job.partition == "normal" && job.gpus > 0 {
            req.partition = "gpu".into();
        }
        let unchanged = req.partition == job.partition
            && req.nodes == job.nodes
            && req.walltime_secs == job.walltime_secs;
        if unchanged {
            Verdict::Accept
        } else {
            Verdict::Rewrite(req)
        }
    }
}

export!(SiteRules);
