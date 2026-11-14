//! gnomon-ctl: the controller daemon skeleton.
//!
//! Wires the pipeline from the report's proposed architecture:
//! submit -> EvalBroker (coalescing) -> worker threads (pure plans over
//! snapshots) -> PlanApplier (optimistic, single-writer) -> FSM events.
//!
//! Server mode adds a 1 s periodic `backfill_pass` (reservation re-planning,
//! overrun extension) next to the event-driven broker workers.
//!
//! Durability: with `--state-dir <dir>` (or `GNOMON_STATE_DIR`) every applied
//! event is written to an fsync'd journal before it takes effect
//! ([`gnomon_consensus::journal::EventJournal`]), and a restart recovers
//! jobs, nodes, reservations and queued assignments exactly. Without it the
//! controller is in-memory, as before.

use gnomon_core::{Clock, Event, Job, Resources, SystemClock, Walltime};
use gnomon_fsm::Fsm;
use gnomon_sched::{BackfillPolicy, EvalBroker, PlanApplier, Policy};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// Where to persist controller state: `--state-dir <dir>` (or
/// `--state-dir=<dir>`) wins over the `GNOMON_STATE_DIR` value `env`.
/// A `--state-dir` without a directory is an error, never a silent fallback
/// to in-memory state: a typo must not quietly disable durability.
fn state_dir(args: &[String], env: Option<String>) -> Result<Option<PathBuf>, String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let v = if a == "--state-dir" {
            it.next().map(String::as_str)
        } else if let Some(v) = a.strip_prefix("--state-dir=") {
            Some(v)
        } else {
            continue;
        };
        return match v {
            Some(d) if !d.is_empty() && !d.starts_with("--") => Ok(Some(PathBuf::from(d))),
            _ => Err("--state-dir needs a directory".into()),
        };
    }
    Ok(env.filter(|v| !v.is_empty()).map(PathBuf::from))
}

/// The controller FSM: recovered from (and journaled to) `dir` when given,
/// otherwise in-memory.
fn open_fsm(dir: Option<&Path>, clock: Arc<dyn Clock>) -> std::io::Result<Arc<Fsm>> {
    match dir {
        Some(d) => gnomon_consensus::journal::EventJournal::open_fsm(d, clock),
        None => Ok(Arc::new(Fsm::with_clock(clock))),
    }
}

/// Recovered pending jobs need fresh evaluations: the broker is not durable.
fn requeue_pending(fsm: &Fsm, broker: &EvalBroker) -> usize {
    let snap = fsm.snapshot();
    let mut n = 0;
    for job in snap.pending_jobs() {
        broker.enqueue(job, snap.index);
        n += 1;
    }
    n
}

