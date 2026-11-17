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
//!
//! Replication: with `--raft-id <n> --raft-peers 1=host:port,2=...` (or
//! `GNOMON_RAFT_ID` / `GNOMON_RAFT_PEERS`, plus a state dir) the controller is
//! one replica of a Raft group ([`gnomon_consensus::replica`]). Every write,
//! from the API, the plan applier or the backfill pass, is committed through
//! the replicated log, and followers forward writes to the leader. Only the
//! leader runs the scheduling loops: a new leader re-plans everything pending
//! on its first backfill pass. Agents and CLIs may list every controller in
//! `GNOMON_ADDR` and fail over between them.

mod plugins;

use gnomon_consensus::replica::{self, Peers};
use gnomon_core::{Clock, Event, Job, Resources, SystemClock, Walltime};
use gnomon_fsm::Fsm;
use gnomon_sched::{BackfillPolicy, EvalBroker, PlanApplier, Policy};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// The value of `--<name> <v>` / `--<name>=<v>` in `args`, else `env`.
/// A flag without a value is an error, never a silent fallback.
fn flag(args: &[String], name: &str, env: Option<String>) -> Result<Option<String>, String> {
    let long = format!("--{name}");
    let eq = format!("--{name}=");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let v = if *a == long {
            it.next().map(String::as_str)
        } else if let Some(v) = a.strip_prefix(&eq) {
            Some(v)
        } else {
            continue;
        };
        return match v {
            Some(v) if !v.is_empty() && !v.starts_with("--") => Ok(Some(v.to_string())),
            _ => Err(format!("--{name} needs a value")),
        };
    }
    Ok(env.filter(|v| !v.is_empty()))
}

/// Where to persist controller state: `--state-dir <dir>` (or
/// `--state-dir=<dir>`) wins over the `GNOMON_STATE_DIR` value `env`.
/// A `--state-dir` without a directory is an error, never a silent fallback
/// to in-memory state: a typo must not quietly disable durability.
fn state_dir(args: &[String], env: Option<String>) -> Result<Option<PathBuf>, String> {
    flag(args, "state-dir", env)
        .map(|d| d.map(PathBuf::from))
        .map_err(|_| "--state-dir needs a directory".into())
}

/// Seconds without a heartbeat after which the leader declares a node lost:
/// `GNOMON_NODE_TIMEOUT_SECS`, else three heartbeat intervals
/// (`GNOMON_HEARTBEAT_SECS`, default 10) but at least 30 s.
fn node_timeout(timeout: Option<String>, heartbeat: Option<String>) -> Result<u64, String> {
    if let Some(t) = timeout.filter(|t| !t.is_empty()) {
        return match t.parse::<u64>() {
            Ok(secs) if secs > 0 => Ok(secs),
            _ => Err(format!(
                "GNOMON_NODE_TIMEOUT_SECS '{t}' is not a positive number"
            )),
        };
    }
    let hb = heartbeat.and_then(|h| h.parse::<u64>().ok()).unwrap_or(10);
    Ok((3 * hb).max(30))
}

/// `(this node's id, every peer)` when the controller is replicated.
fn raft_config(
    args: &[String],
    id_env: Option<String>,
    peers_env: Option<String>,
) -> Result<Option<(u64, Peers)>, String> {
    let id = flag(args, "raft-id", id_env)?;
    let peers = flag(args, "raft-peers", peers_env)?;
    match (id, peers) {
        (None, None) => Ok(None),
        (Some(id), Some(peers)) => {
            let id: u64 = id
                .parse()
                .map_err(|_| format!("--raft-id '{id}' is not a number"))?;
            let peers = replica::parse_peers(&peers)?;
            if !peers.contains_key(&id) {
                return Err(format!("--raft-id {id} is not in --raft-peers"));
            }
            Ok(Some((id, peers)))
        }
        _ => Err("--raft-id and --raft-peers go together".into()),
    }
}

/// Whether this controller may schedule: always when standalone, only while
/// leading when replicated.
enum Role {
    Standalone,
    Replica(gnomon_consensus::SchedulerRaft),
}

