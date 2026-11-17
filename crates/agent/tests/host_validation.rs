//! Real-host validation: the launcher against the kernel's cgroup v2 and a
//! real container runtime, not the fake cgroupfs and fake `docker` of the
//! other tests. Everything here is `#[ignore]`d; run it on a cgroup v2 host
//! with Docker or Podman, as root (the agent's production setup):
//!
//! ```sh
//! testkit/validate-host.sh            # wraps the command below
//! sudo -E cargo test -p gnomon-agent --test host_validation -- --ignored --test-threads=1
//! ```
//!
//! `cgroup_*` tests need a writable cgroup v2 root with the `cpu` and
//! `memory` controllers (`GNOMON_CGROUP_ROOT`, default `/sys/fs/cgroup`);
//! `container_*` tests need a working runtime (`GNOMON_CONTAINER_RUNTIME`,
//! default `docker`) and pull `GNOMON_VALIDATE_IMAGE` (default
//! `alpine:3.20`). A missing prerequisite fails the test with the reason:
//! these tests exist to prove the host works, so they never skip silently.

use gnomon_agent::cgroup::Cgroups;
use gnomon_agent::container::{container_name, ContainerRuntime};
use gnomon_agent::launch::{KILLED_EXIT, TIMEOUT_EXIT};
use gnomon_agent::{LaunchSpec, Launcher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MIB: u64 = 1024 * 1024;

/// A fresh world-writable work directory (jobs may run as another user).
fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gnomon-validate-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o777)).unwrap();
    d
}

fn spec(job_id: u64, command: &str, cores: u32, mem_mb: u64, wall: u64, dir: &Path) -> LaunchSpec {
    LaunchSpec {
        job_id,
        name: "validate".into(),
        command: command.into(),
        cores,
        mem_mb,
        gpus: 0,
        walltime_secs: wall,
        container: None,
        nodelist: vec!["local".into()],
        rank: 0,
        user: String::new(),
        start: 0,
        workdir: dir.to_path_buf(),
    }
}

fn read(dir: &Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file))
        .unwrap_or_else(|e| panic!("{file}: {e}"))
        .trim()
        .to_string()
}

/// Whether a process with `needle` in its command line is alive.
fn alive(needle: &str) -> bool {
    std::fs::read_dir("/proc").unwrap().flatten().any(|e| {
        std::fs::read(e.path().join("cmdline"))
            .map(|c| {
                String::from_utf8_lossy(&c)
                    .replace('\0', " ")
                    .contains(needle)
            })
            .unwrap_or(false)
    })
}

// ---------------------------------------------------------------- cgroup v2

/// A launcher confined by the host's real cgroup v2 hierarchy, running jobs
/// as the agent (no identity switch: the tests check confinement).
fn confined() -> Launcher {
    let cgroups = Cgroups::from_env()
        .unwrap_or_else(|why| panic!("cgroup v2 confinement is not usable on this host: {why}"));
    Launcher {
        cgroups: Some(cgroups),
        privileged: false,
        poll: Duration::from_millis(50),
        ..Launcher::from_env()
    }
}

#[test]
#[ignore = "real host: needs cgroup v2 (run testkit/validate-host.sh)"]
fn cgroup_job_starts_inside_its_cgroup_with_the_requested_limits() {
    let dir = workdir("cg-limits");
    // The job reads its own cgroup from the inside: it must already be
    // confined when its first instruction runs.
    let cmd = r#"cg=$(sed -n 's/^0:://p' /proc/self/cgroup)
        echo "$cg" > cgroup
        cat "/sys/fs/cgroup$cg/cpu.max" > cpu.max
        cat "/sys/fs/cgroup$cg/memory.max" > memory.max
        cat "/sys/fs/cgroup$cg/memory.swap.max" > swap.max"#;
    let out = confined().run(&spec(9101, cmd, 2, 256, 30, &dir));
    assert_eq!(out.exit_code, 0);
    assert!(
        read(&dir, "cgroup").ends_with("/gnomon/job_9101_r0"),
        "job ran in {}",
        read(&dir, "cgroup")
    );
    assert_eq!(read(&dir, "cpu.max"), "200000 100000", "2 cores");
    assert_eq!(read(&dir, "memory.max"), (256 * MIB).to_string());
    assert_eq!(read(&dir, "swap.max"), "0");
    let usage = out.usage.expect("confined jobs report usage");
    assert!(usage.cpu_usec.is_some(), "{usage:?}");
    let cg = confined().cgroups.unwrap().job_path(9101, 0);
    assert!(!cg.exists(), "job cgroup removed after the job");
}

