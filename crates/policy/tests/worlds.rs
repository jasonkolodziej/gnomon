//! The admission, estimator and node-score worlds (ADR 0002 step 5, ADR
//! 0003, ADR 0004), with hand-written component fixtures.

use gnomon_core::{Job, JobId, Resources, Walltime};
use gnomon_graph::{ResourceGraph, VertexKind};
use gnomon_policy::*;
use gnomon_sched::{BackfillPolicy, Decision, Policy, RuntimeEstimator};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

fn job(cores: u32, gpus: u32, nodes: u32, wall: u64) -> Job {
    let mut j = Job::new(
        JobId(9),
        "j",
        "alice",
        Resources {
            cores,
            mem_mb: 512,
            gpus,
        },
        Walltime(Duration::from_secs(wall)),
    );
    j.nodes = nodes;
    j.submit_time = 100;
    j
}

fn limits() -> PluginLimits {
    PluginLimits::default()
}

// --------------------------------------------------------------- admission

fn admission(wat: String) -> AdmissionPlugin {
    AdmissionPlugin::from_bytes(wat.as_bytes(), limits()).unwrap()
}

#[test]
fn admission_accepts_rejects_and_rewrites() {
    let p = admission(fixtures::admission_rules());
    assert_eq!(p.admit(&job(8, 0, 2, 60), 0).unwrap(), Verdict::Accept);
    assert_eq!(
        p.admit(&job(64, 0, 2, 60), 0).unwrap(),
        Verdict::Reject("too many cores".into())
    );
    let Verdict::Rewrite(r) = p.admit(&job(8, 1, 2, 3600), 0).unwrap() else {
        panic!("expected a rewrite");
    };
    assert_eq!(
        r,
        Rewrite {
            partition: "gpu".into(),
            request: Resources {
                cores: 8,
                mem_mb: 512,
                gpus: 1
            },
            nodes: 2,
            walltime_secs: 3600,
        }
    );
}

#[test]
fn invalid_rewrites_are_errors() {
    let p = admission(fixtures::admission_bad_rewrite());
    assert!(matches!(
        p.admit(&job(1, 0, 1, 60), 0),
        Err(PluginError::Invalid(_))
    ));
}

#[test]
fn admission_hook_applies_rewrites_and_honours_the_fail_mode() {
    let hook = AdmissionHook::new(admission(fixtures::admission_rules()), FailMode::Open);
    let mut j = job(8, 1, 2, 3600);
    assert!(matches!(hook.admit(&mut j, 0), (Ok(()), None)));
    assert_eq!(
        (j.partition.as_str(), j.id, j.user.as_str()),
        ("gpu", JobId(9), "alice")
    );
    let mut big = job(64, 0, 2, 60);
    assert_eq!(hook.admit(&mut big, 0).0, Err("too many cores".into()));
    assert_eq!(hook.rewritten.load(Ordering::Relaxed), 1);
    assert_eq!(hook.rejected.load(Ordering::Relaxed), 1);

    let open = AdmissionHook::new(admission(fixtures::admission_trap()), FailMode::Open);
    let mut j = job(1, 0, 1, 60);
    let before = j.clone();
    let (outcome, err) = open.admit(&mut j, 0);
    assert_eq!((outcome, j), (Ok(()), before));
    assert!(err.is_some());
    let closed = AdmissionHook::new(admission(fixtures::admission_trap()), FailMode::Closed);
    assert!(closed.admit(&mut job(1, 0, 1, 60), 0).0.is_err());
    assert_eq!(closed.failures.load(Ordering::Relaxed), 1);

    assert_eq!("closed".parse::<FailMode>(), Ok(FailMode::Closed));
    assert!("maybe".parse::<FailMode>().is_err());
}

#[test]
fn a_component_for_another_world_does_not_load() {
    let r = AdmissionPlugin::from_bytes(fixtures::size_age().as_bytes(), limits());
    assert!(matches!(r, Err(PluginError::Load(_))));
    let r = EstimatorPlugin::from_bytes(fixtures::admission_rules().as_bytes(), limits());
    assert!(matches!(r, Err(PluginError::Load(_))));
    let r = NodeScorePlugin::from_bytes(fixtures::half_walltime().as_bytes(), limits());
    assert!(matches!(r, Err(PluginError::Load(_))));
}

// --------------------------------------------------------------- estimator