impl Role {
    fn is_leader(&self) -> bool {
        match self {
            Role::Standalone => true,
            Role::Replica(raft) => replica::is_leader(raft),
        }
    }
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

    let env = |k: &str| std::env::var(k).ok();
    let fail = |e: String| -> ! {
        eprintln!("gnomon-ctl: {e}");
        std::process::exit(2);
    };
    let args = &args[1..];
    let dir = state_dir(args, env("GNOMON_STATE_DIR")).unwrap_or_else(|e| fail(e));
    let listen = flag(args, "listen", env("GNOMON_LISTEN"))
        .unwrap_or_else(|e| fail(e))
        .unwrap_or_else(|| "0.0.0.0:7631".into()); // neighbors Slurm's 6817
    let raft = raft_config(args, env("GNOMON_RAFT_ID"), env("GNOMON_RAFT_PEERS"))
        .unwrap_or_else(|e| fail(e));
    let plugins = plugins::load(args, env).unwrap_or_else(|e| fail(e));
    for line in &plugins.summary {
        println!("gnomon-ctl: {line}");
    }
    let broker = Arc::new(EvalBroker::new());
    let (fsm, role, _replica) = match raft {
        Some((id, peers)) => {
            let Some(d) = &dir else {
                fail("a replicated controller (--raft-id) needs --state-dir".into())
            };
            let listener = tokio::net::TcpListener::bind(&peers[&id])
                .await
                .unwrap_or_else(|e| fail(format!("raft listen on {}: {e}", peers[&id])));
            let r = replica::start(id, &peers, &d.join("raft"), listener, Arc::new(SystemClock))
                .await
                .unwrap_or_else(|e| {
                    eprintln!(
                        "gnomon-ctl: cannot start raft node {id} in {}: {e}",
                        d.display()
                    );
                    std::process::exit(1);
                });
            println!(
                "gnomon-ctl raft node {id} of {} (raft on {}), recovered index {}",
                peers.len(),
                peers[&id],
                r.fsm.index()
            );
            (r.fsm.clone(), Role::Replica(r.raft.clone()), Some(r))
        }
        None => {
            let fsm = match open_fsm(dir.as_deref(), Arc::new(SystemClock)) {
                Ok(fsm) => fsm,
                Err(e) => {
                    let d = dir.as_deref().unwrap_or(Path::new("?")).display();
                    eprintln!("gnomon-ctl: cannot recover state from {d}: {e}");
                    std::process::exit(1);
                }
            };
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
            (fsm, Role::Standalone, None)
        }
    };
    let role = Arc::new(role);
    let applier = Arc::new(PlanApplier::new());
    let policy = plugins.policy.clone();