#[test]
#[ignore = "real host: needs cgroup v2 (run testkit/validate-host.sh)"]
fn cgroup_memory_limit_oom_kills_the_job_not_the_host() {
    let dir = workdir("cg-oom");
    // `tail` buffers a newline-free stream without bound.
    let out = confined().run(&spec(9102, "tail /dev/zero", 1, 64, 60, &dir));
    assert_ne!(out.exit_code, 0, "the memory hog must be killed");
    assert!(!out.timed_out, "killed by the OOM killer, not the walltime");
    let peak = out.usage.and_then(|u| u.peak_mem_bytes);
    if let Some(peak) = peak {
        // memory.peak exists from Linux 5.19.
        assert!(peak <= 64 * MIB, "peak {peak} B over the 64 MiB limit");
    }
}

#[test]
#[ignore = "real host: needs cgroup v2 (run testkit/validate-host.sh)"]
fn cgroup_cpu_limit_caps_a_multithreaded_job_at_its_cores() {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    assert!(threads >= 2, "need >= 2 CPUs to show throttling");
    let dir = workdir("cg-cpu");
    // One pure spin per CPU for 3 s (no fork per iteration, so each one wants
    // a whole CPU): unconfined this burns `threads` CPUs. An earlier
    // `while [ $(date +%s) -lt $end ]` loop was fork-bound and could use
    // less than one CPU on a busy runner, failing the lower bound below.
    let cmd =
        format!("for i in $(seq {threads}); do timeout 3 sh -c 'while :; do :; done' & done; wait");
    // Build the launcher before timing: `Launcher::from_env` probes the
    // container runtime (`docker info`, up to 5 s on a cold daemon), which
    // must not count as time the job could have spent on its core.
    let launcher = confined();
    let t0 = Instant::now();
    let out = launcher.run(&spec(9103, &cmd, 1, 0, 60, &dir));
    let wall = t0.elapsed().as_micros() as f64;
    assert_eq!(out.exit_code, 0);
    let cpu = out.usage.and_then(|u| u.cpu_usec).expect("cpu.stat usage") as f64;
    // cpu.max "100000 100000": at most one CPU-second per second (plus one
    // period of slack for scheduling granularity).
    assert!(
        cpu <= 1.15 * wall + 100_000.0,
        "used {:.2} CPU-s in {:.2} s with a 1-core limit",
        cpu / 1e6,
        wall / 1e6
    );
    assert!(
        cpu >= 0.5 * wall,
        "the job should have used its core: {:.2} CPU-s in {:.2} s",
        cpu / 1e6,
        wall / 1e6
    );
}

#[test]
#[ignore = "real host: needs cgroup v2 (run testkit/validate-host.sh)"]
fn cgroup_kill_reaches_processes_that_left_the_process_group() {
    let dir = workdir("cg-escape");
    let needle = "sleep 4271";
    // `setsid` escapes the job's process group; only the cgroup holds it.
    let cmd = format!("setsid sh -c '{needle}' </dev/null >/dev/null 2>&1 & sleep 60");
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let killer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        flag.store(true, Ordering::Release);
    });
    let out = confined().run_until(&spec(9104, &cmd, 1, 0, 120, &dir), &cancel);
    killer.join().unwrap();
    assert_eq!(out.exit_code, KILLED_EXIT);
    std::thread::sleep(Duration::from_millis(300));
    let leaked = alive(needle);
    if leaked {
        let _ = Command::new("pkill").args(["-f", needle]).status();
    }
    assert!(!leaked, "a setsid'd child survived the cgroup kill");
}

#[test]
#[ignore = "real host: needs cgroup v2 (run testkit/validate-host.sh)"]
fn cgroup_walltime_kill_reports_124() {
    let dir = workdir("cg-wall");
    let launcher = confined(); // not timed: probes the container runtime
    let t0 = Instant::now();
    let out = launcher.run(&spec(9105, "sleep 60", 1, 0, 1, &dir));
    assert_eq!(out.exit_code, TIMEOUT_EXIT);
    assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
}

// --------------------------------------------------------------- containers

fn image() -> String {
    std::env::var("GNOMON_VALIDATE_IMAGE").unwrap_or_else(|_| "alpine:3.20".into())
}

