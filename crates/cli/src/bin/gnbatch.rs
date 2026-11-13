//! sbatch-compatible submit: parses #GNOMON / #SBATCH directives.
use clap::Parser;
use gnomon_api::pb::*;
mod common {
    include!("../common.rs");
}

/// Command-line flags override `#SBATCH`/`#GNOMON` directives, which
/// override the defaults (1 CPU, 1024 MB, 0 GPUs, 600 s) — sbatch's precedence.
#[derive(Parser)]
struct Args {
    #[arg(long, short = 'J')]
    name: Option<String>,
    #[arg(long, short = 'c')]
    cpus: Option<u32>,
    #[arg(long)]
    mem_mb: Option<u64>,
    #[arg(long)]
    gpus: Option<u32>,
    /// Seconds.
    #[arg(long, short = 't')]
    time: Option<u64>,
    #[arg(long, short = 'p')]
    partition: Option<String>,
    #[arg(default_value = "")]
    script: String,
    /// Arguments passed to the script (like `sbatch script arg1 arg2`).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

/// POSIX single-quote a word for `sh -c`.
fn sh_quote(w: &str) -> String {
    format!("'{}'", w.replace('\'', "'\\''"))
}

/// `--mem` value with Slurm units (K/M/G/T, default MB) -> MB.
fn parse_mem_mb(v: &str) -> Option<u64> {
    let v = v.trim();
    let (num, unit) = v.split_at(v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len()));
    let n: u64 = num.parse().ok()?;
    Some(match unit.to_ascii_uppercase().as_str() {
        "" | "M" | "MB" => n,
        "K" | "KB" => n.div_ceil(1024),
        "G" | "GB" => n * 1024,
        "T" | "TB" => n * 1024 * 1024,
        _ => return None,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    // Directives first, then explicit flags win.
    let (mut name, mut cores, mut mem, mut gpus, mut time, mut part) = (
        None::<String>,
        1u32,
        1024u64,
        0u32,
        600u64,
        "normal".to_string(),
    );
    if !a.script.is_empty() {
        if let Ok(body) = std::fs::read_to_string(&a.script) {
            for line in body.lines() {
                let Some(l) = line
                    .strip_prefix("#SBATCH")
                    .or_else(|| line.strip_prefix("#GNOMON"))
                else {
                    continue;
                };
                for tok in l.split_whitespace() {
                    let (k, v) = tok.split_once('=').unwrap_or((tok, ""));
                    match k {
                        "--cpus-per-task" => cores = v.parse().unwrap_or(cores),
                        "--mem" => mem = parse_mem_mb(v).unwrap_or(mem),
                        "--gpus" => gpus = v.parse().unwrap_or(gpus),
                        "--gres" => {
                            if let Some(n) =
                                v.strip_prefix("gpu:").and_then(|n| n.rsplit(':').next())
                            {
                                gpus = n.parse().unwrap_or(gpus)
                            }
                        }
                        "--time" => time = parse_slurm_time(v).unwrap_or(time),
                        "--job-name" => name = Some(v.to_string()),
                        "--partition" => part = v.to_string(),
                        _ => {} // other directives (output, nodes, ntasks...) are accepted and ignored
                    }
                }
            }
        }
    }
    let name = a.name.or(name).unwrap_or_else(|| "job".into());
    let (cores, mem, gpus, time, part) = (
        a.cpus.unwrap_or(cores),
        a.mem_mb.unwrap_or(mem),
        a.gpus.unwrap_or(gpus),
        a.time.unwrap_or(time),
        a.partition.unwrap_or(part),
    );
    // Like sbatch, resolve the script relative to the submit directory so the
    // agent can run it from anywhere (a bare `job.sh` is not on the agent's PATH).
    let command = if a.script.is_empty() {
        name.clone()
    } else {
        let path = std::fs::canonicalize(&a.script)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| a.script.clone());
        std::iter::once(path)
            .chain(a.args.iter().cloned())
            .map(|w| sh_quote(&w))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut client = common::connect().await?;
    let reply = client
        .submit_job(SubmitJobRequest {
            name,
            user: std::env::var("USER").unwrap_or_else(|_| "unknown".into()),
            partition: part,
            request: Some(Resources {
                cores,
                mem_mb: mem,
                gpus,
            }),
            walltime_secs: time.max(1),
            priority: 1000,
            command,
            nodes: 1,
        })
        .await?
        .into_inner();
    println!("Submitted batch job {}", reply.job_id); // sbatch's exact output format
    Ok(())
}

/// Slurm time formats: minutes, MM:SS, HH:MM:SS, D-HH:MM.
fn parse_slurm_time(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split(['-', ':']).collect();
    let v: Vec<u64> = parts.iter().filter_map(|p| p.parse().ok()).collect();
    Some(match v.as_slice() {
        [m] => m * 60,
        [mm, ss] => mm * 60 + ss,
        [h, m, s2] => h * 3600 + m * 60 + s2,
        [d, h, m, s2] => d * 86400 + h * 3600 + m * 60 + s2,
        _ => return None,
    })
}
