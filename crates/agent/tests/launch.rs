//! Runs real `sh -c` jobs through the launcher with cgroups disabled.

use gnomon_agent::container::ContainerRuntime;
use gnomon_agent::launch::{KILLED_EXIT, TIMEOUT_EXIT};
use gnomon_agent::{LaunchSpec, Launcher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn spec(command: &str, walltime_secs: u64) -> LaunchSpec {
    LaunchSpec {
        job_id: 1,
        name: "it".into(),
        command: command.into(),
        cores: 1,
        mem_mb: 0,
        gpus: 0,
        walltime_secs,
        container: None,
        nodelist: vec!["local".into()],
        rank: 0,
        user: String::new(),
        start: 0,
        workdir: std::env::temp_dir(),
    }
}

fn launcher() -> Launcher {
    Launcher {
        cgroups: None,
        runtime: ContainerRuntime::from_bin("docker"),
        user: None,
        privileged: false,
        poll: Duration::from_millis(50),
    }
}

#[test]
fn exit_code_is_reported() {
    let out = launcher().run(&spec("exit 3", 10));
    assert_eq!(out.exit_code, 3);
    assert!(!out.timed_out);
}

#[test]
fn walltime_kills_long_job_with_exit_124() {
    let t0 = Instant::now();
    let out = launcher().run(&spec("sleep 30", 1));
    assert_eq!(out.exit_code, TIMEOUT_EXIT);
    assert!(out.timed_out);
    assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
}

#[test]
fn kill_flag_stops_a_running_job_with_exit_143() {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let killer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        flag.store(true, Ordering::Release);
    });
    let t0 = Instant::now();
    let out = launcher().run_until(&spec("sleep 30", 60), &cancel);
    killer.join().unwrap();
    assert_eq!(out.exit_code, KILLED_EXIT);
    assert!(out.killed && !out.timed_out);
    assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
}

#[test]
fn a_rank_killed_before_launch_never_runs() {
    let marker = std::env::temp_dir().join(format!("gnomon-never-ran-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let out = launcher().run_until(
        &spec(&format!("touch {}", marker.display()), 10),
        &AtomicBool::new(true),
    );
    assert_eq!(out.exit_code, KILLED_EXIT);
    assert!(!marker.exists(), "the job must not have started");
}

/// Whether any process still has `marker` in its command line. Uses `ps`
/// rather than `/proc`, which macOS does not have; `ps` prints each
/// process's arguments joined by spaces on both Linux and macOS.
fn alive(marker: &str) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-A", "-o", "command="])
        .output()
        .expect("ps");
    assert!(out.status.success(), "ps failed: {out:?}");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l.contains(marker))
}

#[test]
fn unconfined_kill_reaches_grandchildren() {
    // `sleep` runs as a grandchild of the job shell (in a subshell), the
    // case that leaked when only the direct child was SIGKILLed.
    let marker = format!("{}.{}", 7_000 + std::process::id() % 1000, 731);
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let killer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        flag.store(true, Ordering::Release);
    });
    let out = launcher().run_until(
        &spec(
            &format!("(sleep {marker} </dev/null >/dev/null 2>&1; true); true"),
            60,
        ),
        &cancel,
    );
    killer.join().unwrap();
    assert_eq!(out.exit_code, KILLED_EXIT);
    std::thread::sleep(Duration::from_millis(200));
    let leaked = alive(&format!("sleep {marker}"));
    if leaked {
        let _ = std::process::Command::new("pkill")
            .args(["-f", &format!("sleep {marker}")])
            .status();
    }
    assert!(!leaked, "grandchild survived the kill");
}