/// A launcher using the host's real container runtime, image present
/// (pulled once: registries rate-limit repeated pulls).
fn containerized() -> Launcher {
    let runtime = ContainerRuntime::from_env();
    let run = |args: &[&str]| {
        Command::new(&runtime.bin)
            .args(args)
            .stdout(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    let ok = run(&["image", "inspect", &image()]) || run(&["pull", "-q", &image()]);
    assert!(
        ok,
        "`{} pull {}` failed: no usable runtime",
        runtime.bin,
        image()
    );
    Launcher {
        cgroups: None,
        runtime,
        user: None,
        privileged: false,
        poll: Duration::from_millis(100),
    }
}

fn in_container(mut s: LaunchSpec) -> LaunchSpec {
    s.container = Some(image());
    s
}

/// Whether the runtime still knows the job's container (running or not).
fn container_exists(l: &Launcher, job_id: u64) -> bool {
    let out = Command::new(&l.runtime.bin)
        .args(["ps", "-aq", "--filter"])
        .arg(format!("name=^{}$", container_name(job_id, 0)))
        .output()
        .unwrap();
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}

#[test]
#[ignore = "real host: needs a container runtime (run testkit/validate-host.sh)"]
fn container_exit_code_env_and_workdir_reach_the_job() {
    let l = containerized();
    let dir = workdir("ct-env");
    let cmd = "echo \"$GNOMON_JOB_ID $GNOMON_NNODES $GNOMON_CPUS\" > env; pwd > cwd; exit 7";
    let out = l.run(&in_container(spec(9201, cmd, 1, 0, 120, &dir)));
    assert_eq!(out.exit_code, 7);
    assert_eq!(read(&dir, "env"), "9201 1 1");
    assert_eq!(
        read(&dir, "cwd"),
        dir.display().to_string(),
        "bind-mounted workdir"
    );
    assert!(!container_exists(&l, 9201), "--rm removed the container");
}

#[test]
#[ignore = "real host: needs a container runtime (run testkit/validate-host.sh)"]
fn container_runs_as_the_requested_user() {
    let mut l = containerized();
    let dir = workdir("ct-user");
    let mut s = in_container(spec(9202, "id -u > uid; id -g > gid", 1, 0, 120, &dir));
    let want = if gnomon_agent::identity::is_privileged() {
        // A root agent (the production setup) runs the job as its submitter.
        l.privileged = true;
        s.user = "nobody".into();
        let id = gnomon_agent::identity::resolve("nobody").expect("a 'nobody' account");
        id.uid.to_string()
    } else if l.runtime.rootless {
        // Rootless Docker maps the agent to container root by design.
        "0".into()
    } else {
        let (uid, gid) = gnomon_agent::launch::current_user().unwrap();
        l.user = Some((uid, gid));
        uid.to_string()
    };
    let out = l.run(&s);
    assert_eq!(out.exit_code, 0);
    assert_eq!(read(&dir, "uid"), want);
}

#[test]
#[ignore = "real host: needs a container runtime (run testkit/validate-host.sh)"]
fn container_cpu_and_memory_limits_are_applied() {
    let l = containerized();
    let dir = workdir("ct-limits");
    // cgroup v2 (cpu.max) or v1 (cfs quota) inside the container.
    let cmd = r#"if [ -f /sys/fs/cgroup/cpu.max ]; then
            cat /sys/fs/cgroup/cpu.max > cpu; cat /sys/fs/cgroup/memory.max > mem
        else
            echo "$(cat /sys/fs/cgroup/cpu/cpu.cfs_quota_us) $(cat /sys/fs/cgroup/cpu/cpu.cfs_period_us)" > cpu
            cat /sys/fs/cgroup/memory/memory.limit_in_bytes > mem
        fi"#;
    let out = l.run(&in_container(spec(9203, cmd, 2, 128, 120, &dir)));
    assert_eq!(out.exit_code, 0);
    let cpu: Vec<u64> = read(&dir, "cpu")
        .split_whitespace()
        .map(|v| v.parse().unwrap())
        .collect();
    assert_eq!(cpu[0] / cpu[1], 2, "quota/period = 2 cores: {cpu:?}");
    assert_eq!(read(&dir, "mem"), (128 * MIB).to_string());
}

#[test]
#[ignore = "real host: needs a container runtime (run testkit/validate-host.sh)"]
fn container_memory_limit_oom_kills_the_job() {
    let l = containerized();
    let dir = workdir("ct-oom");
    let out = l.run(&in_container(spec(
        9204,
        "tail /dev/zero",
        1,
        64,
        120,
        &dir,
    )));
    assert_eq!(out.exit_code, 137, "OOM-killed (128 + SIGKILL)");
    assert!(!out.timed_out);
}

#[test]
#[ignore = "real host: needs a container runtime (run testkit/validate-host.sh)"]
fn container_walltime_stops_the_container_not_just_the_client() {
    let l = containerized();
    let dir = workdir("ct-wall");
    let t0 = Instant::now();
    let out = l.run(&in_container(spec(9205, "sleep 300", 1, 0, 3, &dir)));
    assert_eq!(out.exit_code, TIMEOUT_EXIT);
    assert!(t0.elapsed() < Duration::from_secs(40), "{:?}", t0.elapsed());
    assert!(!container_exists(&l, 9205), "container left behind");
}

#[test]
#[ignore = "real host: needs a container runtime (run testkit/validate-host.sh)"]
fn container_kill_channel_stops_the_container() {
    let l = containerized();
    let dir = workdir("ct-kill");
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let killer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        flag.store(true, Ordering::Release);
    });
    let out = l.run_until(
        &in_container(spec(9206, "sleep 300", 1, 0, 600, &dir)),
        &cancel,
    );
    killer.join().unwrap();
    assert_eq!(out.exit_code, KILLED_EXIT);
    assert!(out.killed);
    assert!(!container_exists(&l, 9206), "container left behind");
}