fn demo() {
    let fsm = Arc::new(Fsm::new());
    let broker = Arc::new(EvalBroker::new());
    let applier = Arc::new(PlanApplier::new());
    let policy: Arc<dyn Policy> = Arc::new(BackfillPolicy::new(48 * 3600));

    // Register a small demo cluster: 4 nodes x (16 cores, 64GB, 4 GPUs).
    for n in 0..4 {
        fsm.apply(Event::NodeRegistered {
            name: format!("node{n:03}"),
            capacity: Resources {
                cores: 16,
                mem_mb: 65536,
                gpus: 4,
            },
        });
    }

    // Submit 64 demo jobs with mixed shapes.
    for i in 0..64u64 {
        let id = fsm.next_job_id();
        let req = if i % 3 == 0 {
            Resources {
                cores: 8,
                mem_mb: 16384,
                gpus: 2,
            }
        } else {
            Resources {
                cores: 4,
                mem_mb: 8192,
                gpus: 0,
            }
        };
        let mut job = Job::new(
            id,
            &format!("demo-{i}"),
            "alice",
            req,
            Walltime(Duration::from_secs(600)),
        );
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
    let n_workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    std::thread::scope(|s| {
        for _ in 0..n_workers {
            let (broker, fsm, policy, applier) =
                (broker.clone(), fsm.clone(), policy.clone(), applier.clone());
            s.spawn(move || {
                while let Some(eval) = broker.dequeue() {
                    let snap = fsm.snapshot();
                    let Some(job) = snap.job(eval.job) else {
                        continue;
                    };
                    if job.state != gnomon_core::JobState::Pending {
                        continue;
                    }
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
    println!(
        "  plans committed:  {}",
        applier.committed.load(Ordering::Relaxed)
    );
    println!(
        "  evals coalesced:  {}",
        broker.coalesced.load(Ordering::Relaxed)
    );
    println!(
        "  plan conflicts:   {}",
        applier.conflicts.load(Ordering::Relaxed)
    );
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

    let dir = match state_dir(&args[1..], std::env::var("GNOMON_STATE_DIR").ok()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("gnomon-ctl: {e}");
            std::process::exit(2);
        }
    };
    let fsm = match open_fsm(dir.as_deref(), Arc::new(SystemClock)) {
        Ok(fsm) => fsm,
        Err(e) => {
            let d = dir.as_deref().unwrap_or(Path::new("?")).display();
            eprintln!("gnomon-ctl: cannot recover state from {d}: {e}");
            std::process::exit(1);
        }
    };
    let broker = Arc::new(EvalBroker::new());
    if let Some(d) = &dir {
        let snap = fsm.snapshot();
        let requeued = requeue_pending(&fsm, &broker);
        println!(
            "gnomon-ctl state dir {}: recovered index {}, {} jobs ({requeued} pending), {} nodes",
            d.display(),
            snap.index,
            snap.state().jobs.len(),
            snap.state().nodes.len()
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use gnomon_core::{JobId, JobState, ManualClock};

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn state_dir_flag_beats_env_and_empty_env_is_ignored() {
        let env = Some("/env".to_string());
        assert_eq!(
            state_dir(&args(&["--state-dir", "/flag"]), env.clone()),
            Ok(Some(PathBuf::from("/flag")))
        );
        assert_eq!(
            state_dir(&args(&["--state-dir=/eq"]), env.clone()),
            Ok(Some(PathBuf::from("/eq")))
        );
        assert_eq!(state_dir(&[], env), Ok(Some(PathBuf::from("/env"))));
        assert_eq!(state_dir(&[], Some(String::new())), Ok(None));
        assert_eq!(state_dir(&[], None), Ok(None));
    }

    #[test]
    fn state_dir_flag_without_a_directory_is_rejected() {
        // Even with GNOMON_STATE_DIR set: never silently fall back.
        let env = Some("/env".to_string());
        for bad in [
            &["--state-dir"][..],
            &["--state-dir="],
            &["--state-dir", "--other"],
        ] {
            assert!(state_dir(&args(bad), env.clone()).is_err(), "{bad:?}");
        }
    }

    fn submit(fsm: &Fsm, id: u64, cores: u32) {
        fsm.apply(Event::JobSubmitted(Job::new(
            JobId(id),
            "j",
            "u",
            Resources {
                cores,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(Duration::from_secs(600)),
        )));
    }

    #[test]
    fn restart_with_state_dir_recovers_jobs_nodes_and_reservations() {
        let dir = std::env::temp_dir().join(format!("gnomon-ctl-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let clock = Arc::new(ManualClock::new(1000));
        let fsm = open_fsm(Some(&dir), clock.clone()).unwrap();
        fsm.apply(Event::NodeRegistered {
            name: "n0".into(),
            capacity: Resources {
                cores: 4,
                mem_mb: 0,
                gpus: 0,
            },
        });
        submit(&fsm, 1, 4);
        submit(&fsm, 2, 4);
        submit(&fsm, 3, 1);
        // Drive the real scheduling path: job 1 runs, job 2 is reserved behind it.
        let policy = BackfillPolicy::new(48 * 3600);
        let applier = PlanApplier::new();
        for id in [1, 2] {
            let snap = fsm.snapshot();
            let plan = policy.schedule(&snap, snap.job(JobId(id)).unwrap());
            applier.apply(&fsm, &plan).unwrap();
        }
        let before = fsm.snapshot();
        assert_eq!(before.job(JobId(1)).unwrap().state, JobState::Running);
        assert_eq!(before.job(JobId(2)).unwrap().state, JobState::Reserved);
        let fp = fsm.read(|st| format!("{st:?}"));
        drop(fsm);

        clock.set(5000); // restart much later: recovery must not re-time events
        let fsm = open_fsm(Some(&dir), clock).unwrap();
        assert_eq!(fsm.read(|st| format!("{st:?}")), fp);
        assert_eq!(fsm.index(), before.index);
        assert_eq!(fsm.next_job_id(), JobId(4));
        let broker = EvalBroker::new();
        assert_eq!(requeue_pending(&fsm, &broker), 1);
        assert_eq!(broker.dequeue().map(|e| e.job), Some(JobId(3)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_state_dir_the_fsm_is_in_memory() {
        let fsm = open_fsm(None, Arc::new(ManualClock::new(0))).unwrap();
        submit(&fsm, 1, 1);
        let again = open_fsm(None, Arc::new(ManualClock::new(0))).unwrap();
        assert_eq!(again.index(), 0);
    }
}