fn estimator(wat: String) -> WasmEstimator {
    WasmEstimator::new(EstimatorPlugin::from_bytes(wat.as_bytes(), limits()).unwrap())
}

#[test]
fn estimates_are_clamped_cached_and_fall_back_to_walltime() {
    let e = estimator(fixtures::half_walltime());
    assert_eq!(e.planned_secs(&job(1, 0, 1, 3600)), 1800);
    assert_eq!(e.planned_secs(&job(1, 0, 1, 3600)), 1800);
    assert_eq!(e.calls.load(Ordering::Relaxed), 1, "cached per job");

    let mut short = job(1, 0, 1, 300);
    short.id = JobId(10);
    assert_eq!(e.planned_secs(&short), 300); // none: the walltime

    assert_eq!(
        estimator(fixtures::constant_estimate(0)).planned_secs(&job(1, 0, 1, 60)),
        1
    );
    let huge = estimator(fixtures::constant_estimate(1 << 40));
    assert_eq!(huge.planned_secs(&job(1, 0, 1, 60)), 60);

    let broken = estimator(fixtures::estimator_trap());
    assert_eq!(broken.planned_secs(&job(1, 0, 1, 60)), 60);
    assert_eq!(broken.failures.load(Ordering::Relaxed), 1);
}

#[test]
fn the_cache_shrinks_to_the_queue() {
    let e = estimator(fixtures::half_walltime());
    let jobs: Vec<Job> = (1..=3)
        .map(|i| {
            let mut j = job(1, 0, 1, 3600);
            j.id = JobId(i);
            j
        })
        .collect();
    for j in &jobs {
        e.planned_secs(j);
    }
    let shape = gnomon_sched::MachineShape::of(&graph(&[("a", 0)]));
    e.begin_pass(&[&jobs[2]], shape, 0);
    e.planned_secs(&jobs[2]);
    assert_eq!(e.calls.load(Ordering::Relaxed), 3, "job 3 still cached");
    e.planned_secs(&jobs[0]);
    assert_eq!(e.calls.load(Ordering::Relaxed), 4, "job 1 was dropped");
}

#[test]
fn backfill_plans_with_the_plugin_estimate() {
    let e = Arc::new(estimator(fixtures::half_walltime()));
    let policy = BackfillPolicy::with_estimator(86_400, e);
    let g = graph(&[("a", 0)]);
    match policy.place(&g, &job(1, 0, 1, 3600), 0, true) {
        Decision::Start { dur, .. } => assert_eq!(dur, Some(1800)),
        d => panic!("{d:?}"),
    }
}

// -------------------------------------------------------------- node-score

/// 8-core nodes named as given, each with `busy` cores taken for a day.
fn graph(nodes: &[(&str, u32)]) -> ResourceGraph {
    let mut g = ResourceGraph::default();
    let root = g.add_vertex(VertexKind::Cluster, "c", Resources::default(), None);
    for &(name, busy) in nodes {
        let cap = Resources {
            cores: 8,
            mem_mb: 4096,
            gpus: 0,
        };
        let id = g.add_vertex(VertexKind::Node, name, cap, Some(root));
        if busy > 0 {
            let used = Resources {
                cores: busy,
                mem_mb: 0,
                gpus: 0,
            };
            assert!(g.commit(&[id], &used, 0, 86_400));
        }
    }
    g
}

fn scored(wat: String) -> ScoredPolicy {
    let plugin = NodeScorePlugin::from_bytes(wat.as_bytes(), limits()).unwrap();
    ScoredPolicy::new(Arc::new(BackfillPolicy::new(86_400)), plugin)
}

fn names(g: &ResourceGraph, d: &Decision) -> Vec<String> {
    let v = match d {
        Decision::Start { vertices, .. } | Decision::Reserve { vertices, .. } => vertices,
        Decision::Defer => panic!("deferred"),
    };
    v.iter().map(|&id| g.vertex(id).name.clone()).collect()
}

