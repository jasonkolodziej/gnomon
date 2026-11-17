//! Sandbox tests for priority plugins (ADR 0002, step 1): typed calls, and
//! a fallback, never a hang or a crash, for loops, memory bombs and traps.

use gnomon_core::{Job, JobId, Resources, Walltime};
use gnomon_policy::{
    fixtures, PluginError, PluginLimits, PolicyConfig, PriorityHook, PriorityPlugin,
};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

fn job() -> Job {
    let mut j = Job::new(
        JobId(7),
        "j",
        "alice",
        Resources {
            cores: 8,
            mem_mb: 1024,
            gpus: 1,
        },
        Walltime(Duration::from_secs(3600)),
    );
    j.partition = "gpu".into();
    j.nodes = 2;
    j.submit_time = 1_000;
    j
}

fn load(wat: &str) -> PriorityPlugin {
    PriorityPlugin::from_bytes(wat.as_bytes(), PluginLimits::default()).unwrap()
}

#[test]
fn typed_job_reaches_the_plugin() {
    // (8 + 4*1) * 2 nodes * 100 + (4600 - 1000) / 60 minutes
    assert_eq!(
        load(&fixtures::size_age()).priority(&job(), 4_600).unwrap(),
        2_460
    );
    // 'a' = 97, "gpu".len() = 3: strings are lowered into plugin memory.
    assert_eq!(
        load(&fixtures::strings()).priority(&job(), 0).unwrap(),
        97_003
    );
}

#[test]
fn calls_are_independent_and_repeatable() {
    let p = load(&fixtures::size_age());
    let first = p.priority(&job(), 4_600).unwrap();
    for _ in 0..100 {
        assert_eq!(p.priority(&job(), 4_600).unwrap(), first);
    }
}

#[test]
fn infinite_loop_runs_out_of_fuel() {
    let p = PriorityPlugin::from_bytes(
        fixtures::infinite_loop().as_bytes(),
        PluginLimits {
            fuel: 1_000_000,
            ..Default::default()
        },
    )
    .unwrap();
    let t = Instant::now();
    assert!(matches!(p.priority(&job(), 0), Err(PluginError::OutOfFuel)));
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[test]
fn memory_past_the_cap_traps() {
    let grow = load(&fixtures::memory_bomb());
    assert!(matches!(
        grow.priority(&job(), 0),
        Err(PluginError::Trap(_))
    ));
    let initial = load(&fixtures::big_initial_memory());
    assert!(matches!(
        initial.priority(&job(), 0),
        Err(PluginError::Trap(_))
    ));
}

#[test]
fn the_memory_cap_covers_all_memories_and_tables() {
    for wat in [fixtures::split_memory(), fixtures::table_bomb()] {
        match load(&wat).priority(&job(), 0) {
            Err(PluginError::Trap(e)) => assert!(format!("{e:#}").contains("budget"), "{e:#}"),
            other => panic!("expected a budget trap, got {other:?}"),
        }
    }
}

#[test]
fn traps_are_reported() {
    let p = load(&fixtures::trap());
    assert!(matches!(p.priority(&job(), 0), Err(PluginError::Trap(_))));
}

#[test]
fn bad_modules_fail_to_load() {
    let limits = PluginLimits::default();
    assert!(matches!(
        PriorityPlugin::from_bytes(b"not wasm", limits),
        Err(PluginError::Load(_))
    ));
    // A core module (the old pointer/JSON interface) is not a component.
    let core = r#"(module (func (export "priority") (result i64) (i64.const 1)))"#;
    assert!(matches!(
        PriorityPlugin::from_bytes(core.as_bytes(), limits),
        Err(PluginError::Load(_))
    ));
    // Importing anything (here a WASI-like function) is refused: plugins are pure.
    let impure = fixtures::size_age().replacen(
        "(alias export $types",
        r#"(import "wasi:clocks/wall-clock@0.2.0" (instance (export "now" (func (result u64)))))
  (alias export $types"#,
        1,
    );
    assert!(matches!(
        PriorityPlugin::from_bytes(impure.as_bytes(), limits),
        Err(PluginError::Load(_))
    ));
    assert!(matches!(
        PriorityPlugin::from_file("/nonexistent/plugin.wasm", limits),
        Err(PluginError::Load(_))
    ));
}

#[test]
fn hook_falls_back_to_declarative_policy_and_counts_failures() {
    let cfg = PolicyConfig::default();
    let expected = cfg.priority(&job(), 4_600, "normal");

    let none = PriorityHook::new(None, cfg.clone());
    assert_eq!(none.priority(&job(), 4_600).0, expected);
    assert_eq!(none.calls.load(Ordering::Relaxed), 0);

    let ok = PriorityHook::new(Some(load(&fixtures::size_age())), cfg.clone());
    assert!(matches!(ok.priority(&job(), 4_600), (2_460, None)));

    let bad = PriorityHook::new(Some(load(&fixtures::trap())), cfg);
    let (v, err) = bad.priority(&job(), 4_600);
    assert_eq!(v, expected);
    assert!(err.is_some());
    assert_eq!(bad.calls.load(Ordering::Relaxed), 1);
    assert_eq!(bad.failures.load(Ordering::Relaxed), 1);
}

#[test]
fn shadow_priority_counts_disagreements_without_applying() {
    use gnomon_policy::{ShadowOutcome, ShadowPriority};
    let shadow = ShadowPriority::new(load(&fixtures::size_age()));
    let mut j = job();
    j.priority = 2_460;
    assert_eq!(shadow.compare(&j, 4_600), ShadowOutcome::Same);
    j.priority = 1;
    assert_eq!(shadow.compare(&j, 4_600), ShadowOutcome::Differs(2_460));
    assert_eq!(j.priority, 1, "nothing applied");
    assert!((shadow.agreement() - 0.5).abs() < 1e-9);
    let broken = ShadowPriority::new(load(&fixtures::trap()));
    assert!(matches!(broken.compare(&j, 0), ShadowOutcome::Failed(_)));
    assert_eq!(broken.agreement(), 0.0, "failures are not agreement");
    assert_eq!(broken.failures.load(Ordering::Relaxed), 1);
}
