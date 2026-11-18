//! Sample data: what the UI shows with `?demo` (no server), in previews and
//! in screenshots. Shaped like a real 8-node cluster behind three Raft
//! replicas with every plugin world configured.

use gnomon_web_model::{
    ClusterView, ControllerView, EventView, JobView, LabReport, LabRequest, LabRow, MemberView,
    NodeView, PluginView, PolicyView,
};

/// The fixture's clock.
pub const NOW: u64 = 1_794_990_600; // 13:10:00 UTC

fn job(
    id: u64,
    name: &str,
    user: &str,
    state: &str,
    cores: u32,
    n: u32,
    start: i64,
    span: u64,
    nodes: &[&str],
) -> JobView {
    JobView {
        id,
        name: name.into(),
        user: user.into(),
        partition: if name.contains("gpu") {
            "gpu".into()
        } else {
            "normal".into()
        },
        state: state.into(),
        cores,
        mem_mb: u64::from(cores) * 2048,
        gpus: if name.contains("gpu") { 2 } else { 0 },
        num_nodes: n,
        walltime: span + span / 3,
        priority: 1000 - id,
        submit_time: (NOW as i64 + start.min(0) - 300) as u64,
        start: if state == "PENDING" {
            0
        } else {
            (NOW as i64 + start) as u64
        },
        span,
        nodes: nodes.iter().map(|s| s.to_string()).collect(),
        container: if name.starts_with("train") {
            "pytorch:2.4-cuda".into()
        } else {
            String::new()
        },
    }
}

fn node(name: &str, alive: bool, kill: &[u64]) -> NodeView {
    NodeView {
        name: name.into(),
        cores: 32,
        mem_mb: 131_072,
        gpus: if name >= "node06" { 4 } else { 0 },
        used_cores: 0,
        used_mem_mb: 0,
        used_gpus: 0,
        last_heartbeat: if alive { NOW - 1 } else { NOW - 95 },
        alive,
        kill_pending: kill.to_vec(),
    }
}

fn plugin(world: &str, source: &str, mode: &str, counters: &[(&str, u64)]) -> PluginView {
    PluginView {
        world: world.into(),
        source: source.into(),
        mode: mode.into(),
        counters: counters.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
    }
}

fn ev(index: u64, ago: u64, kind: &str, job: Option<u64>, node: &str, detail: &str) -> EventView {
    EventView {
        index,
        at: NOW - ago,
        kind: kind.into(),
        job,
        node: node.into(),
        detail: detail.into(),
        accepted: true,
    }
}

pub fn view() -> ClusterView {
    let jobs = vec![
        job(
            41,
            "train-llm",
            "ada",
            "RUNNING",
            16,
            4,
            -1500,
            5400,
            &["node00", "node01", "node02", "node03"],
        ),
        job(
            44,
            "cfd-sweep",
            "lin",
            "RUNNING",
            8,
            1,
            -600,
            1800,
            &["node04"],
        ),
        job(45, "render", "kai", "RUNNING", 4, 1, -300, 900, &["node04"]),
        job(
            46,
            "gpu-infer",
            "ada",
            "RUNNING",
            8,
            2,
            -900,
            2400,
            &["node06", "node07"],
        ),
        job(
            47,
            "md-sim",
            "noor",
            "RUNNING",
            24,
            1,
            -120,
            3000,
            &["node05"],
        ),
        job(
            48,
            "mpi-solver",
            "lin",
            "FAILED",
            16,
            3,
            -400,
            300,
            &["node00", "node01", "node05"],
        ),
        job(
            49,
            "train-vit",
            "kai",
            "RESERVED",
            32,
            4,
            3900,
            3600,
            &["node00", "node01", "node02", "node03"],
        ),
        job(50, "etl", "sam", "RUNNING", 2, 1, -60, 600, &["node02"]),
        job(
            51,
            "backfill-short",
            "sam",
            "RUNNING",
            6,
            1,
            -30,
            420,
            &["node03"],
        ),
        job(
            52,
            "genome-align",
            "noor",
            "RESERVED",
            16,
            1,
            1500,
            2700,
            &["node04"],
        ),
        job(
            53,
            "gpu-finetune",
            "ada",
            "RESERVED",
            16,
            2,
            1440,
            3600,
            &["node06", "node07"],
        ),
        job(54, "postproc", "lin", "PENDING", 64, 1, 0, 1200, &[]),
        job(55, "sweep-17", "kai", "PENDING", 4, 1, 0, 600, &[]),
        job(56, "sweep-18", "kai", "PENDING", 4, 1, 0, 600, &[]),
        job(
            40,
            "nightly-build",
            "ci",
            "COMPLETED",
            8,
            1,
            -3000,
            1400,
            &["node05"],
        ),
        job(39, "lint", "ci", "CANCELLED", 1, 1, -2600, 200, &["node02"]),
        job(
            38,
            "bench",
            "ci",
            "COMPLETED",
            16,
            2,
            -3500,
            1800,
            &["node06", "node07"],
        ),
    ];
    let nodes = vec![
        node("node00", true, &[]),
        node("node01", true, &[]),
        node("node02", true, &[]),
        node("node03", true, &[]),
        node("node04", true, &[]),
        node("node05", true, &[48]),
        node("node06", true, &[]),
        node("node07", false, &[]),
    ];
    ClusterView {
        controller: "http://ctl2:7631".into(),
        connected: true,
        error: None,
        now: NOW,
        state_index: 18_734,
        writable: true,
        cluster: ControllerView {
            replicated: true,
            node_id: 2,
            role: "follower".into(),
            leader: Some(1),
            term: 7,
            last_log_index: 18_912,
            last_applied: 18_911,
            fsm_index: 18_734,
            members: vec![
                MemberView {
                    id: 1,
                    addr: "ctl1:7700".into(),
                    leader: true,
                    matched: Some(18_912),
                },
                MemberView {
                    id: 2,
                    addr: "ctl2:7700".into(),
                    leader: false,
                    matched: Some(18_912),
                },
                MemberView {
                    id: 3,
                    addr: "ctl3:7700".into(),
                    leader: false,
                    matched: Some(18_874),
                },
            ],
        },
        policy: PolicyView {
            name: "node-score(backfill/wasm-estimator)".into(),
            fuel: 10_000_000,
            memory_mb: 64,
            plugins: vec![
                plugin(
                    "admission",
                    "/etc/gnomon/admission.wasm",
                    "fail open",
                    &[
                        ("calls", 1_204),
                        ("failures", 0),
                        ("rejected", 31),
                        ("rewritten", 118),
                    ],
                ),
                plugin(
                    "priority",
                    "/etc/gnomon/fairshare.wasm",
                    "fallback qos normal",
                    &[("calls", 1_173), ("failures", 2)],
                ),
                plugin(
                    "shadow-priority",
                    "/etc/gnomon/fairshare-v2.wasm",
                    "compared, never applied",
                    &[("compared", 1_173), ("differed", 87), ("failures", 0)],
                ),
                plugin(
                    "estimator",
                    "/etc/gnomon/runtime-model.wasm",
                    "walltime fallback",
                    &[("calls", 1_150), ("failures", 4)],
                ),
                plugin(
                    "node-score",
                    "/etc/gnomon/gpu-locality.wasm",
                    "64 candidates per call",
                    &[("calls", 642), ("failures", 0)],
                ),
            ],
        },
        nodes,
        jobs,
    }
}

