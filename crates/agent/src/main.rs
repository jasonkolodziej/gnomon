//! gnomon-agent: per-node agent (slurmd equivalent), phase 4.
//!
//! Registers with the controller over gRPC, heartbeats every 10 s, polls
//! for work assignments, and launches job commands as child processes
//! (cgroup confinement and CDI device injection are the next iteration;
//! resource detection is real: /proc + nvidia-smi/rocm-smi probing).

use gnomon_api::pb::scheduler_client::SchedulerClient;
use gnomon_api::pb::*;
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
    let addr = std::env::var("GNOMON_ADDR").unwrap_or_else(|_| "http://127.0.0.1:7631".into());
    let cap = detect();
    println!(
        "gnomon-agent '{name}': {} cores, {} MiB, {} GPUs",
        cap.cores, cap.mem_mb, cap.gpus
    );

    let mut client = SchedulerClient::connect(addr).await?;
    let reg = client
        .register_node(RegisterNodeRequest {
            name: name.clone(),
            capacity: Some(cap),
        })
        .await?
        .into_inner();
    let interval = Duration::from_secs(reg.heartbeat_interval_secs.max(2));
    println!("registered; heartbeating every {}s", interval.as_secs());

    loop {
        tokio::time::sleep(interval).await;
        client
            .heartbeat(HeartbeatRequest {
                name: name.clone(),
                allocated: None,
            })
            .await?;
        let work = client
            .fetch_work(FetchWorkRequest { node: name.clone() })
            .await?
            .into_inner();
        for a in work.assignments {
            let (mut client, node) = (client.clone(), name.clone());
            tokio::spawn(async move {
                // Run the job like slurmd: job environment, walltime enforced
                // (SIGKILL at the limit, reported as exit 124 like timeout(1)).
                let req = a.request.unwrap_or_default();
                let code = tokio::task::spawn_blocking(move || {
                    let child = std::process::Command::new("sh")
                        .arg("-c")
                        .arg(&a.command)
                        .env("GNOMON_JOB_ID", a.job_id.to_string())
                        .env("GNOMON_JOB_NAME", &a.name)
                        .env("GNOMON_CPUS", req.cores.to_string())
                        .env("GNOMON_MEM_MB", req.mem_mb.to_string())
                        .env("GNOMON_GPUS", req.gpus.to_string())
                        .env("GNOMON_TIME_LIMIT", a.walltime_secs.to_string())
                        .spawn();
                    let mut child = match child {
                        Ok(c) => c,
                        Err(_) => return -1,
                    };
                    let limit = if a.walltime_secs > 0 {
                        Some(std::time::Instant::now() + Duration::from_secs(a.walltime_secs))
                    } else {
                        None
                    };
                    loop {
                        match child.try_wait() {
                            Ok(Some(st)) => return st.code().unwrap_or(-1),
                            Ok(None) => {}
                            Err(_) => return -1,
                        }
                        if limit.is_some_and(|l| std::time::Instant::now() >= l) {
                            let _ = child.kill();
                            let _ = child.wait();
                            return 124;
                        }
                        std::thread::sleep(Duration::from_millis(200));
                    }
                })
                .await
                .unwrap_or(-1);
                println!("job {} finished: exit {code}", a.job_id);
                let _ = client
                    .report_completion(ReportCompletionRequest {
                        node,
                        job_id: a.job_id,
                        exit_code: code,
                    })
                    .await;
            });
        }
    }
}

mod hostname {
    pub fn fallback() -> String {
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "node".into())
    }
}
