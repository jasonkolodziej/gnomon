//! cgroup v2 job confinement (report section 4.6, "cgroup confinement").
//!
//! Each job gets its own cgroup `<root>/gnomon/job_<id>` with
//! `cpu.max` = cores x period, `memory.max` = the requested memory and
//! `memory.swap.max` = 0, the slurmd `proctrack/cgroup` + `task/cgroup`
//! equivalent. Every path is derived from an injected root so the unit tests
//! drive the same code against a temporary directory tree that mimics
//! cgroupfs; nothing here needs root or a real cgroup2 mount to be tested.
//!
//! `<root>` must be the cgroup2 mount point or a cgroup delegated to the
//! agent's user (e.g. a systemd `Delegate=yes` unit) that holds no processes
//! itself: cgroup v2's "no internal processes" rule forbids enabling
//! controllers for children of a cgroup that has member processes.

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

/// Default cgroup2 mount point.
pub const DEFAULT_ROOT: &str = "/sys/fs/cgroup";
/// CFS bandwidth period written to `cpu.max`, in microseconds (kernel default).
pub const CPU_PERIOD_US: u64 = 100_000;
/// Controllers every job cgroup needs.
const CONTROLLERS: [&str; 2] = ["cpu", "memory"];

/// Why cgroup confinement is not in use; logged once at agent start-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// `GNOMON_CGROUP=off`.
    Disabled,
    /// No `cgroup.controllers` under the root: not a cgroup2 hierarchy
    /// (e.g. a cgroup v1 host).
    NotCgroup2(PathBuf),
    /// A required controller is not offered by the root cgroup.
    MissingController(&'static str),
    /// The root is not writable by the agent (not delegated to this user).
    NotWritable(String),
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unavailable::Disabled => write!(f, "disabled by GNOMON_CGROUP=off"),
            Unavailable::NotCgroup2(p) => {
                write!(f, "{} is not a cgroup2 hierarchy", p.display())
            }
            Unavailable::MissingController(c) => write!(f, "controller '{c}' not available"),
            Unavailable::NotWritable(e) => write!(f, "cgroup root not writable: {e}"),
        }
    }
}

/// Handle on the agent's `gnomon` cgroup subtree.
#[derive(Debug, Clone)]
pub struct Cgroups {
    root: PathBuf,
}

impl Cgroups {
    /// Detect cgroup2 support from the environment: `GNOMON_CGROUP=off`
    /// disables confinement, `GNOMON_CGROUP_ROOT` overrides the root.
    pub fn from_env() -> Result<Cgroups, Unavailable> {
        Cgroups::from_vars(|k| std::env::var(k).ok())
    }

    /// [`Cgroups::from_env`] with an injected variable lookup.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Cgroups, Unavailable> {
        if var("GNOMON_CGROUP").is_some_and(|v| v.eq_ignore_ascii_case("off")) {
            return Err(Unavailable::Disabled);
        }
        let root = var("GNOMON_CGROUP_ROOT").unwrap_or_else(|| DEFAULT_ROOT.into());
        Cgroups::detect(Path::new(&root))
    }

    /// Check that `root` is a writable cgroup2 hierarchy offering the cpu and
    /// memory controllers, then create `<root>/gnomon` with both controllers
    /// enabled for its children.
    pub fn detect(root: &Path) -> Result<Cgroups, Unavailable> {
        let offered = fs::read_to_string(root.join("cgroup.controllers"))
            .map_err(|_| Unavailable::NotCgroup2(root.to_path_buf()))?;
        if let Some(c) = CONTROLLERS
            .into_iter()
            .find(|c| !offered.split_whitespace().any(|o| o == *c))
        {
            return Err(Unavailable::MissingController(c));
        }
        let cg = Cgroups {
            root: root.to_path_buf(),
        };
        cg.prepare()
            .map_err(|e| Unavailable::NotWritable(e.to_string()))?;
        Ok(cg)
    }

    fn prepare(&self) -> io::Result<()> {
        enable_controllers(&self.root)?;
        let base = self.base();
        match fs::create_dir(&base) {
            Err(e) if e.kind() != ErrorKind::AlreadyExists => return Err(e),
            _ => {}
        }
        enable_controllers(&base)
    }

    /// `<root>/gnomon`, the parent of every job cgroup.
    pub fn base(&self) -> PathBuf {
        self.root.join("gnomon")
    }

    /// `<root>/gnomon/job_<id>`.
    pub fn job_path(&self, job_id: u64) -> PathBuf {
        self.base().join(format!("job_{job_id}"))
    }

    /// Create the job's cgroup and write its limits.
    pub fn create(&self, job_id: u64, limits: &Limits) -> io::Result<JobCgroup> {
        let path = self.job_path(job_id);
        match fs::create_dir(&path) {
            Err(e) if e.kind() != ErrorKind::AlreadyExists => return Err(e),
            _ => {}
        }
        let cg = JobCgroup { path };
        if let Err(e) = cg.write_limits(limits) {
            let _ = cg.remove();
            return Err(e);
        }
        Ok(cg)
    }
}

