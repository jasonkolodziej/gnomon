//! Rootless container launches (report section 4.6, container support).
//!
//! A job with a container image runs under `docker run --rm` (or Podman,
//! whose CLI is compatible). Resource limits are delegated to the runtime's
//! `--cpus`/`--memory` flags, which create the container's own cgroup, so
//! the agent does not also confine the client process. Everything here is a
//! pure argv builder so the exact command lines are unit-tested.

use crate::launch::LaunchSpec;
use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Which CLI dialect the runtime binary speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeKind {
    Docker,
    Podman,
}

/// The container runtime binary and its dialect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerRuntime {
    pub bin: String,
    pub kind: RuntimeKind,
    /// Docker running rootless: container uid 0 is the agent's own host user,
    /// while any other `--user` maps to a subordinate uid that cannot write
    /// the agent's (bind-mounted) working directory.
    pub rootless: bool,
}

impl ContainerRuntime {
    /// `GNOMON_CONTAINER_RUNTIME` (default `docker`). For Docker, rootless
    /// mode comes from `GNOMON_CONTAINER_ROOTLESS` (`1`/`0`) or, if unset, is
    /// detected once from `docker info`.
    pub fn from_env() -> ContainerRuntime {
        let mut rt = ContainerRuntime::from_bin(
            &std::env::var("GNOMON_CONTAINER_RUNTIME").unwrap_or_else(|_| "docker".into()),
        );
        if rt.kind == RuntimeKind::Docker {
            rt.rootless = match std::env::var("GNOMON_CONTAINER_ROOTLESS").as_deref() {
                Ok("1") | Ok("true") => true,
                Ok("0") | Ok("false") => false,
                _ => detect_docker_rootless(&rt.bin),
            };
        }
        rt
    }

    /// The dialect is inferred from the binary's file name, so both `podman`
    /// and `/usr/bin/podman` select Podman.
    pub fn from_bin(bin: &str) -> ContainerRuntime {
        let file = Path::new(bin)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let kind = if file.contains("podman") {
            RuntimeKind::Podman
        } else {
            RuntimeKind::Docker
        };
        ContainerRuntime {
            bin: bin.to_string(),
            kind,
            rootless: false,
        }
    }
}

/// `docker info` lists `name=rootless` among the security options of a
/// rootless daemon. Unreachable or slow daemons count as rootful.
fn detect_docker_rootless(bin: &str) -> bool {
    let argv = [
        bin.to_string(),
        "info".into(),
        "--format".into(),
        "{{json .SecurityOptions}}".into(),
    ];
    let Ok(out) = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    out.status.success() && is_rootless_info(&String::from_utf8_lossy(&out.stdout))
}

/// Whether `docker info`'s security options describe a rootless daemon.
pub fn is_rootless_info(security_options: &str) -> bool {
    security_options.contains("name=rootless")
}

/// `gnomon-job-<id>-r<rank>`: a stable name so the walltime kill can address
/// the container itself, not just the `docker run` client. The rank keeps the
/// ranks of a gang distinct when several agents share one daemon.
pub fn container_name(job_id: u64, rank: u32) -> String {
    format!("gnomon-job-{job_id}-r{rank}")
}

/// Who a container job runs as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerUser {
    /// An unprivileged agent's own `(uid, gid)` (see [`container_user`]);
    /// `None` leaves the image's default user.
    Agent(Option<(u32, u32)>),
    /// A root agent runs the job as the submitting user's resolved ids,
    /// never as the agent (which would be host root under rootful Docker).
    Job { uid: u32, gid: u32 },
}

/// Full argv (binary first) of the `run` command for `spec` in `image`.
/// `user` is the agent's `(uid, gid)`; `None` leaves the image's default user.
pub fn run_argv(
    rt: &ContainerRuntime,
    spec: &LaunchSpec,
    image: &str,
    user: Option<(u32, u32)>,
) -> Vec<String> {
    run_argv_as(rt, spec, image, ContainerUser::Agent(user))
}

