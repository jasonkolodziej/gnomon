//! Job launch (report section 4.6): the slurmd equivalent of spawning a
//! batch step. A job runs as `sh -c <command>` with the `GNOMON_*`
//! environment, either confined in its own cgroup v2 ([`crate::cgroup`]) or
//! inside a rootless container ([`crate::container`]). The walltime is
//! enforced by killing the job at the limit, reported as exit 124 like
//! `timeout(1)`. A kill from the controller (a gang peer failed, or the job
//! was cancelled while running) stops it the same way, reported as exit 143
//! (128 + SIGTERM, as a shell reports a terminated child).

use crate::cgroup::{Cgroups, JobCgroup, Limits, Usage};
use crate::container::{self, ContainerRuntime, ContainerUser};
use crate::identity::{self, JobIdentity};
use gnomon_api::pb::fetch_work_reply::Assignment;
use std::io::{self, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Exit code reported when the walltime kills a job (as `timeout(1)`).
pub const TIMEOUT_EXIT: i32 = 124;
/// Exit code reported for a rank stopped through the controller's kill
/// channel, including one that was never started.
pub const KILLED_EXIT: i32 = 143;

/// After a successful tree/container kill, how long to wait for the direct
/// child (the job shell or the `docker run` client) to exit before SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(30);
/// Bound on one `docker kill` / `docker rm -f` invocation.
const RUNTIME_CMD_TIMEOUT: Duration = Duration::from_secs(15);

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
    /// Submitting user; a root agent runs the job as this local account.
    pub user: String,
    /// Planned start shared by every rank (Unix seconds). Ranks are launched
    /// on each agent's next poll, so they start within one poll interval of
    /// each other; the job sees this common reference as `GNOMON_JOB_START`.
    pub start: u64,
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
            user: a.user.clone(),
            start: a.start,
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
            ("GNOMON_JOB_START", self.start.to_string()),
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
    /// Stopped through the kill channel ([`Launcher::run_until`]).
    pub killed: bool,
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
    /// The agent runs as root: every job is run as its submitting user (see
    /// [`crate::identity`]); otherwise jobs run as the agent.
    pub privileged: bool,
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
            privileged: identity::is_privileged(),
            poll: Duration::from_millis(200),
        }
    }

    /// Run the job to completion (blocking) and return its exit status.
    pub fn run(&self, spec: &LaunchSpec) -> Outcome {
        self.run_until(spec, &AtomicBool::new(false))
    }

    /// Like [`Launcher::run`], but stop the job (its whole cgroup or
    /// container, as at the walltime) as soon as `cancel` is set; the
    /// outcome is then [`KILLED_EXIT`]. A job cancelled before it starts is
    /// not started.
    pub fn run_until(&self, spec: &LaunchSpec, cancel: &AtomicBool) -> Outcome {
        if cancel.load(Ordering::Acquire) {
            return killed();
        }
        let who = match self.job_identity(spec) {
            Ok(who) => who,
            Err(e) => {
                eprintln!(
                    "job {} rank {}: {e}; not running it",
                    spec.job_id, spec.rank
                );
                return failed();
            }
        };
        match &spec.container {
            Some(image) => self.run_container(spec, image, who.as_ref(), cancel),
            None => self.run_host(spec, who, cancel),
        }
    }

    /// The account the job runs as: the submitting user when the agent is
    /// root (an unknown user or uid 0 is refused), else the agent (`None`).
    pub fn job_identity(&self, spec: &LaunchSpec) -> Result<Option<JobIdentity>, String> {
        if self.privileged {
            identity::resolve(&spec.user).map(Some)
        } else {
            Ok(None)
        }
    }

    fn run_host(
        &self,
        spec: &LaunchSpec,
        who: Option<JobIdentity>,
        cancel: &AtomicBool,
    ) -> Outcome {
        // Confinement was available at start-up, so a per-job setup failure
        // fails the job rather than running it without the limits the
        // scheduler reserved. Unconfined runs happen only when cgroups were
        // unavailable or disabled for the whole node.
        let cg = match &self.cgroups {
            None => None,
            Some(c) => match c.create(spec.job_id, spec.rank, &spec.limits()) {
                Ok(j) => Some(j),
                Err(e) => {
                    eprintln!(
                        "job {} rank {}: cgroup setup failed ({e}); not running it unconfined",
                        spec.job_id, spec.rank
                    );
                    return failed();
                }
            },
        };
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(&spec.command)
            .envs(spec.env())
            .current_dir(&spec.workdir);
        let child = spawn_job(cmd, cg.as_ref(), who);
        let end = match child {
            Ok(child) => {
                let pid = child.id();
                wait_with_walltime(child, spec.walltime_secs, self.poll, cancel, || {
                    cg.as_ref().is_some_and(JobCgroup::kill) || kill_process_group(pid)
                })
            }
            Err(e) => {
                eprintln!("job {}: spawn failed: {e}", spec.job_id);
                End::Exited(-1)
            }
        };
        let usage = cg.map(|j| {
            let u = j.usage();
            if let Err(e) = j.remove() {
                eprintln!("job {}: removing {}: {e}", spec.job_id, j.path().display());
            }
            u
        });
        end.outcome(usage)
    }

    fn run_container(
        &self,
        spec: &LaunchSpec,
        image: &str,
        who: Option<&JobIdentity>,
        cancel: &AtomicBool,
    ) -> Outcome {
        let user = match who {
            Some(id) => ContainerUser::Job {
                uid: id.uid,
                gid: id.gid,
            },
            None => ContainerUser::Agent(self.user),
        };
        let argv = container::run_argv_as(&self.runtime, spec, image, user);
        let child = Command::new(&argv[0]).args(&argv[1..]).spawn();
        let end = match child {
            Ok(child) => wait_with_walltime(child, spec.walltime_secs, self.poll, cancel, || {
                stop_container(&self.runtime, spec.job_id, spec.rank)
            }),
            Err(e) => {
                eprintln!("job {}: {} failed to start: {e}", spec.job_id, argv[0]);
                End::Exited(-1)
            }
        };
        end.outcome(None)
    }
}

