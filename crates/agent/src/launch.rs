//! Job launch (report section 4.6): the slurmd equivalent of spawning a
//! batch step. A job runs as `sh -c <command>` with the `GNOMON_*`
//! environment, either confined in its own cgroup v2 ([`crate::cgroup`]) or
//! inside a rootless container ([`crate::container`]). The walltime is
//! enforced by killing the job at the limit, reported as exit 124 like
//! `timeout(1)`.

use crate::cgroup::{Cgroups, JobCgroup, Limits, Usage};
use crate::container::{self, ContainerRuntime};
use gnomon_api::pb::fetch_work_reply::Assignment;
use std::io::{self, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Exit code reported when the walltime kills a job (as `timeout(1)`).
pub const TIMEOUT_EXIT: i32 = 124;

/// Everything the agent needs to start one job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub job_id: u64,
    pub name: String,
    pub command: String,
    pub cores: u32,
    pub mem_mb: u64,
    pub gpus: u32,
    /// 0 means unlimited.
    pub walltime_secs: u64,
    /// Container image; `None` runs directly on the node.
    pub container: Option<String>,
    /// Nodes of a gang allocation (this node alone for single-node jobs).
    pub nodelist: Vec<String>,
    /// This node's index in `nodelist`.
    pub rank: u32,
    /// Working directory (bind-mounted into containers).
    pub workdir: PathBuf,
}

impl LaunchSpec {
    /// Build from a FetchWork assignment. An empty nodelist (single-node job,
    /// or a controller that predates gang placement) becomes `[local_node]`.
    pub fn from_assignment(a: &Assignment, local_node: &str, workdir: PathBuf) -> LaunchSpec {
        let req = a.request.unwrap_or_default();
        let nodelist = if a.nodelist.is_empty() {
            vec![local_node.to_string()]
        } else {
            a.nodelist.clone()
        };
        LaunchSpec {
            job_id: a.job_id,
            name: a.name.clone(),
            command: a.command.clone(),
            cores: req.cores,
            mem_mb: req.mem_mb,
            gpus: req.gpus,
            walltime_secs: a.walltime_secs,
            container: Some(a.container.clone()).filter(|c| !c.is_empty()),
            nodelist,
            rank: a.rank,
            workdir,
        }
    }

    /// The job's `GNOMON_*` environment (Slurm's `SLURM_*` counterparts).
    pub fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("GNOMON_JOB_ID", self.job_id.to_string()),
            ("GNOMON_JOB_NAME", self.name.clone()),
            ("GNOMON_CPUS", self.cores.to_string()),
            ("GNOMON_MEM_MB", self.mem_mb.to_string()),
            ("GNOMON_GPUS", self.gpus.to_string()),
            ("GNOMON_TIME_LIMIT", self.walltime_secs.to_string()),
            ("GNOMON_NODELIST", self.nodelist.join(",")),
            ("GNOMON_NODE_RANK", self.rank.to_string()),
            ("GNOMON_NNODES", self.nodelist.len().max(1).to_string()),
        ]
    }

    fn limits(&self) -> Limits {
        Limits {
            cores: self.cores,
            mem_mb: self.mem_mb,
        }
    }
}

/// Result of one job run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub exit_code: i32,
    pub timed_out: bool,
    /// Measured from the job cgroup, when the job was confined.
    pub usage: Option<Usage>,
}

/// Node-wide launch configuration, fixed at agent start-up.
#[derive(Debug, Clone)]
pub struct Launcher {
    /// `None` runs jobs unconfined (cgroup v2 unavailable or disabled).
    pub cgroups: Option<Cgroups>,
    pub runtime: ContainerRuntime,
    /// The agent's `(uid, gid)`, passed to `--user` for containers.
    pub user: Option<(u32, u32)>,
    /// How often the walltime watchdog checks the child.
    pub poll: Duration,
}

impl Launcher {
    /// Configuration from the environment; logs once when cgroup
    /// confinement is unavailable.
    pub fn from_env() -> Launcher {
        let cgroups = match Cgroups::from_env() {
            Ok(cg) => {
                println!("cgroup v2 confinement under {}", cg.base().display());
                Some(cg)
            }
            Err(why) => {
                println!("cgroup v2 confinement unavailable ({why}); running jobs unconfined");
                None
            }
        };
        Launcher {
            cgroups,
            runtime: ContainerRuntime::from_env(),
            user: current_user(),
            poll: Duration::from_millis(200),
        }
    }