/// Append `+cpu +memory` to `dir/cgroup.subtree_control`. The kernel treats
/// the write as a delta, so controllers already enabled are untouched.
fn enable_controllers(dir: &Path) -> io::Result<()> {
    let ctl = CONTROLLERS.map(|c| format!("+{c}")).join(" ");
    fs::write(dir.join("cgroup.subtree_control"), ctl)
}

/// The resource limits applied to one job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub cores: u32,
    pub mem_mb: u64,
}

impl Limits {
    /// `cpu.max` contents: `<quota> <period>`, or unlimited for 0 cores.
    pub fn cpu_max(&self) -> String {
        if self.cores == 0 {
            format!("max {CPU_PERIOD_US}")
        } else {
            format!("{} {CPU_PERIOD_US}", u64::from(self.cores) * CPU_PERIOD_US)
        }
    }

    /// `memory.max` contents in bytes, `None` when no memory was requested.
    pub fn memory_max(&self) -> Option<String> {
        (self.mem_mb > 0).then(|| (self.mem_mb * 1024 * 1024).to_string())
    }
}

/// One job's cgroup directory.
#[derive(Debug)]
pub struct JobCgroup {
    path: PathBuf,
}

impl JobCgroup {
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn write_limits(&self, l: &Limits) -> io::Result<()> {
        fs::write(self.path.join("cpu.max"), l.cpu_max())?;
        if let Some(m) = l.memory_max() {
            fs::write(self.path.join("memory.max"), m)?;
        }
        // No swap: a job over its memory limit is OOM-killed, not paged out.
        fs::write(self.path.join("memory.swap.max"), "0")
    }

    /// Open `cgroup.procs` for the child to join itself through (see
    /// `launch::spawn`).
    pub fn procs_file(&self) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.path.join("cgroup.procs"))
    }

    /// Kill every process in the cgroup through `cgroup.kill` (Linux 5.14+).
    /// Returns `false` when the file does not exist so the caller can fall
    /// back to killing the direct child.
    pub fn kill(&self) -> bool {
        let f = self.path.join("cgroup.kill");
        f.exists() && fs::write(f, "1").is_ok()
    }

    /// Read peak memory and CPU time; missing files yield `None` fields.
    pub fn usage(&self) -> Usage {
        Usage {
            peak_mem_bytes: fs::read_to_string(self.path.join("memory.peak"))
                .ok()
                .and_then(|s| s.trim().parse().ok()),
            cpu_usec: fs::read_to_string(self.path.join("cpu.stat"))
                .ok()
                .and_then(|s| parse_cpu_usage(&s)),
        }
    }

    /// Remove the cgroup. Stragglers that outlived the job's main process
    /// are killed first (via `cgroup.kill`) so the directory can go.
    pub fn remove(&self) -> io::Result<()> {
        let mut last = Ok(());
        for attempt in 0..5 {
            match fs::remove_dir(&self.path) {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
                // Only a plain directory (the test fixture, never cgroupfs,
                // whose control files vanish with rmdir) reports this: drop
                // the files we wrote and retry.
                Err(e) if e.kind() == ErrorKind::DirectoryNotEmpty => {
                    remove_plain_files(&self.path)?;
                    last = Err(e);
                }
                // EBUSY: processes still live in it.
                Err(e) => {
                    self.kill();
                    std::thread::sleep(std::time::Duration::from_millis(20 << attempt));
                    last = Err(e);
                }
            }
        }
        last
    }
}

