//! Runs real `sh -c` jobs through the launcher with cgroups disabled.

use gnomon_agent::container::ContainerRuntime;
use gnomon_agent::launch::TIMEOUT_EXIT;
use gnomon_agent::{LaunchSpec, Launcher};
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
        workdir: std::env::temp_dir(),
    }
}

fn launcher() -> Launcher {
    Launcher {
        cgroups: None,
        runtime: ContainerRuntime::from_bin("docker"),
        user: None,
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
