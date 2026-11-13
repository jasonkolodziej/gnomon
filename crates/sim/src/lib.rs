//! `gnomon-sim` — deterministic replay and counterfactual policy evaluation.
//!
//! Because Gnomon's state is a pure function of its event journal (the FSM
//! applies events in order against an injected clock) and scheduling
//! decisions are pure functions of (graph, job, now), the *production*
//! scheduler can be re-run against recorded history:
//!
//! * [`replay::verify_exact`] — re-apply a journal and prove the state is
//!   bit-for-bit identical (what a Raft follower does).
//! * [`replay::workload_from_journal`] + [`Simulator`] — extract what was
//!   submitted and how long jobs really ran, then re-schedule that history
//!   under a different policy: a counterfactual, using the real code.
//! * `ShadowPolicy` (in `gnomon-sched`) — run a candidate next to the live
//!   policy and record disagreements without acting on them.
//!
//! Workloads can also come from Standard Workload Format traces ([`swf`]).

pub mod metrics;
pub mod replay;
pub mod swf;

use gnomon_core::{Clock, Event, Job, JobId, JobState, ManualClock, Resources};
use gnomon_fsm::Fsm;
use gnomon_graph::CapacityProfile;
use gnomon_sched::{backfill_pass, PassOptions, PlanApplier, Policy};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::Arc;

/// A job to simulate: what the user asked for plus how long it really runs.
#[derive(Clone, Debug, PartialEq)]
pub struct SimJob {
    pub job: Job,
    /// Actual runtime in seconds (clipped to walltime: the kill limit).
    pub actual: u64,
}

/// Per-job outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub id: JobId,
    pub submit: u64,
    pub start: u64,
    pub end: u64,
    pub cores: u32,
    pub walltime: u64,
}

#[derive(Clone, Debug)]
pub struct SimResult {
    pub policy: String,
    pub outcomes: Vec<Outcome>,
    pub extensions: u64,
    pub passes: u64,
    /// Jobs that could never be placed (request exceeds machine).
    pub unplaced: usize,
    pub journal: Vec<gnomon_fsm::JournalEntry>,
}

/// Machine description: one vertex per node.
#[derive(Clone, Debug)]
pub struct Machine {
    pub nodes: Vec<(String, Resources)>,
    /// Capacity profile the *planner* sees (None = planner unaware).
    pub profile: Option<CapacityProfile>,
}

impl Machine {
    /// A single vertex with `cores` processors — the standard model for SWF
    /// traces, which record processor counts rather than node placement.
    pub fn single(cores: u32) -> Self {
        Machine {
            nodes: vec![(
                "machine".into(),
                Resources {
                    cores,
                    mem_mb: 0,
                    gpus: 0,
                },
            )],
            profile: None,
        }
    }
    pub fn total_cores(&self) -> u32 {
        self.nodes.iter().map(|n| n.1.cores).sum()
    }
}

pub struct Simulator {
    pub machine: Machine,
    pub policy: Arc<dyn Policy>,
    pub opts: PassOptions,
    /// Record the FSM journal (needed for replay verification).
    pub record: bool,
}

impl Simulator {
    pub fn new(machine: Machine, policy: Arc<dyn Policy>) -> Self {
        Simulator {
            machine,
            policy,
            opts: PassOptions::default(),
            record: true,
        }
    }