    /// Run the job to completion (blocking) and return its exit status.
    pub fn run(&self, spec: &LaunchSpec) -> Outcome {
        match &spec.container {
            Some(image) => self.run_container(spec, image),
            None => self.run_host(spec),
        }
    }

    fn run_host(&self, spec: &LaunchSpec) -> Outcome {
        let cg = self
            .cgroups
            .as_ref()
            .and_then(|c| match c.create(spec.job_id, &spec.limits()) {
                Ok(j) => Some(j),
                Err(e) => {
                    eprintln!("job {}: cgroup setup failed ({e}); unconfined", spec.job_id);
                    None
                }
            });
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(&spec.command)
            .envs(spec.env())
            .current_dir(&spec.workdir);
        let child = match &cg {
            Some(j) => spawn_in_cgroup(cmd, j),
            None => cmd.spawn(),
        };
        let (exit_code, timed_out) = match child {
            Ok(child) => wait_with_walltime(child, spec.walltime_secs, self.poll, || {
                cg.as_ref().is_some_and(JobCgroup::kill)
            }),
            Err(e) => {
                eprintln!("job {}: spawn failed: {e}", spec.job_id);
                (-1, false)
            }
        };
        let usage = cg.map(|j| {
            let u = j.usage();
            if let Err(e) = j.remove() {
                eprintln!("job {}: removing {}: {e}", spec.job_id, j.path().display());
            }
            u
        });
        Outcome {
            exit_code,
            timed_out,
            usage,
        }
    }

    fn run_container(&self, spec: &LaunchSpec, image: &str) -> Outcome {
        let argv = container::run_argv(&self.runtime, spec, image, self.user);
        let child = Command::new(&argv[0]).args(&argv[1..]).spawn();
        let (exit_code, timed_out) = match child {
            Ok(child) => wait_with_walltime(child, spec.walltime_secs, self.poll, || {
                // Stop the container itself: killing only the client would
                // leave the job running under the daemon.
                let kill = container::kill_argv(&self.runtime, spec.job_id);
                if let Err(e) = Command::new(&kill[0])
                    .args(&kill[1..])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                {
                    eprintln!("job {}: {} kill failed: {e}", spec.job_id, kill[0]);
                }
                false
            }),
            Err(e) => {
                eprintln!("job {}: {} failed to start: {e}", spec.job_id, argv[0]);
                (-1, false)
            }
        };
        Outcome {
            exit_code,
            timed_out,
            usage: None,
        }
    }
}

/// Spawn `cmd` with the child already inside the job cgroup.
///
/// Race choice: the child joins the cgroup itself, in `pre_exec`, by writing
/// `0` ("the writing process") to `cgroup.procs` before it execs the job.
/// Writing the pid from the parent after `spawn` would leave a window in
/// which the job runs unconfined and anything it forks stays outside the
/// cgroup for good. The file is opened in the parent so the post-fork
/// closure performs a single `write(2)`: no allocation, async-signal-safe.
fn spawn_in_cgroup(mut cmd: Command, cg: &JobCgroup) -> io::Result<Child> {
    let procs = cg.procs_file()?;
    // SAFETY: the closure only issues write(2) on an fd owned by the parent;
    // it neither allocates nor takes locks, as required between fork and exec.
    unsafe {
        cmd.pre_exec(move || (&procs).write_all(b"0"));
    }
    cmd.spawn()
}

/// Wait for `child`, killing it at the walltime (0 = unlimited). `kill_tree`
/// runs first at the limit; when it returns `false` the direct child is
/// SIGKILLed instead. Returns `(exit code, timed out)`.
fn wait_with_walltime(
    mut child: Child,
    walltime_secs: u64,
    poll: Duration,
    kill_tree: impl FnOnce() -> bool,
) -> (i32, bool) {
    let limit = (walltime_secs > 0).then(|| Instant::now() + Duration::from_secs(walltime_secs));
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return (st.code().unwrap_or(-1), false),
            Ok(None) => {}
            Err(_) => return (-1, false),
        }
        if limit.is_some_and(|l| Instant::now() >= l) {
            if !kill_tree() {
                let _ = child.kill();
            }
            let _ = child.wait();
            return (TIMEOUT_EXIT, true);
        }
        std::thread::sleep(poll);
    }
}

