//! Rootless container launches (report section 4.6, container support).
//!
//! A job with a container image runs under `docker run --rm` (or Podman,
//! whose CLI is compatible). Resource limits are delegated to the runtime's
//! `--cpus`/`--memory` flags, which create the container's own cgroup, so
//! the agent does not also confine the client process. Everything here is a
//! pure argv builder so the exact command lines are unit-tested.

use crate::launch::LaunchSpec;
use std::path::Path;

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
}

impl ContainerRuntime {
    /// `GNOMON_CONTAINER_RUNTIME` (default `docker`).
    pub fn from_env() -> ContainerRuntime {
        ContainerRuntime::from_bin(
            &std::env::var("GNOMON_CONTAINER_RUNTIME").unwrap_or_else(|_| "docker".into()),
        )
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
        }
    }
}

/// `gnomon-job-<id>`: a stable name so the walltime kill can address the
/// container itself, not just the `docker run` client.
pub fn container_name(job_id: u64) -> String {
    format!("gnomon-job-{job_id}")
}

/// Full argv (binary first) of the `run` command for `spec` in `image`.
/// `user` is the agent's `(uid, gid)`; `None` leaves the image's default user.
pub fn run_argv(
    rt: &ContainerRuntime,
    spec: &LaunchSpec,
    image: &str,
    user: Option<(u32, u32)>,
) -> Vec<String> {
    let mut v: Vec<String> = vec![rt.bin.clone(), "run".into(), "--rm".into()];
    let mut push = |a: &str, b: String| {
        v.push(a.into());
        v.push(b);
    };
    push("--name", container_name(spec.job_id));
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
    if let Some((uid, gid)) = user {
        if rt.kind == RuntimeKind::Podman {
            // Map the agent's uid to itself inside the user namespace so files
            // written to the bind mount belong to the job's user.
            v.push("--userns=keep-id".into());
        }
        v.push("--user".into());
        v.push(format!("{uid}:{gid}"));
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

/// Argv that kills the job's container (walltime enforcement).
pub fn kill_argv(rt: &ContainerRuntime, job_id: u64) -> Vec<String> {
    vec![rt.bin.clone(), "kill".into(), container_name(job_id)]
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
        assert!(has_pair(&v, "--name", "gnomon-job-42"));
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
        assert_eq!(kill_argv(&rt, 7), ["podman", "kill", "gnomon-job-7"]);
    }
}
