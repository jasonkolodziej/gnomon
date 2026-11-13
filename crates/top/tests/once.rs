//! `gntop --once` end to end, without a network: an unparsable controller
//! address fails before any connection attempt, which exercises the
//! "disconnected" frame and the exit status CI smoke tests rely on.

use std::process::Command;

fn gntop(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gntop"))
        .args(args)
        .env_remove("GNOMON_ADDR")
        .output()
        .expect("spawn gntop")
}

#[test]
fn once_prints_a_plain_disconnected_frame_and_fails() {
    let out = gntop(&[
        "--once",
        "--addr",
        "not a uri",
        "--width",
        "90",
        "--height",
        "14",
    ]);
    assert!(!out.status.success(), "unreachable controller must fail");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(!stdout.contains('\x1b'), "no ANSI escapes: {stdout:?}");
    assert_eq!(stdout.lines().count(), 14);
    assert!(stdout.starts_with("gntop not a uri"), "{stdout}");
    assert!(stdout.contains("disconnected, retrying"), "{stdout}");
    assert!(stdout.contains("window -10m … +2h"), "{stdout}");
    assert!(stdout.lines().all(|l| l.chars().count() <= 90));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("unreachable"), "{stderr}");
}

#[test]
fn once_honours_window_and_user_flags() {
    let out = gntop(&[
        "--once",
        "--addr",
        "not a uri",
        "--history",
        "1h",
        "--horizon",
        "1d",
        "--user",
        "alice",
        "--width",
        "120",
        "--height",
        "10",
    ]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("window -1h … +1d"), "{stdout}");
    assert!(stdout.contains("user=alice"), "{stdout}");
}

#[test]
fn bad_durations_are_rejected_by_the_parser() {
    let out = gntop(&["--once", "--interval", "soon"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("invalid duration"), "{stderr}");
}
