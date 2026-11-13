//! gnomon-agent: per-node agent (slurmd equivalent), phase 4.
//!
//! Registers with the controller over gRPC, heartbeats every 10 s, polls
//! for work assignments, and launches jobs through [`gnomon_agent::Launcher`]:
//! confined in a per-job cgroup v2 when available, or inside a rootless
//! container when the job names an image (CDI device injection is the next
//! iteration; resource detection is real: /proc + nvidia-smi/rocm-smi probing).

use gnomon_agent::{LaunchSpec, Launcher};
use gnomon_api::pb::scheduler_client::SchedulerClient;
use gnomon_api::pb::*;
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
    let addr = std::env::var("GNOMON_ADDR").unwrap_or_else(|_| "http://127.0.0.1:7631".into());
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
            let spec = LaunchSpec::from_assignment(&a, &name, workdir.clone());
            let launcher = Arc::clone(&launcher);
            tokio::spawn(async move {
                // Run the job like slurmd: job environment, limits, walltime
                // enforced (killed at the limit, reported as exit 124).
                let job_id = spec.job_id;
                let out = tokio::task::spawn_blocking(move || launcher.run(&spec))
                    .await
                    .ok();
                let code = out.map_or(-1, |o| o.exit_code);
                match out.and_then(|o| o.usage) {
                    Some(u) => println!("job {job_id} finished: exit {code} ({u})"),
                    None => println!("job {job_id} finished: exit {code}"),
                }
                let _ = client
                    .report_completion(ReportCompletionRequest {
                        node,
                        job_id,
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