/// [`run_argv`] for an explicit [`ContainerUser`].
pub fn run_argv_as(
    rt: &ContainerRuntime,
    spec: &LaunchSpec,
    image: &str,
    who: ContainerUser,
) -> Vec<String> {
    let mut v: Vec<String> = vec![rt.bin.clone(), "run".into(), "--rm".into()];
    let mut push = |a: &str, b: String| {
        v.push(a.into());
        v.push(b);
    };
    push("--name", container_name(spec.job_id, spec.rank));
    if spec.cores > 0 {
        push("--cpus", spec.cores.to_string());
    }
    if spec.mem_mb > 0 {
        push("--memory", format!("{}m", spec.mem_mb));
    }
    // Podman exposes GPUs through CDI `--device nvidia.com/gpu=...` instead.
    if spec.gpus > 0 && rt.kind == RuntimeKind::Docker {
        push("--gpus", spec.gpus.to_string());
    }
    push("--network", "host".into());
    match who {
        ContainerUser::Agent(user) => {
            if let Some(id) = container_user(rt, user) {
                if rt.kind == RuntimeKind::Podman {
                    // Map the agent's uid to itself inside the user namespace so
                    // files written to the bind mount belong to the job's user.
                    v.push("--userns=keep-id".into());
                }
                v.push("--user".into());
                v.push(id);
            }
        }
        ContainerUser::Job { uid, gid } => {
            v.push("--user".into());
            v.push(format!("{uid}:{gid}"));
        }
    }
    let wd = spec.workdir.display().to_string();
    v.push("-v".into());
    v.push(format!("{wd}:{wd}"));
    v.push("-w".into());
    v.push(wd);
    for (k, val) in spec.env() {
        v.push("-e".into());
        v.push(format!("{k}={val}"));
    }
    v.extend([
        "--entrypoint".into(),
        "sh".into(),
        image.into(),
        "-c".into(),
        spec.command.clone(),
    ]);
    v
}

/// The `--user` the job runs as, given the agent's `(uid, gid)`.
///
/// * Podman (with `--userns=keep-id`) and rootful Docker: the agent's ids,
///   so the job owns what it writes and never runs as host root.
/// * Rootless Docker: `0:0`. The daemon runs as the agent's user, and
///   container root *is* that user on the host; any other uid maps to a
///   subordinate id that cannot write an agent-owned (0755) or private
///   (0700) working directory. This grants nothing beyond the agent's own
///   privileges.
pub fn container_user(rt: &ContainerRuntime, user: Option<(u32, u32)>) -> Option<String> {
    let (uid, gid) = user?;
    if rt.kind == RuntimeKind::Docker && rt.rootless {
        Some("0:0".into())
    } else {
        Some(format!("{uid}:{gid}"))
    }
}

/// Argv that kills the job's container (walltime enforcement).
pub fn kill_argv(rt: &ContainerRuntime, job_id: u64, rank: u32) -> Vec<String> {
    vec![rt.bin.clone(), "kill".into(), container_name(job_id, rank)]
}

/// Argv that force-removes the job's container: the fallback when `kill`
/// fails (e.g. the container is still being created).
pub fn rm_argv(rt: &ContainerRuntime, job_id: u64, rank: u32) -> Vec<String> {
    vec![
        rt.bin.clone(),
        "rm".into(),
        "-f".into(),
        container_name(job_id, rank),
    ]
}