pub fn events() -> Vec<EventView> {
    vec![
        ev(
            18_700,
            410,
            "JobSubmitted",
            Some(48),
            "",
            "mpi-solver by lin: 16c 32768MB 0g x3 node(s), 1800s, prio 952",
        ),
        ev(
            18_702,
            400,
            "JobStarted",
            Some(48),
            "",
            "vertices [1, 2, 6] at 1794990200",
        ),
        ev(
            18_705,
            395,
            "AssignmentsFetched",
            None,
            "node00",
            "1 assignment(s)",
        ),
        ev(
            18_709,
            120,
            "JobStarted",
            Some(47),
            "",
            "vertices [6] at 1794990480, span 3000s",
        ),
        ev(
            18_712,
            95,
            "NodeLost",
            None,
            "node07",
            "no heartbeat since 1794990505",
        ),
        ev(
            18_715,
            70,
            "JobSubmitted",
            Some(55),
            "",
            "sweep-17 by kai: 4c 8192MB 0g x1 node(s), 600s, prio 945",
        ),
        ev(
            18_716,
            69,
            "JobSubmitted",
            Some(56),
            "",
            "sweep-18 by kai: 4c 8192MB 0g x1 node(s), 600s, prio 944",
        ),
        ev(
            18_720,
            60,
            "JobStarted",
            Some(50),
            "",
            "vertices [3] at 1794990540, span 600s",
        ),
        ev(
            18_722,
            31,
            "JobReserved",
            Some(53),
            "",
            "vertices [7, 8] from 1794992040, span 3600s",
        ),
        ev(
            18_725,
            30,
            "JobStarted",
            Some(51),
            "",
            "vertices [4] at 1794990570, span 420s",
        ),
        ev(
            18_728,
            12,
            "JobRankFinished",
            Some(48),
            "node00",
            "exit 3 at 1794990588",
        ),
        ev(
            18_729,
            11,
            "JobRankFinished",
            Some(48),
            "node01",
            "exit 143 at 1794990589",
        ),
        ev(
            18_731,
            6,
            "JobExtended",
            Some(44),
            "",
            "planned end now 1794992400",
        ),
        ev(
            18_734,
            1,
            "ReservationsCleared",
            None,
            "",
            "re-planning reservations",
        ),
    ]
}

/// The lab table for the default request, from a real gnomon-web run.
pub fn lab_report(req: &LabRequest) -> LabReport {
    let row =
        |label: &str, policy: &str, w: f64, p95: f64, b: f64, wide: f64, u: f64, m: f64| LabRow {
            policy: policy.into(),
            label: label.into(),
            mean_wait_h: w,
            p95_wait_h: p95,
            mean_bsld: b,
            wide_wait_h: wide,
            utilization: u,
            makespan_h: m,
        };
    LabReport {
        request: req.clone(),
        rows: vec![
            row(
                "walltime backfill (default)",
                "backfill/walltime",
                4.23,
                13.9,
                38.2,
                11.11,
                0.71,
                52.4,
            ),
            row(
                "runtime quantile θ=0.9",
                "backfill/quantile-0.90",
                4.58,
                14.6,
                41.0,
                10.94,
                0.73,
                51.0,
            ),
            row(
                "bias-corrected predictor",
                "backfill/bias",
                4.25,
                13.2,
                36.8,
                10.94,
                0.72,
                51.6,
            ),
            row(
                "bias-corrected + wide-job shield",
                "backfill/bias-ws.plan",
                3.67,
                12.1,
                31.5,
                10.61,
                0.73,
                51.1,
            ),
        ],
        millis: 640,
    }
}
