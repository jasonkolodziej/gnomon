//! gnomon-ctl: the controller daemon skeleton.
//!
//! Wires the pipeline from the report's proposed architecture:
//! submit -> EvalBroker (coalescing) -> worker threads (pure plans over
//! snapshots) -> PlanApplier (optimistic, single-writer) -> FSM events.
//!
//! Server mode adds a 1 s periodic `backfill_pass` (reservation re-planning,
//! overrun extension) next to the event-driven broker workers.

use gnomon_core::{Event, Job, Resources, Walltime};
use gnomon_fsm::Fsm;
use gnomon_sched::{BackfillPolicy, EvalBroker, PlanApplier, Policy};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

fn demo() {
    let fsm = Arc::new(Fsm::new());
    let broker = Arc::new(EvalBroker::new());
    let applier = Arc::new(PlanApplier::new());
    let policy: Arc<dyn Policy> = Arc::new(BackfillPolicy::new(48 * 3600));

    // Register a small demo cluster: 4 nodes x (16 cores, 64GB, 4 GPUs).
    for n in 0..4 {
        fsm.apply(Event::NodeRegistered {
            name: format!("node{n:03}"),
            capacity: Resources { cores: 16, mem_mb: 65536, gpus: 4 },
        });
    }

    // Submit 64 demo jobs with mixed shapes.
    for i in 0..64u64 {
        let id = fsm.next_job_id();
        let req = if i % 3 == 0 {
            Resources { cores: 8, mem_mb: 16384, gpus: 2 }
        } else {
            Resources { cores: 4, mem_mb: 8192, gpus: 0 }
        };
        let mut job = Job::new(id, &format!("demo-{i}"), "alice", req,
                               Walltime(Duration::from_secs(600)));
        job.priority = 1000 - i; // earlier submissions rank higher
        fsm.apply(Event::JobSubmitted(job.clone()));
        broker.enqueue(&job, fsm.index());
    }
    // Duplicate enqueues demonstrate coalescing.
    let snap = fsm.snapshot();
    for j in snap.state().jobs.values() {
        broker.enqueue(j, snap.index);
    }

    // Worker pool: N threads pulling evals, computing plans as pure
    // functions of snapshots, and committing optimistically (one inline
    // re-evaluation on conflict, then re-enqueue for a later pass).
    let n_workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    std::thread::scope(|s| {
        for _ in 0..n_workers {
            let (broker, fsm, policy, applier) =
                (broker.clone(), fsm.clone(), policy.clone(), applier.clone());
            s.spawn(move || {
                while let Some(eval) = broker.dequeue() {
                    let snap = fsm.snapshot();
                    let Some(job) = snap.job(eval.job) else { continue };
                    if job.state != gnomon_core::JobState::Pending { continue; }
                    let plan = policy.schedule(&snap, job);
                    if applier.apply(&fsm, &plan).is_err() {
                        // Optimistic conflict: re-evaluate once against fresh state.
                        let snap2 = fsm.snapshot();
                        if let Some(job2) = snap2.job(plan.job) {
                            if job2.state == gnomon_core::JobState::Pending {
                                let retry = policy.schedule(&snap2, job2);
                                let _ = applier.apply(&fsm, &retry);
                            }
                        }
                    }
                }
            });
        }
    });

    let final_snap = fsm.snapshot();
    let (mut running, mut reserved, mut pending) = (0, 0, 0);
    for j in final_snap.state().jobs.values() {
        match j.state {
            gnomon_core::JobState::Running => running += 1,
            gnomon_core::JobState::Reserved => reserved += 1,
            gnomon_core::JobState::Pending => pending += 1,
            _ => {}
        }
    }
    println!("gnomon-ctl demo run");
    println!("  fsm index:        {}", fsm.index());
    println!("  running/reserved/pending: {running}/{reserved}/{pending}");
    println!("  plans committed:  {}", applier.committed.load(Ordering::Relaxed));
    println!("  evals coalesced:  {}", broker.coalesced.load(Ordering::Relaxed));
    println!("  plan conflicts:   {}", applier.conflicts.load(Ordering::Relaxed));
}


/// Server mode: gRPC API + background scheduler loop (phases 3-4).
#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("--demo") {
        demo();
        return;
    }
    use gnomon_api::SchedulerSvc;
    use gnomon_sched::{BackfillPolicy, EvalBroker, PlanApplier, Policy};
    use std::sync::Arc;

    let fsm = Arc::new(Fsm::new());
    let broker = Arc::new(EvalBroker::new());
    let applier = Arc::new(PlanApplier::new());
    let policy: Arc<dyn Policy> = Arc::new(BackfillPolicy::new(48 * 3600));

    // Background scheduler loop: dequeue evals, plan on snapshots, commit.
    {
        let (fsm, broker, applier, policy) =
            (fsm.clone(), broker.clone(), applier.clone(), policy.clone());
        std::thread::spawn(move || loop {
            match broker.dequeue() {
                Some(eval) => {
                    let snap = fsm.snapshot();
                    if let Some(job) = snap.job(eval.job) {
                        if job.state == gnomon_core::JobState::Pending {
                            let plan = policy.schedule(&snap, job);
                            if applier.apply(&fsm, &plan).is_err() {
                                broker.enqueue(job, fsm.index());
                            }
                        }
                    }
                }
                None => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        });
    }
    // Periodic conservative-backfill pass: re-plans reservations (so reserved
    // jobs start when their time comes or earlier), and extends predicted
    // spans that overran. Safe alongside the broker workers because the
    // applier uses per-vertex optimistic concurrency.
    {
        let (fsm, applier, policy) = (fsm.clone(), applier.clone(), policy.clone());
        std::thread::spawn(move || loop {
            gnomon_sched::backfill_pass(&fsm, policy.as_ref(), &applier, Default::default());
            std::thread::sleep(std::time::Duration::from_secs(1));
        });
    }

    let addr = "0.0.0.0:7631".parse().unwrap(); // neighbors Slurm's 6817
    println!("gnomon-ctl listening on {addr}");
    tonic::transport::Server::builder()
        .add_service(SchedulerSvc { fsm, broker }.server())
        .serve(addr)
        .await
        .expect("gRPC server");
}
