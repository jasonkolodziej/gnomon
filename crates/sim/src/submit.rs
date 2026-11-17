//! Offline evaluation of submit-time policy, such as candidate WASM
//! admission and priority plugins (ADR 0002, 0003), on recorded or SWF
//! workloads.
//!
//! The controller admits a job and computes its priority once, at submit
//! time, and records the result in `JobSubmitted`; [`admit`] and [`assign`]
//! do the same for a workload, so the [`Simulator`](crate::Simulator) then
//! schedules it exactly as the controller would have.

use crate::SimJob;
use gnomon_core::Job;

/// Set every job's priority to `priority(job, submit_time)`, as the submit
/// handler would have at the moment of submission.
pub fn assign(workload: &mut [SimJob], mut priority: impl FnMut(&Job, u64) -> u64) {
    for w in workload {
        w.job.priority = priority(&w.job, w.job.submit_time);
    }
}

/// What [`admit`] did to a workload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Admitted {
    pub rejected: usize,
    pub rewritten: usize,
}

/// Run `admission(job, submit_time)` on every job, as the submit handler
/// would: rejected jobs (`Err`) are dropped, rewritten ones kept as
/// rewritten. A rewrite that shortens the walltime also shortens the actual
/// runtime, since the walltime is the kill limit.
pub fn admit(
    workload: &mut Vec<SimJob>,
    mut admission: impl FnMut(&mut Job, u64) -> Result<(), String>,
) -> Admitted {
    let mut out = Admitted::default();
    workload.retain_mut(|w| {
        let before = w.job.clone();
        let submitted = w.job.submit_time;
        if admission(&mut w.job, submitted).is_err() {
            out.rejected += 1;
            return false;
        }
        if w.job != before {
            out.rewritten += 1;
            w.actual = w.actual.min(w.job.walltime.secs()).max(1);
        }
        true
    });
    out
}

/// Fraction of job pairs that the priorities leave in first-come,
/// first-served order: for jobs `a` submitted before `b` (ties by id), the
/// pair is kept when `a` does not rank below `b`. 1.0 means the function
/// changes nothing relative to FCFS; lower values mean more reordering.
pub fn fcfs_agreement(workload: &[SimJob]) -> f64 {
    let mut jobs: Vec<&Job> = workload.iter().map(|w| &w.job).collect();
    jobs.sort_by_key(|j| (j.submit_time, j.id));
    let n = jobs.len();
    if n < 2 {
        return 1.0;
    }
    let mut kept = 0u64;
    for (i, a) in jobs.iter().enumerate() {
        kept += jobs[i + 1..]
            .iter()
            .filter(|b| a.priority >= b.priority)
            .count() as u64;
    }
    kept as f64 / (n as u64 * (n as u64 - 1) / 2) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Machine, Simulator};
    use gnomon_core::{JobId, Resources, Walltime};
    use gnomon_policy::{fixtures, PluginLimits, PriorityPlugin};
    use gnomon_sched::BackfillPolicy;
    use std::sync::Arc;
    use std::time::Duration;

    fn sim_job(id: u64, submit: u64, cores: u32) -> SimJob {
        let mut job = Job::new(
            JobId(id),
            "s",
            "u",
            Resources {
                cores,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(Duration::from_secs(1000)),
        );
        job.submit_time = submit;
        SimJob { job, actual: 1000 }
    }

    #[test]
    fn agreement_counts_pairs_kept_in_submit_order() {
        let mut w = vec![sim_job(1, 0, 1), sim_job(2, 10, 1), sim_job(3, 20, 1)];
        assert_eq!(fcfs_agreement(&w), 1.0); // all priority 0: FCFS
        w[2].job.priority = 5; // job 3 now jumps both earlier jobs
        assert!((fcfs_agreement(&w) - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn admission_drops_rejected_jobs_and_clips_rewritten_runtimes() {
        let mut w = vec![sim_job(1, 0, 1), sim_job(2, 10, 64), sim_job(3, 20, 2)];
        let r = admit(&mut w, |j, _| match j.request.cores {
            64 => Err("too big".into()),
            2 => {
                j.walltime = Walltime(Duration::from_secs(300));
                Ok(())
            }
            _ => Ok(()),
        });
        assert_eq!(
            r,
            Admitted {
                rejected: 1,
                rewritten: 1
            }
        );
        assert_eq!(w.iter().map(|s| s.job.id.0).collect::<Vec<_>>(), [1, 3]);
        assert_eq!((w[0].actual, w[1].actual), (1000, 300));
    }

    #[test]
    fn a_plugin_reorders_the_simulated_queue() {
        // 4-core machine: job 1 fills it until t=1000, then a 1-core job
        // (2) and a 4-core job (3) queue behind it. They cannot overlap with
        // job 1, and job 3 cannot overlap with job 2, so queue order decides
        // which starts at 1000: FCFS picks job 2, the size plugin job 3.
        let base = vec![sim_job(1, 0, 4), sim_job(2, 1, 1), sim_job(3, 2, 4)];
        let plugin =
            PriorityPlugin::from_bytes(fixtures::size_age().as_bytes(), PluginLimits::default())
                .unwrap();
        let mut w = base.clone();
        assign(&mut w, |j, now| plugin.priority(j, now).unwrap());
        assert_eq!(
            w.iter().map(|s| s.job.priority).collect::<Vec<_>>(),
            [400, 100, 400]
        );
        assert!(fcfs_agreement(&w) < 1.0);

        let run = |w: &[SimJob]| {
            let sim = Simulator::new(Machine::single(4), Arc::new(BackfillPolicy::new(86_400)));
            let r = sim.run(w);
            let start = |id| r.outcomes.iter().find(|o| o.id == JobId(id)).unwrap().start;
            (start(2), start(3))
        };
        assert_eq!(run(&base), (1000, 2000));
        assert_eq!(run(&w), (2000, 1000));
    }
}