/// Run `argv` to completion, but give up (killing it) after `timeout`: a
/// hung daemon must not stall the walltime watchdog.
pub fn run_bounded(argv: &[String], timeout: Duration) -> io::Result<ExitStatus> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(st) = child.try_wait()? {
            return Ok(st);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{} did not finish within {timeout:?}", argv[0]),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn spec(cores: u32, mem_mb: u64, gpus: u32) -> LaunchSpec {
        LaunchSpec {
            job_id: 42,
            name: "train".into(),
            command: "python train.py --epochs 3".into(),
            cores,
            mem_mb,
            gpus,
            walltime_secs: 60,
            container: Some("pytorch:24.05".into()),
            nodelist: vec!["n0".into()],
            rank: 0,
            user: String::new(),
            start: 0,
            workdir: PathBuf::from("/scratch/u"),
        }
    }

    fn has_pair(v: &[String], a: &str, b: &str) -> bool {
        v.windows(2).any(|w| w[0] == a && w[1] == b)
    }

    #[test]
    fn runtime_kind_from_binary_name() {
        assert_eq!(
            ContainerRuntime::from_bin("docker").kind,
            RuntimeKind::Docker
        );
        assert_eq!(
            ContainerRuntime::from_bin("/usr/bin/podman").kind,
            RuntimeKind::Podman
        );
    }

    #[test]
    fn docker_argv_is_complete_and_ordered() {
        let rt = ContainerRuntime::from_bin("docker");
        let v = run_argv(&rt, &spec(4, 2048, 2), "pytorch:24.05", Some((1000, 100)));
        assert_eq!(&v[..3], ["docker", "run", "--rm"]);
        assert!(has_pair(&v, "--name", "gnomon-job-42-r0"));
        assert!(has_pair(&v, "--cpus", "4"));
        assert!(has_pair(&v, "--memory", "2048m"));
        assert!(has_pair(&v, "--gpus", "2"));
        assert!(has_pair(&v, "--network", "host"));
        assert!(has_pair(&v, "--user", "1000:100"));
        assert!(!v.contains(&"--userns=keep-id".to_string()));
        assert!(has_pair(&v, "-v", "/scratch/u:/scratch/u"));
        assert!(has_pair(&v, "-w", "/scratch/u"));
        let n = v.len();
        assert_eq!(
            &v[n - 5..],
            [
                "--entrypoint",
                "sh",
                "pytorch:24.05",
                "-c",
                "python train.py --epochs 3"
            ]
        );
    }

    #[test]
    fn podman_omits_gpus_and_keeps_user_id() {
        let rt = ContainerRuntime::from_bin("podman");
        let v = run_argv(&rt, &spec(1, 512, 2), "img", Some((1000, 1000)));
        assert_eq!(v[0], "podman");
        assert!(!v.contains(&"--gpus".to_string()));
        assert!(v.contains(&"--userns=keep-id".to_string()));
        assert!(has_pair(&v, "--user", "1000:1000"));
    }

    #[test]
    fn zero_values_drop_their_flags() {
        let rt = ContainerRuntime::from_bin("docker");
        let v = run_argv(&rt, &spec(0, 0, 0), "img", None);
        for flag in ["--cpus", "--memory", "--gpus", "--user"] {
            assert!(!v.contains(&flag.to_string()), "{flag} should be absent");
        }
    }

    #[test]
    fn job_env_is_passed_with_e() {
        let rt = ContainerRuntime::from_bin("docker");
        let v = run_argv(&rt, &spec(2, 0, 0), "img", None);
        for kv in [
            "GNOMON_JOB_ID=42",
            "GNOMON_JOB_NAME=train",
            "GNOMON_CPUS=2",
            "GNOMON_MEM_MB=0",
            "GNOMON_NODELIST=n0",
            "GNOMON_NODE_RANK=0",
            "GNOMON_NNODES=1",
        ] {
            assert!(has_pair(&v, "-e", kv), "missing {kv}");
        }
    }

    #[test]
    fn kill_targets_the_named_container() {
        let rt = ContainerRuntime::from_bin("podman");
        assert_eq!(kill_argv(&rt, 7, 0), ["podman", "kill", "gnomon-job-7-r0"]);
        assert_eq!(
            rm_argv(&rt, 7, 2),
            ["podman", "rm", "-f", "gnomon-job-7-r2"]
        );
    }

    #[test]
    fn gang_ranks_get_distinct_container_names() {
        let rt = ContainerRuntime::from_bin("docker");
        let mut s = spec(1, 0, 0);
        let a = run_argv(&rt, &s, "img", None);
        s.rank = 1;
        let b = run_argv(&rt, &s, "img", None);
        assert!(has_pair(&a, "--name", "gnomon-job-42-r0"));
        assert!(has_pair(&b, "--name", "gnomon-job-42-r1"));
    }

    #[test]
    fn rootless_docker_runs_as_container_root_which_is_the_agent_user() {
        let mut rt = ContainerRuntime::from_bin("docker");
        rt.rootless = true;
        let v = run_argv(&rt, &spec(1, 0, 0), "img", Some((1000, 100)));
        assert!(has_pair(&v, "--user", "0:0"));
        // Rootful Docker and Podman keep the agent's own ids.
        assert_eq!(
            container_user(&ContainerRuntime::from_bin("docker"), Some((1000, 100))),
            Some("1000:100".into())
        );
        let mut podman = ContainerRuntime::from_bin("podman");
        podman.rootless = true; // ignored: keep-id already maps the agent's uid
        assert_eq!(
            container_user(&podman, Some((1000, 100))),
            Some("1000:100".into())
        );
        assert_eq!(container_user(&rt, None), None);
    }

    #[test]
    fn root_agent_runs_container_jobs_as_the_submitting_user() {
        for bin in ["docker", "podman"] {
            let rt = ContainerRuntime::from_bin(bin);
            let v = run_argv_as(
                &rt,
                &spec(1, 0, 0),
                "img",
                ContainerUser::Job {
                    uid: 1234,
                    gid: 1234,
                },
            );
            assert!(has_pair(&v, "--user", "1234:1234"), "{bin}");
            assert!(!v.contains(&"--userns=keep-id".to_string()), "{bin}");
            assert!(!v.iter().any(|a| a == "0:0"), "{bin}: never host root");
        }
    }

    #[test]
    fn rootless_detection_reads_docker_security_options() {
        assert!(is_rootless_info(
            r#"["name=seccomp,profile=builtin","name=rootless","name=cgroupns"]"#
        ));
        assert!(!is_rootless_info(r#"["name=seccomp,profile=builtin"]"#));
        assert!(!is_rootless_info(""));
    }

    #[test]
    fn bounded_run_reports_status_and_times_out() {
        let ok = run_bounded(&["true".into()], Duration::from_secs(5)).unwrap();
        assert!(ok.success());
        let bad = run_bounded(&["false".into()], Duration::from_secs(5)).unwrap();
        assert!(!bad.success());
        let t0 = Instant::now();
        let hung = run_bounded(&["sleep".into(), "30".into()], Duration::from_millis(200));
        assert_eq!(hung.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(t0.elapsed() < Duration::from_secs(5));
    }
}
