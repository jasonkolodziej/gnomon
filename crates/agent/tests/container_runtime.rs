//! Container launches against a fake runtime script that records its argv,
//! so no Docker daemon or network is needed. One test per binary: writing
//! and exec'ing a script while other test threads fork could hit ETXTBSY.

use gnomon_agent::container::ContainerRuntime;
use gnomon_agent::launch::TIMEOUT_EXIT;
use gnomon_agent::{LaunchSpec, Launcher};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A fake `docker`: `run` sleeps when the command mentions "sleep", else
/// exits 9; every call is appended to `calls.log` next to the script.
const FAKE: &str = r#"#!/bin/sh
dir=$(dirname "$0")
echo "$*" >> "$dir/calls.log"
case "$1" in
  run) case "$*" in *sleep*) exec sleep 30 ;; esac; exit 9 ;;
  kill) exit 0 ;;
esac
"#;

fn spec(command: &str, walltime_secs: u64) -> LaunchSpec {
    LaunchSpec {
        job_id: 77,
        name: "c".into(),
        command: command.into(),
        cores: 2,
        mem_mb: 0,
        gpus: 1,
        walltime_secs,
        container: Some("alpine:3.20".into()),
        nodelist: vec!["local".into()],
        rank: 0,
        workdir: std::env::temp_dir(),
    }
}

#[test]
fn container_jobs_run_through_runtime_and_walltime_kills_container() {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("gnomon-fake-runtime-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("docker");
    std::fs::write(&bin, FAKE).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let l = Launcher {
        cgroups: None,
        runtime: ContainerRuntime::from_bin(bin.to_str().unwrap()),
        user: Some((1000, 1000)),
        poll: Duration::from_millis(50),
    };

    let out = l.run(&spec("exit 0", 10));
    assert_eq!(out.exit_code, 9, "runtime's exit code is the job's");
    assert_eq!(out.usage, None, "limits are delegated to the runtime");

    let t0 = Instant::now();
    let out = l.run(&spec("sleep 30", 1));
    assert_eq!(out.exit_code, TIMEOUT_EXIT);
    assert!(t0.elapsed() < Duration::from_secs(3));

    let log = std::fs::read_to_string(dir.join("calls.log")).unwrap();
    let calls: Vec<&str> = log.lines().collect();
    assert_eq!(calls.len(), 3, "{log}");
    assert!(calls[0].starts_with("run --rm --name gnomon-job-77 --cpus 2 --gpus 1"));
    assert!(calls[0].contains("--user 1000:1000"));
    assert!(calls[0].ends_with("--entrypoint sh alpine:3.20 -c exit 0"));
    assert_eq!(calls[2], "kill gnomon-job-77");
    let _ = std::fs::remove_dir_all(&dir);
}