    /// Run the workload to completion. Event loop:
    /// finishes → arrivals → one `backfill_pass` per distinct timestamp;
    /// wake-ups at job finishes, arrivals and reservation start times.
    pub fn run(&self, workload: &[SimJob]) -> SimResult {
        let t0 = workload
            .iter()
            .map(|w| w.job.submit_time)
            .min()
            .unwrap_or(0);
        let clock = Arc::new(ManualClock::new(t0));
        let fsm = if self.record {
            Fsm::recording(clock.clone())
        } else {
            Fsm::with_clock(clock.clone())
        };
        for (name, cap) in &self.machine.nodes {
            fsm.apply(Event::NodeRegistered {
                name: name.clone(),
                capacity: *cap,
            });
        }
        // Profiles are configuration, not events: set them directly on the graph
        // the FSM owns via a registration-time helper.
        if let Some(p) = &self.machine.profile {
            set_profiles(&fsm, p.clone());
        }
        let applier = PlanApplier::new();

        let mut arrivals: Vec<&SimJob> = workload.iter().collect();
        arrivals.sort_by_key(|w| (w.job.submit_time, w.job.id));
        let actual: BTreeMap<JobId, u64> = workload.iter().map(|w| (w.job.id, w.actual)).collect();
        let mut ai = 0usize;
        let mut finishes: BinaryHeap<Reverse<(u64, JobId)>> = BinaryHeap::new();
        let mut started: BTreeMap<JobId, u64> = BTreeMap::new();
        let mut outcomes = Vec::with_capacity(workload.len());
        let (mut extensions, mut passes) = (0u64, 0u64);

        loop {
            // Next wake-up.
            let next_arrival = arrivals.get(ai).map(|w| w.job.submit_time);
            let next_finish = finishes.peek().map(|Reverse((t, _))| *t);
            let now = clock.now();
            let next_resv = fsm.read(|st| {
                st.jobs
                    .values()
                    .filter(|j| j.state == JobState::Reserved)
                    .filter_map(|j| j.planned_start)
                    .filter(|&s| s > now)
                    .min()
            });
            let Some(t) = [next_arrival, next_finish, next_resv]
                .into_iter()
                .flatten()
                .min()
            else {
                break;
            };
            clock.set(t);

            while let Some(&Reverse((ft, id))) = finishes.peek() {
                if ft > t {
                    break;
                }
                finishes.pop();
                fsm.apply(Event::JobFinished {
                    job: id,
                    end: ft,
                    code: 0,
                });
                let (job, s) = fsm.read(|st| (st.jobs[&id].clone(), started[&id]));
                self.policy.observe(&job, ft - s);
                outcomes.push(Outcome {
                    id,
                    submit: job.submit_time,
                    start: s,
                    end: ft,
                    cores: job.request.cores,
                    walltime: job.walltime.secs(),
                });
            }
            while ai < arrivals.len() && arrivals[ai].job.submit_time <= t {
                fsm.apply(Event::JobSubmitted(arrivals[ai].job.clone()));
                ai += 1;
            }
            let st = backfill_pass(&fsm, self.policy.as_ref(), &applier, self.opts);
            passes += 1;
            extensions += st.extended;
            // Newly running jobs get their (hidden) true finish time.
            let newly: Vec<(JobId, u64)> = fsm.read(|s| {
                s.jobs
                    .values()
                    .filter(|j| j.state == JobState::Running && !started.contains_key(&j.id))
                    .map(|j| (j.id, j.planned_start.unwrap()))
                    .collect()
            });
            for (id, s) in newly {
                started.insert(id, s);
                finishes.push(Reverse((s + actual[&id], id)));
            }
        }
        let unplaced = fsm.read(|st| {
            st.jobs
                .values()
                .filter(|j| j.state == JobState::Pending)
                .count()
        });
        outcomes.sort_by_key(|o| o.id);
        SimResult {
            policy: self.policy.name(),
            outcomes,
            extensions,
            passes,
            unplaced,
            journal: fsm.journal(),
        }
    }
}