/// Stop the job's container at the walltime: killing only the `docker run`
/// client would leave the job running under the daemon. Each runtime call is
/// bounded and its exit status checked; `kill` is retried, then `rm -f` is the
/// fallback. Returns whether the container is confirmed stopped (the caller
/// then waits a bounded grace period for the client to exit).
fn stop_container(rt: &ContainerRuntime, job_id: u64, rank: u32) -> bool {
    let attempts = [
        container::kill_argv(rt, job_id, rank),
        container::kill_argv(rt, job_id, rank),
        container::rm_argv(rt, job_id, rank),
    ];
    for argv in &attempts {
        match container::run_bounded(argv, RUNTIME_CMD_TIMEOUT) {
            Ok(st) if st.success() => return true,
            Ok(st) => eprintln!(
                "job {job_id} rank {rank}: `{}` exited {st}",
                argv[1..].join(" ")
            ),
            Err(e) => eprintln!("job {job_id} rank {rank}: `{}`: {e}", argv[1..].join(" ")),
        }
    }
    eprintln!(
        "job {job_id} rank {rank}: could NOT confirm the container stopped; it may still be \
         running under the {} daemon",
        rt.bin
    );
    false
}

/// A job that never ran.
fn failed() -> Outcome {
    End::Exited(-1).outcome(None)
}

/// A rank stopped (or never started) through the kill channel.
pub fn killed() -> Outcome {
    End::Killed.outcome(None)
}

/// How a launched job ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Exited(i32),
    TimedOut,
    Killed,
}

impl End {
    fn outcome(self, usage: Option<Usage>) -> Outcome {
        let exit_code = match self {
            End::Exited(c) => c,
            End::TimedOut => TIMEOUT_EXIT,
            End::Killed => KILLED_EXIT,
        };
        Outcome {
            exit_code,
            timed_out: self == End::TimedOut,
            killed: self == End::Killed,
            usage,
        }
    }
}

/// Spawn `cmd` with the child already inside the job cgroup (if any) and
/// running as the job's user (if the agent is root).
///
/// Race choice: the child joins the cgroup itself, in `pre_exec`, by writing
/// `0` ("the writing process") to `cgroup.procs` before it execs the job.
/// Writing the pid from the parent after `spawn` would leave a window in
/// which the job runs unconfined and anything it forks stays outside the
/// cgroup for good. The file is opened in the parent so the post-fork
/// closure performs a single `write(2)`: no allocation, async-signal-safe.
///
/// The cgroup is joined first, while the child still has the agent's
/// rights (through the fd the parent opened), then privileges are dropped:
/// the job cannot write its own cgroup's limits, which stay root-owned.
fn spawn_job(
    mut cmd: Command,
    cg: Option<&JobCgroup>,
    who: Option<JobIdentity>,
) -> io::Result<Child> {
    if let Some(cg) = cg {
        let procs = cg.procs_file()?;
        // SAFETY: the closure only issues write(2) on an fd owned by the
        // parent; it neither allocates nor takes locks, as required between
        // fork and exec.
        unsafe {
            cmd.pre_exec(move || (&procs).write_all(b"0"));
        }
    }
    if let Some(id) = who {
        // SAFETY: setgroups/setgid/setuid on data resolved by the parent.
        unsafe {
            cmd.pre_exec(move || identity::drop_privileges(&id));
        }
    }
    // Its own process group, so an unconfined job's whole tree (the shell
    // and everything it started) can be killed with one signal.
    cmd.process_group(0);
    cmd.spawn()
}

