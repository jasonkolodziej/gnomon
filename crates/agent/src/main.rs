//! gnomon-agent: per-node agent (slurmd equivalent), phase 4.
//!
//! Registers with the controller over gRPC, heartbeats every 10 s, polls
//! for work assignments, and launches jobs through [`gnomon_agent::Launcher`]:
//! confined in a per-job cgroup v2 when available, or inside a rootless
//! container when the job names an image. Every poll also carries the
//! controller's kill list ([`gnomon_agent::kill`]): local ranks on it are
//! stopped, and listed jobs with no local rank are reported as killed (CDI device injection is the next
//! iteration; resource detection is real: /proc + nvidia-smi/rocm-smi probing).
//!
//! On SIGTERM or SIGINT the agent stops polling, stops its running ranks
//! through the same kill path and waits (bounded) until each has reported,
//! so a stopped agent leaves no job Running and no container unsupervised.

use gnomon_agent::barrier::{self, Release, BARRIER_POLL, BARRIER_TIMEOUT_EXIT};
use gnomon_agent::controllers::{parse_addrs, Controllers};
use gnomon_agent::kill::LocalRanks;
use gnomon_agent::{LaunchSpec, Launcher};
use gnomon_api::pb::*;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

fn detect() -> Resources {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let mem_mb = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .map(str::to_owned)
        })
        .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|kb| kb / 1024)
        .unwrap_or(0);
    // GPU probing: nvidia-smi or rocm-smi presence (CDI spec files are the
    // production path — /etc/cdi, /var/run/cdi).
    let gpus = count_gpus();
    // Overrides let one machine emulate smaller nodes (test kit, CI).
    let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u64>().ok());
    Resources {
        cores: env("GNOMON_CORES").map(|v| v as u32).unwrap_or(cores),
        mem_mb: env("GNOMON_MEM_MB").unwrap_or(mem_mb),
        gpus: env("GNOMON_GPUS").map(|v| v as u32).unwrap_or(gpus),
    }
}

fn count_gpus() -> u32 {
    for tool in ["nvidia-smi", "rocm-smi"] {
        if let Ok(out) = std::process::Command::new(tool).arg("-L").output() {
            if out.status.success() {
                let n = String::from_utf8_lossy(&out.stdout).lines().count() as u32;
                if n > 0 {
                    return n;
                }
            }
        }
    }
    0
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let name = std::env::var("GNOMON_NODE").unwrap_or_else(|_| hostname::fallback());
    // One controller, or every replica of a replicated controller group.
    let addrs = parse_addrs(
        &std::env::var("GNOMON_ADDR").unwrap_or_else(|_| "http://127.0.0.1:7631".into()),
    );
    anyhow::ensure!(!addrs.is_empty(), "GNOMON_ADDR lists no controller");
    let cap = detect();
    // Probed once: cgroup availability is logged here, not per job.
    let launcher = Arc::new(Launcher::from_env());
    // Jobs run (and containers bind-mount) the agent's working directory
    // unless GNOMON_WORKDIR names another, e.g. a shared filesystem.
    let workdir = std::env::var_os("GNOMON_WORKDIR")
        .map(std::path::PathBuf::from)
        .map_or_else(std::env::current_dir, Ok)?;
    println!(
        "gnomon-agent '{name}': {} cores, {} MiB, {} GPUs",
        cap.cores, cap.mem_mb, cap.gpus
    );

    let ctl = Arc::new(Controllers::new(addrs));
    let ranks = Arc::new(LocalRanks::default());
    let mut interval = Duration::from_secs(2);
    // (Re-)register on every new controller connection: harmless when the
    // controller already knows the node, and it restores the node on a
    // standalone controller that restarted without a state dir.
    let mut registered: Option<u64> = None;
    let mut first = true;
    let mut stop = shutdown_signal();
    loop {
        // Shutdown is noticed here, between polls, never in the middle of a
        // FetchWork: an assignment the controller handed out is always tracked.
        if !first {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = stop.changed() => {}
            }
        }
        if *stop.borrow() {
            break;
        }
        first = false;
        let (gen, mut client, via) = match ctl.client().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("no controller reachable ({e}); retrying");
                continue;
            }
        };
        if registered != Some(gen) {
            let reg = client
                .register_node(RegisterNodeRequest {
                    name: name.clone(),
                    capacity: Some(cap),
                })
                .await;
            match reg {
                Ok(r) => {
                    let secs = r.into_inner().heartbeat_interval_secs.max(2);
                    if registered.is_none() {
                        println!("registered; heartbeating every {secs}s");
                    } else {
                        println!("re-registered with {via}");
                    }
                    interval = Duration::from_secs(secs);
                    registered = Some(gen);
                    tokio::time::sleep(interval).await;
                }
                Err(e) => {
                    let next = ctl.failed(gen).await;
                    eprintln!("register failed ({e}); trying {next}");
                    continue;
                }
            }
        }
        let polled = async {
            client
                .heartbeat(HeartbeatRequest {
                    name: name.clone(),
                    allocated: None,
                })
                .await?;
            client
                .fetch_work(FetchWorkRequest { node: name.clone() })
                .await
                .map(|r| r.into_inner())
        };
        let work = match polled.await {
            Ok(w) => w,
            Err(e) => {
                let next = ctl.failed(gen).await;
                eprintln!("controller error ({e}); failing over to {next}");
                continue;
            }
        };
        let plan = ranks.kill(&work.kill);
        for job_id in plan.signalled {
            println!("job {job_id}: killed by the controller");
        }
        for job_id in plan.report {
            // Never started here (or already gone): report it so the job ends.
            let (ctl, node) = (Arc::clone(&ctl), name.clone());
            tokio::spawn(async move {
                report(&ctl, &node, job_id, gnomon_agent::launch::KILLED_EXIT).await
            });
        }
        for a in work.assignments {
            let (ctl, node) = (Arc::clone(&ctl), name.clone());
            let spec = LaunchSpec::from_assignment(&a, &name, workdir.clone());
            let launcher = Arc::clone(&launcher);
            let (ranks, cancel) = (Arc::clone(&ranks), ranks.track(spec.job_id));
            tokio::spawn(async move {
                let job_id = spec.job_id;
                // Gang ranks wait at the release barrier until every rank has
                // been fetched, so they start together. A kill while waiting
                // ends the wait (the job then reports as killed, unstarted).
                let release = if spec.nodelist.len() > 1 {
                    let probe = || {
                        let ctl = Arc::clone(&ctl);
                        let cancel = Arc::clone(&cancel);
                        async move {
                            if cancel.load(Ordering::Acquire) {
                                return Ok((false, false));
                            }
                            let (gen, mut c, _) = ctl.client().await?;
                            match c.gang_ready(GangReadyRequest { job_id }).await {
                                Ok(r) => {
                                    let r = r.into_inner();
                                    Ok((r.ready, r.active))
                                }
                                Err(e) => {
                                    ctl.failed(gen).await;
                                    Err(e.to_string())
                                }
                            }
                        }
                    };
                    let limit = barrier::wait_limit(spec.walltime_secs);
                    barrier::await_release(probe, BARRIER_POLL, limit).await
                } else {
                    Release::Go
                };
                let out = match release {
                    // Run the job like slurmd: job environment, limits, walltime
                    // enforced (killed at the limit, reported as exit 124), and
                    // stopped at once if the controller kills it (exit 143).
                    Release::Go => {
                        let cancel = Arc::clone(&cancel);
                        tokio::task::spawn_blocking(move || launcher.run_until(&spec, &cancel))
                            .await
                            .ok()
                    }
                    Release::Gone if cancel.load(Ordering::Acquire) => {
                        println!("job {job_id}: killed at the gang barrier; rank not started");
                        Some(gnomon_agent::launch::killed())
                    }
                    Release::Gone => {
                        println!("job {job_id}: no longer running; rank not started");
                        ranks.untrack(job_id);
                        return;
                    }
                    Release::TimedOut => {
                        eprintln!("job {job_id}: gang peers never all arrived; failing this rank");
                        Some(gnomon_agent::Outcome {
                            exit_code: BARRIER_TIMEOUT_EXIT,
                            timed_out: false,
                            killed: false,
                            usage: None,
                        })
                    }
                };
                let code = out.map_or(-1, |o| o.exit_code);
                match out.and_then(|o| o.usage) {
                    Some(u) => println!("job {job_id} finished: exit {code} ({u})"),
                    None => println!("job {job_id} finished: exit {code}"),
                }
                report(&ctl, &node, job_id, code).await;
                ranks.untrack(job_id);
            });
        }
    }
    drain(&ranks).await;
    Ok(())
}