#[test]
fn candidates_are_lowered_with_free_capacity() {
    let p = NodeScorePlugin::from_bytes(fixtures::most_free_cores().as_bytes(), limits()).unwrap();
    let c = |name: &str, free| Candidate {
        name: name.into(),
        capacity: Resources {
            cores: 8,
            mem_mb: 0,
            gpus: 0,
        },
        free: Resources {
            cores: free,
            mem_mb: 0,
            gpus: 0,
        },
    };
    let scores = p.score(&job(1, 0, 1, 60), &[c("x", 3), c("y", 8), c("z", 0)], 0);
    assert_eq!(scores.unwrap(), [3, 8, 0]);
    let none = p.score(&job(1, 0, 1, 60), &[], 0).unwrap();
    assert!(none.is_empty());
}

#[test]
fn spread_and_pack_pick_different_nodes() {
    let g = graph(&[("na", 6), ("nb", 0), ("nc", 2), ("nd", 4)]);
    let j = job(2, 0, 2, 3600);
    let spread = scored(fixtures::most_free_cores()).place(&g, &j, 0, true);
    assert_eq!(names(&g, &spread), ["nb", "nc"]);
    let pack = scored(fixtures::fewest_free_cores()).place(&g, &j, 0, true);
    assert_eq!(names(&g, &pack), ["na", "nd"]);
    // Same start either way: the plugin chooses where, never when.
    let base = BackfillPolicy::new(86_400).place(&g, &j, 0, true);
    for d in [&spread, &pack] {
        assert!(matches!((d, &base),
            (Decision::Start { start: a, .. }, Decision::Start { start: b, .. }) if a == b));
    }
}

#[test]
fn names_reach_the_plugin_and_reservations_are_rescored() {
    // Every node is too busy now; the 7-core job is reserved for when the
    // day-long load ends, and may go to any node then.
    let g = graph(&[("na", 4), ("nb", 4), ("nc", 4)]);
    let p = scored(fixtures::prefer_name_ending_b());
    let d = p.place(&g, &job(7, 0, 1, 3600), 0, true);
    assert!(
        matches!(d, Decision::Reserve { start: 86_400, .. }),
        "{d:?}"
    );
    assert_eq!(names(&g, &d), ["nb"]);
}

#[test]
fn no_choice_means_no_call_and_failures_keep_the_default() {
    let g = graph(&[("na", 0), ("nb", 8)]);
    let p = scored(fixtures::most_free_cores());
    p.place(&g, &job(4, 0, 1, 60), 0, true);
    assert_eq!(p.calls.load(Ordering::Relaxed), 0, "only na fits");

    let g = graph(&[("na", 0), ("nb", 0), ("nc", 0)]);
    let base = BackfillPolicy::new(86_400).place(&g, &job(4, 0, 2, 60), 0, true);
    for wat in [fixtures::wrong_length(), fixtures::node_score_trap()] {
        let p = scored(wat);
        assert_eq!(p.place(&g, &job(4, 0, 2, 60), 0, true), base);
        assert_eq!(p.failures.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn a_scored_backfill_pass_never_overcommits_a_node() {
    use gnomon_core::Event;
    use gnomon_fsm::Fsm;
    use gnomon_sched::{backfill_pass, PlanApplier};
    let cap = Resources {
        cores: 32,
        mem_mb: 65_536,
        gpus: 0,
    };
    let cluster = || {
        let fsm = Fsm::new();
        for n in 0..16 {
            fsm.apply(Event::NodeRegistered {
                name: format!("n{n:02}"),
                capacity: cap,
            });
        }
        for i in 0..200u64 {
            let cores = [16, 8, 4, 12][(i % 4) as usize];
            let mut j = job(cores, 0, 1 + u32::from(i % 3 == 0), 600 + 30 * (i % 40));
            j.id = JobId(i + 1);
            j.priority = 1_000 - i;
            fsm.apply(Event::JobSubmitted(j));
        }
        fsm
    };
    for wat in [fixtures::most_free_cores(), fixtures::fewest_free_cores()] {
        let fsm = cluster();
        let policy = scored(wat).with_max_candidates(Some(3));
        let applier = PlanApplier::new();
        let stats = backfill_pass(&fsm, &policy, &applier, Default::default());
        assert_eq!(stats.conflicts, 0);
        assert!(stats.started > 0 && policy.calls.load(Ordering::Relaxed) > 0);
        let snap = fsm.snapshot();
        for id in snap.graph().schedulable_nodes() {
            let peak = snap.graph().vertex(id).planner.peak_usage(0, u64::MAX / 2);
            assert!(peak.fits_in(&cap), "node {id} overcommitted: {peak:?}");
        }
    }
}