    // Background scheduler loop: dequeue evals, plan on snapshots, commit.
    // Replicated, only the leader plans; evals reaching a follower are
    // dropped (the leader's backfill pass places every pending job), and a
    // controller that becomes leader re-evaluates everything pending.
    {
        let (fsm, broker, applier, policy, role) = (
            fsm.clone(),
            broker.clone(),
            applier.clone(),
            policy.clone(),
            role.clone(),
        );
        std::thread::spawn(move || {
            let mut leading = matches!(*role, Role::Standalone);
            loop {
                let now_leading = role.is_leader();
                if now_leading && !leading {
                    let n = requeue_pending(&fsm, &broker);
                    println!("gnomon-ctl: now leader; re-evaluating {n} pending jobs");
                } else if leading && !now_leading {
                    println!("gnomon-ctl: no longer leader; scheduling paused");
                }
                leading = now_leading;
                match broker.dequeue() {
                    Some(eval) if leading => {
                        // Extend overruns first: the FSM refuses a start on
                        // resources an overrunning job still holds.
                        gnomon_sched::extend_overruns(&fsm, policy.as_ref(), fsm.now());
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
                    Some(_) => {}
                    None => std::thread::sleep(std::time::Duration::from_millis(10)),
                }
            }
        });
    }
    // Periodic conservative-backfill pass: re-plans reservations (so reserved
    // jobs start when their time comes or earlier), and extends predicted
    // spans that overran. Safe alongside the broker workers because the
    // applier uses per-vertex optimistic concurrency. Leader only. Before
    // each pass the leader declares nodes lost that stopped heartbeating
    // (their jobs end, their unstarted work is re-planned by this pass).
    {
        let (fsm, applier, policy, role) =
            (fsm.clone(), applier.clone(), policy.clone(), role.clone());
        let timeout = node_timeout(
            env("GNOMON_NODE_TIMEOUT_SECS"),
            env("GNOMON_HEARTBEAT_SECS"),
        )
        .unwrap_or_else(|e| fail(e));
        println!("gnomon-ctl: nodes are lost after {timeout}s without a heartbeat");
        std::thread::spawn(move || {
            let mut leading_since: Option<u64> = None;
            loop {
                if role.is_leader() {
                    let now = SystemClock.now();
                    let since = *leading_since.get_or_insert(now);
                    let stale = fsm.snapshot().state().stale_nodes(now, timeout, since);
                    for (name, last_heartbeat) in stale {
                        let ev = Event::NodeLost {
                            name: name.clone(),
                            last_heartbeat,
                        };
                        // Rejected if the node heartbeated since the check.
                        if fsm.apply_checked(ev).accepted {
                            eprintln!("gnomon-ctl: node {name} lost (no heartbeat for {timeout}s)");
                        }
                    }
                    gnomon_sched::backfill_pass(
                        &fsm,
                        policy.as_ref(),
                        &applier,
                        Default::default(),
                    );
                } else {
                    leading_since = None;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        });
    }

    let addr = listen
        .parse()
        .unwrap_or_else(|e| fail(format!("--listen '{listen}': {e}")));
    println!("gnomon-ctl listening on {addr}");
    tonic::transport::Server::builder()
        .add_service(
            SchedulerSvc {
                fsm,
                broker,
                submit_policy: plugins.submit,
            }
            .server(),
        )
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

    #[test]
    fn raft_flags_need_each_other_and_a_listed_id() {
        let none = || None;
        assert_eq!(raft_config(&[], none(), none()), Ok(None));
        let ok = raft_config(
            &args(&["--raft-id", "2", "--raft-peers", "1=a:7,2=b:7"]),
            None,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!((ok.0, ok.1.len()), (2, 2));
        // Environment fallbacks, as for --state-dir.
        let env = raft_config(&[], Some("1".into()), Some("1=a:7".into())).unwrap();
        assert_eq!(env.map(|c| c.0), Some(1));
        for bad in [
            &["--raft-id", "1"][..],
            &["--raft-peers", "1=a:7"],
            &["--raft-id", "3", "--raft-peers", "1=a:7,2=b:7"],
            &["--raft-id", "x", "--raft-peers", "1=a:7"],
            &["--raft-id", "--raft-peers", "1=a:7"],
        ] {
            assert!(raft_config(&args(bad), None, None).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn node_timeout_defaults_to_three_heartbeats_and_at_least_30s() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(node_timeout(None, None), Ok(30));
        assert_eq!(node_timeout(None, s("2")), Ok(30));
        assert_eq!(node_timeout(None, s("20")), Ok(60));
        assert_eq!(node_timeout(s("45"), s("20")), Ok(45));
        assert_eq!(node_timeout(s(""), None), Ok(30));
        assert!(node_timeout(s("0"), None).is_err());
        assert!(node_timeout(s("soon"), None).is_err());
    }

    #[test]
    fn listen_flag_takes_a_value_or_the_environment() {
        assert_eq!(
            flag(
                &args(&["--listen", "127.0.0.1:9"]),
                "listen",
                Some("x".into())
            ),
            Ok(Some("127.0.0.1:9".into()))
        );
        assert_eq!(
            flag(&[], "listen", Some("e:1".into())),
            Ok(Some("e:1".into()))
        );
        assert!(flag(&args(&["--listen"]), "listen", None).is_err());
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
