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
    /// Container image to run the job in (rootless Docker/Podman on the node).
    #[arg(long, alias = "container-image", value_name = "IMAGE")]
    container: Option<String>,
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

/// Job settings gathered from `#SBATCH`/`#GNOMON` directives, starting from
/// gnbatch's defaults.
#[derive(Debug, Clone, PartialEq)]
struct Directives {
    name: Option<String>,
    cores: u32,
    mem_mb: u64,
    gpus: u32,
    time: u64,
    partition: String,
    container: Option<String>,
}

impl Default for Directives {
    fn default() -> Self {
        Directives {
            name: None,
            cores: 1,
            mem_mb: 1024,
            gpus: 0,
            time: 600,
            partition: "normal".into(),
            container: None,
        }
    }
}

/// Parse the directive lines of a batch script body. Unknown directives
/// (output, nodes, ntasks...) are accepted and ignored.
fn parse_directives(body: &str) -> Directives {
    let mut d = Directives::default();
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
                "--cpus-per-task" => d.cores = v.parse().unwrap_or(d.cores),
                "--mem" => d.mem_mb = parse_mem_mb(v).unwrap_or(d.mem_mb),
                "--gpus" => d.gpus = v.parse().unwrap_or(d.gpus),
                "--gres" => {
                    if let Some(n) = v.strip_prefix("gpu:").and_then(|n| n.rsplit(':').next()) {
                        d.gpus = n.parse().unwrap_or(d.gpus)
                    }
                }
                "--time" => d.time = parse_slurm_time(v).unwrap_or(d.time),
                "--job-name" => d.name = Some(v.to_string()),
                "--partition" => d.partition = v.to_string(),
                // `--container-image` is the pyxis/enroot spelling Slurm users know.
                "--container" | "--container-image" if !v.is_empty() => {
                    d.container = Some(v.to_string())
                }
                _ => {}
            }
        }
    }
    d
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    // Directives first, then explicit flags win.
    let d = if a.script.is_empty() {
        Directives::default()
    } else {
        std::fs::read_to_string(&a.script)
            .map(|body| parse_directives(&body))
            .unwrap_or_default()
    };
    let Directives {
        name,
        cores,
        mem_mb: mem,
        gpus,
        time,
        partition: part,
        container,
    } = d;
    let container = a.container.or(container).unwrap_or_default();
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
            container,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_without_directives() {
        assert_eq!(
            parse_directives("#!/bin/sh\necho hi\n"),
            Directives::default()
        );
    }

    #[test]
    fn parses_resource_directives() {
        let d = parse_directives(
            "#!/bin/sh\n#SBATCH --cpus-per-task=4 --mem=2G\n#GNOMON --gres=gpu:a100:2\n\
             #SBATCH --time=1:30:00\n#SBATCH --job-name=train --partition=gpu\n",
        );
        assert_eq!(d.cores, 4);
        assert_eq!(d.mem_mb, 2048);
        assert_eq!(d.gpus, 2);
        assert_eq!(d.time, 5400);
        assert_eq!(d.name.as_deref(), Some("train"));
        assert_eq!(d.partition, "gpu");
        assert_eq!(d.container, None);
    }

    #[test]
    fn parses_container_directive_both_spellings() {
        let d = parse_directives("#SBATCH --container=alpine:3.20\n");
        assert_eq!(d.container.as_deref(), Some("alpine:3.20"));
        let d = parse_directives("#SBATCH --container-image=nvcr.io/nvidia/pytorch:24.05-py3\n");
        assert_eq!(
            d.container.as_deref(),
            Some("nvcr.io/nvidia/pytorch:24.05-py3")
        );
        assert_eq!(parse_directives("#SBATCH --container=\n").container, None);
    }

    #[test]
    fn container_flag_accepts_both_spellings() {
        let a = Args::try_parse_from(["gnbatch", "--container", "alpine", "job.sh"]).unwrap();
        assert_eq!(a.container.as_deref(), Some("alpine"));
        let a = Args::try_parse_from(["gnbatch", "--container-image=alpine", "job.sh"]).unwrap();
        assert_eq!(a.container.as_deref(), Some("alpine"));
        assert_eq!(a.script, "job.sh");
    }

    #[test]
    fn slurm_time_and_mem_units() {
        assert_eq!(parse_slurm_time("10"), Some(600));
        assert_eq!(parse_slurm_time("1-02:00:00"), Some(93600));
        assert_eq!(parse_mem_mb("512K"), Some(1));
        assert_eq!(parse_mem_mb("3T"), Some(3 * 1024 * 1024));
        assert_eq!(parse_mem_mb("x"), None);
    }
}