/// Seconds to wait at shutdown for stopped ranks to report.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(60);

/// Becomes `true` on SIGTERM (systemd stop, `docker stop`) or SIGINT.
fn shutdown_signal() -> tokio::sync::watch::Receiver<bool> {
    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("cannot watch SIGTERM ({e}); stop the agent with SIGINT");
                let _ = tokio::signal::ctrl_c().await;
                let _ = tx.send(true);
                return;
            }
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        let _ = tx.send(true);
    });
    rx
}

/// Agent shutdown: stop every local rank exactly as the kill channel does
/// (cgroup.kill, or docker/podman kill for container jobs), then wait for
/// each rank's task to report its exit (143) so no job is left Running in
/// the controller's books. Bounded by [`SHUTDOWN_GRACE`].
async fn drain(ranks: &LocalRanks) {
    let stopping = ranks.kill_all();
    if stopping.is_empty() {
        println!("shutting down: no running jobs");
        return;
    }
    println!("shutting down: stopping jobs {stopping:?} and reporting them");
    let deadline = tokio::time::Instant::now() + SHUTDOWN_GRACE;
    while !ranks.is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !ranks.is_empty() {
        eprintln!("shutting down: some jobs did not report within {SHUTDOWN_GRACE:?}");
    }
}

/// Report a rank's exit, retrying (and failing over between controllers)
/// until one accepts it: a lost report would leave the job running in the
/// controller's books until its walltime. Repeated reports are harmless
/// (the controller ignores a rank's second report).
async fn report(ctl: &Controllers, node: &str, job_id: u64, exit_code: i32) {
    let req = ReportCompletionRequest {
        node: node.to_string(),
        job_id,
        exit_code,
    };
    let mut delay = Duration::from_millis(500);
    for _ in 0..REPORT_ATTEMPTS {
        match ctl.client().await {
            Ok((gen, mut c, _)) => match c.report_completion(req.clone()).await {
                Ok(_) => return,
                Err(e) => {
                    let next = ctl.failed(gen).await;
                    eprintln!("job {job_id}: report failed ({e}); retrying via {next}");
                }
            },
            Err(e) => eprintln!("job {job_id}: no controller for the report ({e})"),
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
    eprintln!("job {job_id}: gave up reporting exit {exit_code}");
}

/// About an hour of retries at the 30 s cap.
const REPORT_ATTEMPTS: usize = 128;

mod hostname {
    pub fn fallback() -> String {
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "node".into())
    }
}
