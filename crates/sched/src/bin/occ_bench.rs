//! Controller-path benchmark for the optimistic-concurrency fix.
//!
//! Unlike `bench` (which measures pure planning throughput and applies
//! events directly), this drives the real controller loop: worker threads
//! dequeue evaluations from the [`EvalBroker`], plan against an FSM
//! snapshot, and commit through the [`PlanApplier`], re-enqueueing on
//! conflict — while a background thread applies node heartbeats at a fixed
//! rate, as live agents do.
//!
//! Compares `ConflictMode::GlobalIndex` (original skeleton: any event since
//! the snapshot invalidates a plan) with `ConflictMode::PerVertex`.
//!
//! Output CSV: mode,nodes,jobs,workers,hb_per_sec,placed,evals,conflicts,millis,jobs_per_sec

use gnomon_core::{Event, Job, JobId, JobState, Resources, Walltime};
use gnomon_fsm::Fsm;
use gnomon_sched::{BackfillPolicy, ConflictMode, EvalBroker, PlanApplier, Policy};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn setup(nodes: u32, jobs: u64) -> (Arc<Fsm>, Arc<EvalBroker>) {
    let fsm = Arc::new(Fsm::new());
    let broker = Arc::new(EvalBroker::new());
    for n in 0..nodes {
        fsm.apply(Event::NodeRegistered { name: format!("node{n:04}"),
            capacity: Resources { cores: 32, mem_mb: 131072, gpus: 4 } });
    }
    for i in 0..jobs {
        let req = match i % 4 {
            0 => Resources { cores: 16, mem_mb: 32768, gpus: 2 },
            1 => Resources { cores: 8, mem_mb: 16384, gpus: 1 },
            _ => Resources { cores: 4, mem_mb: 8192, gpus: 0 },
        };
        let mut job = Job::new(JobId(i + 1), "b", "bench", req, Walltime(Duration::from_secs(300)));
        job.priority = 10_000_000 - i;
        fsm.apply(Event::JobSubmitted(job.clone()));
        broker.enqueue(&job, fsm.index());
    }
    (fsm, broker)
}

fn run(mode: ConflictMode, nodes: u32, jobs: u64, workers: usize, hb_per_sec: u64) -> String {
    let (fsm, broker) = setup(nodes, jobs);
    let policy: Arc<dyn Policy> = Arc::new(BackfillPolicy::new(72 * 3600));
    let applier = Arc::new(PlanApplier::with_mode(mode));
    let evals = Arc::new(AtomicU64::new(0));
    let in_flight = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs(20);

    std::thread::scope(|s| {
        // Heartbeat source (live agents).
        if hb_per_sec > 0 {
            let (fsm, stop) = (fsm.clone(), stop.clone());
            s.spawn(move || {
                let gap = Duration::from_nanos(1_000_000_000 / hb_per_sec);
                let mut k = 0u32;
                while !stop.load(Ordering::Relaxed) {
                    fsm.apply(Event::Heartbeat { name: format!("node{:04}", k % nodes), allocated: Resources::default() });
                    k += 1;
                    std::thread::sleep(gap);
                }
            });
        }
        let mut hs = Vec::new();
        for _ in 0..workers {
            let (fsm, broker, policy, applier, evals, in_flight) =
                (fsm.clone(), broker.clone(), policy.clone(), applier.clone(), evals.clone(), in_flight.clone());
            hs.push(s.spawn(move || {
                let mut idle = 0;
                while Instant::now() < deadline {
                    in_flight.fetch_add(1, Ordering::SeqCst);
                    let Some(eval) = broker.dequeue() else {
                        in_flight.fetch_sub(1, Ordering::SeqCst);
                        if in_flight.load(Ordering::SeqCst) == 0 { idle += 1; if idle > 3 { break; } }
                        std::thread::sleep(Duration::from_micros(200));
                        continue;
                    };
                    idle = 0;
                    let snap = fsm.snapshot();
                    if let Some(job) = snap.job(eval.job) {
                        if job.state == JobState::Pending {
                            evals.fetch_add(1, Ordering::Relaxed);
                            let plan = policy.schedule(&snap, job);
                            if applier.apply(&fsm, &plan).is_err() {
                                broker.enqueue(job, fsm.index());
                            }
                        }
                    }
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                }
            }));
        }
        for h in hs { h.join().unwrap(); }
        stop.store(true, Ordering::Relaxed);
    });
    let ms = t0.elapsed().as_millis().max(1);
    let placed = fsm.read(|st| st.jobs.values().filter(|j| j.state != JobState::Pending).count());
    format!("{mode:?},{nodes},{jobs},{workers},{hb_per_sec},{placed},{},{},{ms},{:.0}",
        evals.load(Ordering::Relaxed), applier.conflicts.load(Ordering::Relaxed),
        placed as f64 * 1000.0 / ms as f64)
}

fn main() {
    println!("mode,nodes,jobs,workers,hb_per_sec,placed,evals,conflicts,millis,jobs_per_sec");
    for &hb in &[0u64, 100, 1000] {
        for &workers in &[1usize, 2, 4] {
            for mode in [ConflictMode::GlobalIndex, ConflictMode::PerVertex] {
                println!("{}", run(mode, 256, 2048, workers, hb));
            }
        }
    }
}