fn set_profiles(fsm: &Fsm, p: CapacityProfile) {
    let names: Vec<String> = fsm.read(|st| {
        st.graph
            .leaves()
            .map(|id| st.graph.vertex(id).name.clone())
            .collect()
    });
    for name in names {
        fsm.apply(Event::NodeProfileSet {
            name,
            profile: Some(p.clone()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnomon_core::Walltime;
    use gnomon_sched::{
        BackfillPolicy, BiasConfig, BiasCorrectedEstimator, GuardMode, QuantileEstimator,
    };
    use std::time::Duration;

    /// Small deterministic synthetic workload with heavy walltime overestimation.
    fn workload(n: u64) -> Vec<SimJob> {
        let mut x: u64 = 42;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        (0..n)
            .map(|i| {
                let wall = 600 + rnd() % 7200;
                let actual = (wall * (5 + rnd() % 95) / 100).max(1);
                let mut job = Job::new(
                    JobId(i + 1),
                    "s",
                    &format!("u{}", rnd() % 5),
                    Resources {
                        cores: 1 + (rnd() % 16) as u32,
                        mem_mb: 0,
                        gpus: 0,
                    },
                    Walltime(Duration::from_secs(wall)),
                );
                job.submit_time = 10_000 + i * 120;
                job.priority = 0;
                SimJob { job, actual }
            })
            .collect()
    }

    #[test]
    fn simulation_completes_and_respects_capacity() {
        let w = workload(300);
        let r = Simulator::new(Machine::single(32), Arc::new(BackfillPolicy::default())).run(&w);
        assert_eq!(r.outcomes.len(), 300);
        assert_eq!(r.unplaced, 0);
        // Never more than 32 cores busy at once.
        let mut ev: Vec<(u64, i64)> = r
            .outcomes
            .iter()
            .flat_map(|o| [(o.start, o.cores as i64), (o.end, -(o.cores as i64))])
            .collect();
        ev.sort();
        let mut used = 0;
        for (_, d) in ev {
            used += d;
            assert!(used <= 32, "oversubscribed: {used}");
        }
        // Every job ran exactly its actual runtime, after submission.
        for (o, s) in r.outcomes.iter().zip(w.iter()) {
            assert_eq!(o.end - o.start, s.actual);
            assert!(o.start >= o.submit);
        }
    }

    #[test]
    fn journal_replay_is_exact_and_counterfactual_reproduces_history() {
        let w = workload(200);
        let sim = Simulator::new(Machine::single(32), Arc::new(BackfillPolicy::default()));
        let r = sim.run(&w);
        // 1. Exact replay of the journal (JSON round-trip included).
        let journal = replay::from_jsonl(&replay::to_jsonl(&r.journal))
            .expect("generated JSONL should parse");
        assert_eq!(journal, r.journal);
        let a = replay::rebuild(&journal);
        let b = replay::rebuild(&r.journal);
        assert_eq!(replay::fingerprint(&a), replay::fingerprint(&b));
        // 2. Recover the workload from the journal and re-run the same policy:
        //    identical start times for every job.
        let recovered = replay::workload_from_journal(&journal);
        assert_eq!(recovered.len(), 200);
        let r2 = sim.run(&recovered);
        let starts = |r: &SimResult| {
            r.outcomes
                .iter()
                .map(|o| (o.id, o.start))
                .collect::<Vec<_>>()
        };
        assert_eq!(starts(&r), starts(&r2));
        // 3. A different policy on the same history gives a different schedule.
        let est = Arc::new(QuantileEstimator::new(0.7));
        let r3 = Simulator::new(
            Machine::single(32),
            Arc::new(BackfillPolicy::with_estimator(48 * 3600, est)),
        )
        .run(&recovered);
        assert_ne!(starts(&r), starts(&r3));
    }

    #[test]
    fn predictions_never_oversubscribe_even_when_wrong() {
        // theta = 0.3: most jobs overrun their planned span.
        let w = workload(300);
        let est = Arc::new(QuantileEstimator::new(0.3));
        let r = Simulator::new(
            Machine::single(32),
            Arc::new(BackfillPolicy::with_estimator(48 * 3600, est)),
        )
        .run(&w);
        assert_eq!(r.outcomes.len(), 300);
        assert!(r.extensions > 0);
        let mut ev: Vec<(u64, i64)> = r
            .outcomes
            .iter()
            .flat_map(|o| [(o.start, o.cores as i64), (o.end, -(o.cores as i64))])
            .collect();
        ev.sort();
        let mut used = 0;
        for (_, d) in ev {
            used += d;
            assert!(used <= 32, "oversubscribed: {used}");
        }
    }

    #[test]
    fn bias_corrected_never_oversubscribes_even_when_badly_wrong() {
        // Warm-started on a lie (every job ran 1 s) and planning at a low
        // level: nearly every job overruns. Both guard modes, with slip
        // protection and wide jobs (>= 8 of 32 cores) in the mix.
        let w = workload(300);
        for guard_mode in [GuardMode::Block, GuardMode::Record] {
            let est = Arc::new(BiasCorrectedEstimator::new(BiasConfig {
                tau: 0.1,
                guard_tau: Some(0.2),
                guard_mode,
                ..BiasConfig::default()
            }));
            for s in &w {
                est.observe(&s.job, 1);
            }
            let r = Simulator::new(
                Machine::single(32),
                Arc::new(BackfillPolicy::with_estimator(48 * 3600, est)),
            )
            .run(&w);
            assert_eq!(r.outcomes.len(), 300, "{guard_mode:?}");
            assert_eq!(r.unplaced, 0);
            assert!(r.extensions > 0);
            let mut ev: Vec<(u64, i64)> = r
                .outcomes
                .iter()
                .flat_map(|o| [(o.start, o.cores as i64), (o.end, -(o.cores as i64))])
                .collect();
            ev.sort();
            let mut used = 0;
            for (_, d) in ev {
                used += d;
                assert!(used <= 32, "oversubscribed: {used} ({guard_mode:?})");
            }
            for (o, s) in r.outcomes.iter().zip(w.iter()) {
                assert_eq!(o.end - o.start, s.actual, "never killed early");
            }
        }
    }

    #[test]
    fn capacity_profile_is_honoured_by_aware_planner() {
        let w = workload(300);
        let cap = CapacityProfile::daily(vec![
            (
                0,
                Resources {
                    cores: 32,
                    mem_mb: 0,
                    gpus: 0,
                },
            ),
            (
                16 * 3600,
                Resources {
                    cores: 16,
                    mem_mb: 0,
                    gpus: 0,
                },
            ),
            (
                20 * 3600,
                Resources {
                    cores: 32,
                    mem_mb: 0,
                    gpus: 0,
                },
            ),
        ]);
        let mut m = Machine::single(32);
        m.profile = Some(cap.clone());
        let r = Simulator::new(m, Arc::new(BackfillPolicy::default())).run(&w);
        assert_eq!(r.outcomes.len(), 300);
        let e = metrics::energy(&r, &cap, 32, &[400.0; 24], 10.0);
        assert_eq!(
            e.over_cap_core_h, 0.0,
            "aware planner never exceeds the cap"
        );
        let naive =
            Simulator::new(Machine::single(32), Arc::new(BackfillPolicy::default())).run(&w);
        let e2 = metrics::energy(&naive, &cap, 32, &[400.0; 24], 10.0);
        assert!(e2.over_cap_core_h > 0.0, "unaware planner violates the cap");
    }
}
