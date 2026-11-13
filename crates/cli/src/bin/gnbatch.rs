//! sbatch-compatible submit: parses #GNOMON / #SBATCH directives.
use clap::Parser;
use gnomon_api::pb::*;
mod common {
    include!("../common.rs");
}

/// Command-line flags override `#SBATCH`/`#GNOMON` directives, which
/// override the defaults (1 node, 1 CPU, 1024 MB, 0 GPUs, 600 s) — sbatch's
/// precedence. CPUs, memory and GPUs are per node; `-N` asks for that many
/// nodes allocated together (gang).
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
    /// Nodes allocated together, all starting at once (`min-max` uses min).
    #[arg(long, short = 'N', value_parser = parse_nodes)]
    nodes: Option<u32>,
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

/// Job shape from `#SBATCH`/`#GNOMON` directives, starting from the defaults.
#[derive(Debug, PartialEq)]
struct Directives {
    name: Option<String>,
    cores: u32,
    mem: u64,
    gpus: u32,
    time: u64,
    part: String,
    nodes: u32,
}

impl Default for Directives {
    fn default() -> Self {
        Directives {
            name: None,
            cores: 1,
            mem: 1024,
            gpus: 0,
            time: 600,
            part: "normal".into(),
            nodes: 1,
        }
    }
}

/// `-N/--nodes` value: a count or Slurm's `min-max` (we allocate `min`).
fn parse_nodes(v: &str) -> Result<u32, String> {
    let min = v.split('-').next().unwrap_or(v);
    match min.trim().parse::<u32>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(format!("invalid node count '{v}'")),
    }
}

/// Directive options that take a value (`--k=v`, `--k v`, `-N v`, `-Nv`).
fn takes_value(k: &str) -> bool {
    matches!(
        k,
        "--cpus-per-task"
            | "--mem"
            | "--gpus"
            | "--gres"
            | "--time"
            | "--job-name"
            | "--partition"
            | "--nodes"
            | "-N"
    )
}

impl Directives {
    /// Apply the directive lines of a batch script; unknown options are
    /// accepted and ignored like sbatch's unsupported ones.
    fn parse(body: &str) -> Self {
        let mut d = Directives::default();
        for line in body.lines() {
            let Some(l) = line
                .strip_prefix("#SBATCH")
                .or_else(|| line.strip_prefix("#GNOMON"))
            else {
                continue;
            };
            let toks: Vec<&str> = l.split_whitespace().collect();
            let mut i = 0;
            while i < toks.len() {
                let tok = toks[i];
                i += 1;
                let (k, inline) = match tok.split_once('=') {
                    Some((k, v)) => (k, Some(v)),
                    None if tok.starts_with("-N") && tok.len() > 2 => ("-N", Some(&tok[2..])),
                    None => (tok, None),
                };
                let v = match inline {
                    Some(v) => v,
                    None if takes_value(k) && i < toks.len() && !toks[i].starts_with('-') => {
                        i += 1;
                        toks[i - 1]
                    }
                    None => "",
                };
                d.apply(k, v);
            }
        }
        d
    }

    fn apply(&mut self, k: &str, v: &str) {
        match k {
            "--cpus-per-task" => self.cores = v.parse().unwrap_or(self.cores),
            "--mem" => self.mem = parse_mem_mb(v).unwrap_or(self.mem),
            "--gpus" => self.gpus = v.parse().unwrap_or(self.gpus),
            "--gres" => {
                if let Some(n) = v.strip_prefix("gpu:").and_then(|n| n.rsplit(':').next()) {
                    self.gpus = n.parse().unwrap_or(self.gpus)
                }
            }
            "--time" => self.time = parse_slurm_time(v).unwrap_or(self.time),
            "--job-name" => self.name = Some(v.to_string()),
            "--partition" => self.part = v.to_string(),
            "--nodes" | "-N" => self.nodes = parse_nodes(v).unwrap_or(self.nodes),
            _ => {} // other directives (output, ntasks...) are accepted and ignored
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    // Directives first, then explicit flags win.
    let d = if a.script.is_empty() {
        Directives::default()
    } else {
        std::fs::read_to_string(&a.script)
            .map(|body| Directives::parse(&body))
            .unwrap_or_default()
    };
    let Directives {
        name,
        cores,
        mem,
        gpus,
        time,
        part,
        nodes,
    } = d;
    let name = a.name.or(name).unwrap_or_else(|| "job".into());
    let (cores, mem, gpus, time, part, nodes) = (
        a.cpus.unwrap_or(cores),
        a.mem_mb.unwrap_or(mem),
        a.gpus.unwrap_or(gpus),
        a.time.unwrap_or(time),
        a.partition.unwrap_or(part),
        a.nodes.unwrap_or(nodes),
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
            nodes,
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
    fn nodes_directive_forms() {
        for body in [
            "#SBATCH -N 4",
            "#SBATCH -N4",
            "#SBATCH --nodes=4",
            "#SBATCH --nodes 4",
            "#GNOMON --nodes=4-8",
        ] {
            assert_eq!(Directives::parse(body).nodes, 4, "{body}");
        }
        assert_eq!(Directives::parse("#!/bin/sh\necho hi").nodes, 1);
        // Invalid counts keep the default.
        assert_eq!(Directives::parse("#SBATCH -N 0").nodes, 1);
        assert_eq!(Directives::parse("#SBATCH --nodes=x").nodes, 1);
    }

    #[test]
    fn directives_combine_per_node_shape() {
        let d = Directives::parse(
            "#!/bin/bash\n#SBATCH --job-name=mpi -N 2 --cpus-per-task=8\n\
             #SBATCH --mem=4G --time=1:00:00 --ntasks-per-node=8\n#SBATCH --partition gpu\n\
             srun ./app\n#SBATCH --nodes=3",
        );
        assert_eq!(
            d,
            Directives {
                name: Some("mpi".into()),
                cores: 8,
                mem: 4096,
                gpus: 0,
                time: 3600,
                part: "gpu".into(),
                nodes: 3,
            }
        );
    }

    #[test]
    fn nodes_flag_parses_count_and_range() {
        assert_eq!(parse_nodes("2"), Ok(2));
        assert_eq!(parse_nodes("2-4"), Ok(2));
        assert!(parse_nodes("0").is_err());
        let a = Args::try_parse_from(["gnbatch", "-N", "3", "job.sh"]).unwrap();
        assert_eq!(a.nodes, Some(3));
        let a = Args::try_parse_from(["gnbatch", "--nodes=2-5", "job.sh"]).unwrap();
        assert_eq!(a.nodes, Some(2));
    }
}