fn remove_plain_files(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Resource usage of a finished job, read from its cgroup.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// `memory.peak` (Linux 5.19+).
    pub peak_mem_bytes: Option<u64>,
    /// `usage_usec` from `cpu.stat`.
    pub cpu_usec: Option<u64>,
}

impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cpu = self
            .cpu_usec
            .map(|u| format!("{:.3}s", u as f64 / 1e6))
            .unwrap_or_else(|| "n/a".into());
        let mem = self
            .peak_mem_bytes
            .map(|b| format!("{} MiB", b / (1024 * 1024)))
            .unwrap_or_else(|| "n/a".into());
        write!(f, "cpu {cpu}, peak mem {mem}")
    }
}

/// Extract `usage_usec` from `cpu.stat` contents.
pub fn parse_cpu_usage(stat: &str) -> Option<u64> {
    stat.lines()
        .find_map(|l| l.strip_prefix("usage_usec "))
        .and_then(|v| v.trim().parse().ok())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A unique temporary directory, removed on drop.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new() -> TempDir {
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!(
                "gnomon-agent-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A temp tree that looks like a cgroup2 root offering `controllers`.
    pub fn fake_root(controllers: &str) -> TempDir {
        let t = TempDir::new();
        fs::write(t.0.join("cgroup.controllers"), controllers).unwrap();
        t
    }

    fn read(p: &Path) -> String {
        fs::read_to_string(p).unwrap()
    }

    #[test]
    fn detect_enables_controllers_and_creates_base() {
        let t = fake_root("cpuset cpu io memory pids\n");
        let cg = Cgroups::detect(&t.0).unwrap();
        assert_eq!(cg.base(), t.0.join("gnomon"));
        assert_eq!(read(&t.0.join("cgroup.subtree_control")), "+cpu +memory");
        assert_eq!(
            read(&t.0.join("gnomon/cgroup.subtree_control")),
            "+cpu +memory"
        );
        // Idempotent across agent restarts.
        assert!(Cgroups::detect(&t.0).is_ok());
    }

    #[test]
    fn env_can_disable_or_redirect_root() {
        let t = fake_root("cpu memory");
        let root = t.0.display().to_string();
        let vars = |off: bool| {
            let root = root.clone();
            move |k: &str| match k {
                "GNOMON_CGROUP" if off => Some("OFF".to_string()),
                "GNOMON_CGROUP_ROOT" => Some(root.clone()),
                _ => None,
            }
        };
        assert_eq!(
            Cgroups::from_vars(vars(true)).unwrap_err(),
            Unavailable::Disabled
        );
        assert_eq!(
            Cgroups::from_vars(vars(false)).unwrap().base(),
            t.0.join("gnomon")
        );
    }

    #[test]
    fn detect_rejects_non_cgroup2_root() {
        let t = TempDir::new();
        assert_eq!(
            Cgroups::detect(&t.0).unwrap_err(),
            Unavailable::NotCgroup2(t.0.clone())
        );
    }

    #[test]
    fn detect_requires_cpu_and_memory() {
        let t = fake_root("cpu pids");
        assert_eq!(
            Cgroups::detect(&t.0).unwrap_err(),
            Unavailable::MissingController("memory")
        );
        let t = fake_root("memory");
        assert_eq!(
            Cgroups::detect(&t.0).unwrap_err(),
            Unavailable::MissingController("cpu")
        );
    }

    #[test]
    fn detect_reports_unwritable_root() {
        let t = fake_root("cpu memory");
        // A file where the subtree should go makes creating it fail.
        fs::write(t.0.join("gnomon"), "").unwrap();
        assert!(matches!(
            Cgroups::detect(&t.0),
            Err(Unavailable::NotWritable(_))
        ));
    }

    #[test]
    fn limits_render_cgroup_values() {
        let l = Limits {
            cores: 4,
            mem_mb: 2048,
        };
        assert_eq!(l.cpu_max(), "400000 100000");
        assert_eq!(l.memory_max().as_deref(), Some("2147483648"));
        let l = Limits {
            cores: 0,
            mem_mb: 0,
        };
        assert_eq!(l.cpu_max(), "max 100000");
        assert_eq!(l.memory_max(), None);
    }

    #[test]
    fn create_writes_limit_files_under_job_path() {
        let t = fake_root("cpu memory");
        let cg = Cgroups::detect(&t.0).unwrap();
        let job = cg
            .create(
                7,
                &Limits {
                    cores: 2,
                    mem_mb: 512,
                },
            )
            .unwrap();
        assert_eq!(job.path(), t.0.join("gnomon/job_7"));
        assert_eq!(read(&job.path().join("cpu.max")), "200000 100000");
        assert_eq!(read(&job.path().join("memory.max")), "536870912");
        assert_eq!(read(&job.path().join("memory.swap.max")), "0");
    }

    #[test]
    fn zero_memory_leaves_memory_max_untouched() {
        let t = fake_root("cpu memory");
        let cg = Cgroups::detect(&t.0).unwrap();
        let job = cg
            .create(
                8,
                &Limits {
                    cores: 1,
                    mem_mb: 0,
                },
            )
            .unwrap();
        assert!(!job.path().join("memory.max").exists());
        assert_eq!(read(&job.path().join("memory.swap.max")), "0");
    }

    #[test]
    fn remove_deletes_job_cgroup() {
        let t = fake_root("cpu memory");
        let cg = Cgroups::detect(&t.0).unwrap();
        let job = cg
            .create(
                9,
                &Limits {
                    cores: 1,
                    mem_mb: 64,
                },
            )
            .unwrap();
        job.remove().unwrap();
        assert!(!cg.job_path(9).exists());
        assert!(cg.base().exists());
        // Removing twice is fine.
        job.remove().unwrap();
    }

    #[test]
    fn kill_uses_cgroup_kill_when_present() {
        let t = fake_root("cpu memory");
        let cg = Cgroups::detect(&t.0).unwrap();
        let job = cg
            .create(
                10,
                &Limits {
                    cores: 1,
                    mem_mb: 0,
                },
            )
            .unwrap();
        assert!(!job.kill(), "no cgroup.kill on older kernels");
        fs::write(job.path().join("cgroup.kill"), "").unwrap();
        assert!(job.kill());
        assert_eq!(read(&job.path().join("cgroup.kill")), "1");
    }

    #[test]
    fn usage_reads_peak_and_cpu_stat() {
        let t = fake_root("cpu memory");
        let cg = Cgroups::detect(&t.0).unwrap();
        let job = cg
            .create(
                11,
                &Limits {
                    cores: 1,
                    mem_mb: 0,
                },
            )
            .unwrap();
        assert_eq!(job.usage(), Usage::default());
        fs::write(job.path().join("memory.peak"), "10485760\n").unwrap();
        fs::write(
            job.path().join("cpu.stat"),
            "usage_usec 1500000\nuser_usec 1000000\nsystem_usec 500000\n",
        )
        .unwrap();
        let u = job.usage();
        assert_eq!(u.peak_mem_bytes, Some(10 * 1024 * 1024));
        assert_eq!(u.cpu_usec, Some(1_500_000));
        assert_eq!(u.to_string(), "cpu 1.500s, peak mem 10 MiB");
    }

    #[test]
    fn parse_cpu_usage_ignores_other_keys() {
        assert_eq!(parse_cpu_usage("user_usec 5\nusage_usec 42\n"), Some(42));
        assert_eq!(parse_cpu_usage("nr_periods 0\n"), None);
    }
}