/// The agent's `(uid, gid)`, read from the owner of `/proc/self`.
pub fn current_user() -> Option<(u32, u32)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .ok()
        .map(|m| (m.uid(), m.gid()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cgroup::tests::fake_root;
    use gnomon_api::pb::Resources;

    fn assignment() -> Assignment {
        Assignment {
            job_id: 5,
            command: "echo hi".into(),
            request: Some(Resources {
                cores: 2,
                mem_mb: 256,
                gpus: 1,
            }),
            start: 0,
            walltime_secs: 30,
            name: "j".into(),
            container: String::new(),
            nodelist: vec![],
            rank: 0,
        }
    }

    fn launcher(cgroups: Option<Cgroups>) -> Launcher {
        Launcher {
            cgroups,
            runtime: ContainerRuntime::from_bin("docker"),
            user: None,
            poll: Duration::from_millis(10),
        }
    }

    fn env_of(spec: &LaunchSpec, k: &str) -> String {
        spec.env().into_iter().find(|e| e.0 == k).unwrap().1
    }

    #[test]
    fn spec_from_single_node_assignment_defaults_to_local_node() {
        let s = LaunchSpec::from_assignment(&assignment(), "n3", PathBuf::from("/w"));
        assert_eq!(s.job_id, 5);
        assert_eq!((s.cores, s.mem_mb, s.gpus), (2, 256, 1));
        assert_eq!(s.container, None);
        assert_eq!(s.nodelist, ["n3"]);
        assert_eq!(env_of(&s, "GNOMON_NODELIST"), "n3");
        assert_eq!(env_of(&s, "GNOMON_NODE_RANK"), "0");
        assert_eq!(env_of(&s, "GNOMON_NNODES"), "1");
        assert_eq!(env_of(&s, "GNOMON_TIME_LIMIT"), "30");
    }

    #[test]
    fn spec_from_gang_assignment_exports_nodelist_and_rank() {
        let mut a = assignment();
        a.container = "alpine:3.20".into();
        a.nodelist = vec!["n0".into(), "n1".into(), "n2".into()];
        a.rank = 2;
        let s = LaunchSpec::from_assignment(&a, "n2", PathBuf::from("/w"));
        assert_eq!(s.container.as_deref(), Some("alpine:3.20"));
        assert_eq!(env_of(&s, "GNOMON_NODELIST"), "n0,n1,n2");
        assert_eq!(env_of(&s, "GNOMON_NODE_RANK"), "2");
        assert_eq!(env_of(&s, "GNOMON_NNODES"), "3");
    }

    #[test]
    fn missing_request_is_zero_resources() {
        let mut a = assignment();
        a.request = None;
        let s = LaunchSpec::from_assignment(&a, "n0", PathBuf::from("/w"));
        assert_eq!((s.cores, s.mem_mb, s.gpus), (0, 0, 0));
    }

    #[test]
    fn confined_job_joins_cgroup_and_cleans_up() {
        let t = fake_root("cpu memory");
        let cg = Cgroups::detect(&t.0).unwrap();
        let mut a = assignment();
        a.command = "test \"$GNOMON_JOB_ID\" = 5 && exit 7".into();
        let spec = LaunchSpec::from_assignment(&a, "n0", std::env::temp_dir());
        // Observe the cgroup.procs write: keep a hard link to the file the
        // child writes into before the launcher removes the directory.
        let job = cg
            .create(
                5,
                &Limits {
                    cores: 2,
                    mem_mb: 256,
                },
            )
            .unwrap();
        drop(job.procs_file().unwrap());
        let witness = t.0.join("procs-witness");
        std::fs::hard_link(job.path().join("cgroup.procs"), &witness).unwrap();

        let out = launcher(Some(cg.clone())).run(&spec);
        assert_eq!(out.exit_code, 7);
        assert!(!out.timed_out);
        assert_eq!(out.usage, Some(Usage::default()));
        assert_eq!(std::fs::read_to_string(&witness).unwrap(), "0");
        assert!(!cg.job_path(5).exists(), "job cgroup removed after exit");
    }

    #[test]
    fn unconfined_job_has_no_usage() {
        let mut a = assignment();
        a.command = "exit 0".into();
        let spec = LaunchSpec::from_assignment(&a, "n0", std::env::temp_dir());
        let out = launcher(None).run(&spec);
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.usage, None);
    }

    #[test]
    fn missing_container_runtime_fails_cleanly() {
        let mut a = assignment();
        a.container = "alpine".into();
        let spec = LaunchSpec::from_assignment(&a, "n0", std::env::temp_dir());
        let mut l = launcher(None);
        l.runtime = ContainerRuntime::from_bin("/nonexistent/gnomon-docker");
        assert_eq!(l.run(&spec).exit_code, -1);
    }
}