/// SIGKILL the process group led by `pid` (see [`spawn_job`]): the kill
/// path for jobs without a cgroup. Processes that left the group (`setsid`,
/// daemons) escape it; only cgroup confinement catches those.
fn kill_process_group(pid: u32) -> bool {
    let Ok(pgid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: kill(2) with a negative pid signals that process group.
    unsafe { libc::kill(-pgid, libc::SIGKILL) == 0 }
}

/// Wait for `child`, killing it at the walltime (0 = unlimited) or as soon
/// as `cancel` is set. `kill_tree` runs first; when it returns `false` the
/// direct child is SIGKILLed instead.
fn wait_with_walltime(
    mut child: Child,
    walltime_secs: u64,
    poll: Duration,
    cancel: &AtomicBool,
    kill_tree: impl FnOnce() -> bool,
) -> End {
    let limit = (walltime_secs > 0).then(|| Instant::now() + Duration::from_secs(walltime_secs));
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return End::Exited(st.code().unwrap_or(-1)),
            Ok(None) => {}
            Err(_) => return End::Exited(-1),
        }
        let why = if cancel.load(Ordering::Acquire) {
            Some(End::Killed)
        } else if limit.is_some_and(|l| Instant::now() >= l) {
            Some(End::TimedOut)
        } else {
            None
        };
        if let Some(why) = why {
            if kill_tree() {
                // The tree is dead; give the direct child a bounded grace
                // period to notice before SIGKILLing it too.
                let grace = Instant::now() + KILL_GRACE;
                while matches!(child.try_wait(), Ok(None)) && Instant::now() < grace {
                    std::thread::sleep(poll);
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            return why;
        }
        std::thread::sleep(poll);
    }
}

/// The agent's effective `(uid, gid)`.
pub fn current_user() -> Option<(u32, u32)> {
    // SAFETY: geteuid/getegid have no preconditions and cannot fail.
    Some(unsafe { (libc::geteuid(), libc::getegid()) })
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
            user: "nobody".into(),
        }
    }

    fn launcher(cgroups: Option<Cgroups>) -> Launcher {
        Launcher {
            cgroups,
            runtime: ContainerRuntime::from_bin("docker"),
            user: None,
            privileged: false,
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
        a.start = 1_794_613_465;
        let s = LaunchSpec::from_assignment(&a, "n2", PathBuf::from("/w"));
        assert_eq!(s.container.as_deref(), Some("alpine:3.20"));
        assert_eq!(env_of(&s, "GNOMON_NODELIST"), "n0,n1,n2");
        assert_eq!(env_of(&s, "GNOMON_NODE_RANK"), "2");
        assert_eq!(env_of(&s, "GNOMON_NNODES"), "3");
        assert_eq!(
            env_of(&s, "GNOMON_JOB_START"),
            "1794613465",
            "common reference"
        );
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
                0,
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
        assert!(!cg.job_path(5, 0).exists(), "job cgroup removed after exit");
    }

    #[test]
    fn cgroup_setup_failure_fails_the_job_instead_of_running_unconfined() {
        let t = fake_root("cpu memory");
        let cg = Cgroups::detect(&t.0).unwrap();
        // A plain file where the job cgroup directory must go: create fails.
        std::fs::write(cg.job_path(5, 0), "").unwrap();
        let marker = t.0.join("ran");
        let mut a = assignment();
        a.command = format!("touch {}", marker.display());
        let spec = LaunchSpec::from_assignment(&a, "n0", std::env::temp_dir());
        let out = launcher(Some(cg)).run(&spec);
        assert_eq!(out.exit_code, -1);
        assert!(!marker.exists(), "the job must not run unconfined");
    }

    #[test]
    fn root_agent_refuses_unknown_and_root_job_users() {
        let marker = std::env::temp_dir().join(format!("gnomon-refused-{}", std::process::id()));
        let mut l = launcher(None);
        l.privileged = true;
        for user in ["root", "gnomon-no-such-user-x9q", ""] {
            let mut a = assignment();
            a.user = user.into();
            a.command = format!("touch {}", marker.display());
            let spec = LaunchSpec::from_assignment(&a, "n0", std::env::temp_dir());
            assert_eq!(l.run(&spec).exit_code, -1, "user {user:?}");
        }
        assert!(!marker.exists(), "a refused job must not run");
    }

    #[test]
    fn jobs_run_as_the_submitting_user_when_the_agent_is_root() {
        let out = std::env::temp_dir().join(format!("gnomon-uid-{}", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let mut a = assignment();
        a.user = "nobody".into();
        a.command = format!("id -u > {}; id -g >> {}", out.display(), out.display());
        let spec = LaunchSpec::from_assignment(&a, "n0", std::env::temp_dir());
        let mut l = launcher(None);
        l.privileged = identity::is_privileged();
        assert_eq!(l.run(&spec).exit_code, 0);
        let ids: Vec<u32> = std::fs::read_to_string(&out)
            .unwrap()
            .lines()
            .map(|x| x.trim().parse().unwrap())
            .collect();
        let _ = std::fs::remove_file(&out);
        if l.privileged {
            // Root agent: the job dropped to `nobody`, uid and gid.
            let nobody = identity::resolve("nobody").unwrap();
            assert_eq!(ids, vec![nobody.uid, nobody.gid]);
        } else {
            // Unprivileged agent: it cannot switch users; the job runs as it.
            let me = current_user().unwrap();
            assert_eq!(ids, vec![me.0, me.1]);
        }
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
