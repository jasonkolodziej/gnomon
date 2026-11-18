//! Policy lab: the same synthetic workload scheduled by each policy with the
//! production code path (`gnomon-sim`), side by side.

use gnomon_core::{Job, JobId, Resources, Walltime};
use gnomon_sched::{
    BackfillPolicy, BiasConfig, BiasCorrectedEstimator, Policy, QuantileEstimator, WideSafeBackfill,
};
use gnomon_sim::metrics::summarize;
use gnomon_sim::{Machine, SimJob, Simulator};
use gnomon_web_model::{LabReport, LabRequest, LabRow};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WINDOW_SECS: u64 = 48 * 3600;

/// Bounds that keep one lab run to about a second.
pub fn validate(r: &LabRequest) -> Result<(), String> {
    if !(1..=64).contains(&r.nodes) {
        return Err("nodes must be 1-64".into());
    }
    if !(1..=256).contains(&r.cores_per_node) {
        return Err("cores_per_node must be 1-256".into());
    }
    if !(10..=3000).contains(&r.jobs) {
        return Err("jobs must be 10-3000".into());
    }
    Ok(())
}

/// Deterministic xorshift, so a seed always gives the same workload.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Jobs that overestimate their walltime (as real users do: actual runtime
/// 5-100% of it), some of them gangs, arriving every 1-5 minutes: on the
/// default 16x16 machine that is roughly 90% load, where backfill policies
/// differ (an idle machine schedules everything at once under any policy).
pub fn workload(r: &LabRequest) -> Vec<SimJob> {
    let mut rng = Rng(r.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut t = 10_000;
    (0..u64::from(r.jobs))
        .map(|i| {
            t += 60 + rng.next() % 260;
            let wall = 600 + rng.next() % 14_400;
            let actual = (wall * (5 + rng.next() % 96) / 100).max(1);
            let cores = 1 + (rng.next() % u64::from(r.cores_per_node)) as u32;
            let mut job = Job::new(
                JobId(i + 1),
                "lab",
                &format!("u{}", rng.next() % 6),
                Resources {
                    cores,
                    mem_mb: 0,
                    gpus: 0,
                },
                Walltime(Duration::from_secs(wall)),
            );
            if r.gang_every > 0 && (i + 1) % u64::from(r.gang_every) == 0 && r.nodes > 1 {
                job.nodes = 2 + (rng.next() % 7) as u32;
                job.nodes = job.nodes.min(r.nodes);
            }
            job.submit_time = t;
            SimJob { job, actual }
        })
        .collect()
}

fn machine(r: &LabRequest) -> Machine {
    Machine {
        nodes: (0..r.nodes)
            .map(|i| {
                (
                    format!("node{i:02}"),
                    Resources {
                        cores: r.cores_per_node,
                        mem_mb: 0,
                        gpus: 0,
                    },
                )
            })
            .collect(),
        profile: None,
    }
}

/// The policies compared, with a short label for the UI.
fn policies() -> Vec<(&'static str, Arc<dyn Policy>)> {
    let bias = || Arc::new(BiasCorrectedEstimator::new(BiasConfig::default()));
    vec![
        (
            "walltime backfill (default)",
            Arc::new(BackfillPolicy::new(WINDOW_SECS)),
        ),
        (
            "runtime quantile θ=0.9",
            Arc::new(BackfillPolicy::with_estimator(
                WINDOW_SECS,
                Arc::new(QuantileEstimator::new(0.9)),
            )),
        ),
        (
            "bias-corrected predictor",
            Arc::new(BackfillPolicy::with_estimator(WINDOW_SECS, bias())),
        ),
        (
            "bias-corrected + wide-job shield",
            Arc::new(WideSafeBackfill::new(WINDOW_SECS, bias())),
        ),
    ]
}

pub fn run(r: &LabRequest) -> LabReport {
    let t0 = Instant::now();
    let jobs = workload(r);
    let m = machine(r);
    let cores = m.total_cores();
    let rows = policies()
        .into_iter()
        .map(|(label, policy)| {
            let mut sim = Simulator::new(m.clone(), policy);
            sim.record = false;
            let s = summarize(&sim.run(&jobs), cores);
            LabRow {
                policy: s.policy,
                label: label.into(),
                mean_wait_h: s.mean_wait_h,
                p95_wait_h: s.p95_wait_h,
                mean_bsld: s.mean_bsld,
                wide_wait_h: s.wide_wait_h,
                utilization: s.utilization,
                makespan_h: s.makespan_h,
            }
        })
        .collect();
    LabReport {
        request: r.clone(),
        rows,
        millis: t0.elapsed().as_millis() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> LabRequest {
        LabRequest {
            nodes: 4,
            cores_per_node: 8,
            jobs: 120,
            gang_every: 5,
            seed: 7,
        }
    }

    #[test]
    fn workloads_are_deterministic_and_within_the_machine() {
        let (a, b) = (workload(&small()), workload(&small()));
        assert_eq!(a.len(), 120);
        assert!(a
            .iter()
            .zip(&b)
            .all(|(x, y)| x.job == y.job && x.actual == y.actual));
        assert!(a
            .iter()
            .all(|j| j.job.request.cores <= 8 && j.job.nodes <= 4));
        assert!(a.iter().any(|j| j.job.nodes > 1), "some gangs");
        assert!(a.iter().all(|j| j.actual <= j.job.walltime.secs()));
    }

    #[test]
    fn every_policy_schedules_every_job() {
        let report = run(&small());
        assert_eq!(report.rows.len(), 4);
        for row in &report.rows {
            assert!(row.utilization > 0.0 && row.utilization <= 1.0, "{row:?}");
            assert!(row.mean_wait_h >= 0.0 && row.makespan_h > 0.0, "{row:?}");
        }
        assert_eq!(report.rows[0].policy, "backfill/walltime");
    }

    #[test]
    fn oversized_requests_are_refused() {
        let mut r = small();
        r.jobs = 1_000_000;
        assert!(validate(&r).is_err());
        assert!(validate(&small()).is_ok());
    }
}
