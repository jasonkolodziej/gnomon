//! Reproducible evaluation of Gnomon's scheduling features on real HPC
//! traces from the Parallel Workloads Archive (SWF).
//!
//! usage: `experiments <dir-with-swf-traces> <out-dir> [jobs-per-window]
//! [--only e0|e1|e2|e3|e4] [--priority-plugin candidate.wasm]`
//!
//! E0  exact replay: journal -> identical state; recovered history under the
//!     same policy -> identical start times.
//! E1  distribution-aware backfill: walltime baseline vs runtime quantiles
//!     (risk budgets) vs bias-corrected prediction vs a perfect-knowledge
//!     oracle (bias-corrected subset also in `e1b_bias_corrected.csv`).
//! E2  time-varying capacity: power cap and carbon windows, planner-aware vs
//!     planner-unaware.
//! E3  shadow mode on live traffic.
//! E4  candidate priority plugin (only with `--priority-plugin`): every
//!     trace window scheduled under FCFS priorities and under the plugin's
//!     submit-time priorities (ADR 0002, step 4).

use gnomon_core::{CapacityProfile, Job, JobId, Resources};
use gnomon_policy::{PluginLimits, PriorityPlugin};
use gnomon_sched::{
    BackfillPolicy, BiasConfig, BiasCorrectedEstimator, GuardMode, NarrowAhead, Policy,
    QuantileEstimator, RuntimeEstimator, ShadowPolicy, WideSafeBackfill,
};
use gnomon_sim::{metrics, priority, replay, swf, Machine, SimJob, Simulator};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Instant;

const WINDOW_SECS: u64 = 72 * 3600;

#[derive(Debug)]
struct Oracle(HashMap<JobId, u64>);
impl RuntimeEstimator for Oracle {
    fn name(&self) -> String {
        "oracle".into()
    }
    fn planned_secs(&self, job: &Job) -> u64 {
        *self.0.get(&job.id).unwrap_or(&job.walltime.secs())
    }
}

fn quantile_policy(theta: f64, history: &[SimJob]) -> Arc<dyn Policy> {
    let est = Arc::new(QuantileEstimator::new(theta));
    for h in history {
        est.observe(&h.job, h.actual);
    } // warm start from accounting history
    Arc::new(BackfillPolicy::with_estimator(WINDOW_SECS, est))
}

fn bias_policy(cfg: BiasConfig, history: &[SimJob]) -> Arc<dyn Policy> {
    let est = Arc::new(BiasCorrectedEstimator::new(cfg));
    for h in history {
        est.observe(&h.job, h.actual);
    }
    Arc::new(BackfillPolicy::with_estimator(WINDOW_SECS, est))
}

fn shielded_policy(
    cfg: BiasConfig,
    ahead: NarrowAhead,
    history: &[SimJob],
) -> Arc<WideSafeBackfill> {
    let est = Arc::new(BiasCorrectedEstimator::new(cfg));
    for h in history {
        est.observe(&h.job, h.actual);
    }
    Arc::new(WideSafeBackfill::new(WINDOW_SECS, est).with_ahead(ahead))
}

/// Variants run with the wide-job shield in E1 (§3b): the selected one
/// (default predictor, narrow jobs ahead at their predicted span) and
/// ablations of where a new slot goes and of slip protection.
fn shielded_variants() -> Vec<(BiasConfig, NarrowAhead)> {
    let d = BiasConfig::default();
    let no_slip = BiasConfig {
        protect_slipped: false,
        ..d
    };
    vec![
        (d, NarrowAhead::Planned),
        (d, NarrowAhead::Walltime),
        (d, NarrowAhead::Skip),
        (no_slip, NarrowAhead::Planned),
    ]
}

/// Bias-corrected variants scored in E1: the selected default and ablations
/// of its parts (cautious span, slip protection, guard mode, wide jobs at
/// walltime, planning level).
fn bias_variants() -> Vec<BiasConfig> {
    let d = BiasConfig::default();
    vec![
        d,
        BiasConfig {
            protect_slipped: false,
            ..d
        },
        BiasConfig {
            guard_tau: None,
            protect_slipped: false,
            ..d
        },
        BiasConfig {
            guard_mode: GuardMode::Block,
            ..d
        },
        BiasConfig {
            wide_at_walltime: true,
            ..d
        },
        BiasConfig { tau: 0.5, ..d },
    ]
}

fn cores(c: u32) -> Resources {
    Resources {
        cores: c,
        mem_mb: 0,
        gpus: 0,
    }
}

/// Stylised hourly grid carbon intensity (g CO2/kWh, UTC hours): a solar-heavy
/// "duck curve" — low midday, high evening/night. Illustrative, not a
/// measured grid.
const CARBON: [f64; 24] = [
    480., 470., 460., 455., 460., 470., 450., 400., 340., 280., 230., 200., 190., 195., 220., 270.,
    340., 430., 520., 560., 550., 530., 510., 495.,
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().collect();
    let only = match args.iter().position(|a| a == "--only") {
        Some(i) if i + 1 < args.len() => {
            let v = args.remove(i + 1);
            args.remove(i);
            Some(v)
        }
        _ => None,
    };
    let plugin = match args.iter().position(|a| a == "--priority-plugin") {
        Some(i) if i + 1 < args.len() => {
            let path = args.remove(i + 1);
            args.remove(i);
            let p = PriorityPlugin::from_file(&path, PluginLimits::default())
                .map_err(|e| format!("--priority-plugin {path}: {e}"))?;
            Some((path, p))
        }
        Some(_) => return Err("--priority-plugin needs a path".into()),
        None => None,
    };
    let run = |e: &str| only.as_deref().is_none_or(|o| o == e);
    let dir = args
        .get(1)
        .expect("usage: experiments <swf-dir> <out-dir> [jobs]");
    let out = args
        .get(2)
        .expect("usage: experiments <swf-dir> <out-dir> [jobs]");
    let n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5000);
    std::fs::create_dir_all(out).unwrap();

    let names = [
        "SDSC-SP2-1998-4.2-cln.swf",
        "CTC-SP2-1996-3.1-cln.swf",
        "HPC2N-2002-2.2-cln.swf",
    ];
    let traces: Vec<swf::Trace> = names
        .iter()
        .map(|f| {
            let text = std::fs::read_to_string(format!("{dir}/{f}")).expect(f);
            swf::parse(
                f.split('-').take(2).collect::<Vec<_>>().join("-").as_str(),
                &text,
            )
        })
        .collect();
    let offsets = [10_000usize, 20_000, 30_000];
    let mut md = String::new();
    writeln!(md, "# Gnomon feature evaluation\n").unwrap();
    for t in &traces {
        writeln!(
            md,
            "- {}: {} jobs after filtering, {} processors",
            t.name,
            t.jobs.len(),
            t.max_procs
        )
        .unwrap();
    }
    writeln!(md, "- windows: {n} consecutive jobs at offsets {offsets:?}; learned models warm-started on the preceding 2000 jobs\n").unwrap();

    // ---------------- E0: exact replay -----------------------------------
    if run("e0") {
        let t = &traces[0];
        let w = t.window(offsets[0], n);
        let sim = Simulator::new(
            Machine::single(t.max_procs),
            Arc::new(BackfillPolicy::new(WINDOW_SECS)),
        );
        let t0 = Instant::now();
        let r = sim.run(&w);
        let sim_ms = t0.elapsed().as_millis();
        let jsonl = replay::to_jsonl(&r.journal);
        let journal = replay::from_jsonl(&jsonl)?;
        let t1 = Instant::now();
        let rebuilt = replay::rebuild(&journal);
        let replay_ms = t1.elapsed().as_millis();
        // Original final state vs rebuilt: compare via a second rebuild of the
        // in-memory journal (the simulator's FSM is internal to the run).
        let same_state =
            replay::fingerprint(&rebuilt) == replay::fingerprint(&replay::rebuild(&r.journal));
        let recovered = replay::workload_from_journal(&journal);
        let r2 = sim.run(&recovered);
        let same_starts = r
            .outcomes
            .iter()
            .zip(&r2.outcomes)
            .all(|(a, b)| a.id == b.id && a.start == b.start)
            && r.outcomes.len() == r2.outcomes.len();
        writeln!(md, "## E0 — exact replay ({})\n", t.name).unwrap();
        writeln!(md, "| check | result |\n|---|---|").unwrap();
        writeln!(
            md,
            "| journal events | {} ({} KiB JSONL) |",
            journal.len(),
            jsonl.len() / 1024
        )
        .unwrap();
        writeln!(
            md,
            "| JSONL round-trip lossless | {} |",
            journal == r.journal
        )
        .unwrap();
        writeln!(md, "| replayed state identical | {same_state} |").unwrap();
        writeln!(
            md,
            "| recovered history, same policy → identical start times ({} jobs) | {same_starts} |",
            r.outcomes.len()
        )
        .unwrap();
        writeln!(
            md,
            "| simulate {n} jobs / rebuild from journal | {sim_ms} ms / {replay_ms} ms |\n"
        )
        .unwrap();
        std::fs::write(
            format!("{out}/e0_sample_journal_head.jsonl"),
            jsonl.lines().take(40).collect::<Vec<_>>().join("\n"),
        )
        .unwrap();
        eprintln!(
            "E0 done: lossless={} state={} starts={}",
            journal == r.journal,
            same_state,
            same_starts
        );
    }

    // ---------------- E1: distribution-aware backfill ---------------------
    let header = "trace,offset,policy,jobs,mean_wait_h,p95_wait_h,mean_bsld,wide_wait_h,utilization,makespan_h,extensions_per_job,unplaced,sim_ms\n";
    let mut csv = String::from(header);
    let mut csv_b = String::from(header);
    let thetas = [0.5, 0.7, 0.9, 0.95];
    for t in traces.iter().filter(|_| run("e1")) {
        for &off in &offsets {
            let w = t.window(off, n);
            let hist = t.history(off, 2000);
            let mut variants: Vec<(String, Arc<dyn Policy>)> = vec![(
                "walltime".into(),
                Arc::new(BackfillPolicy::new(WINDOW_SECS)),
            )];
            for &q in &thetas {
                variants.push((format!("q{q:.2}"), quantile_policy(q, &hist)));
            }
            for cfg in bias_variants() {
                let pol = bias_policy(cfg, &hist);
                let label = pol.name().trim_start_matches("backfill/").to_string();
                variants.push((label, pol));
            }
            let mut shields = Vec::new();
            for (cfg, ahead) in shielded_variants() {
                let pol = shielded_policy(cfg, ahead, &hist);
                let label = pol.name().trim_start_matches("backfill/").to_string();
                shields.push((label.clone(), pol.clone()));
                variants.push((label, pol));
            }
            let oracle = Oracle(w.iter().map(|s| (s.job.id, s.actual)).collect());
            variants.push((
                "oracle".into(),
                Arc::new(BackfillPolicy::with_estimator(
                    WINDOW_SECS,
                    Arc::new(oracle),
                )),
            ));
            for (label, pol) in variants {
                let mut sim = Simulator::new(Machine::single(t.max_procs), pol);
                sim.record = false;
                let t0 = Instant::now();
                let r = sim.run(&w);
                let ms = t0.elapsed().as_millis();
                let s = metrics::summarize(&r, t.max_procs);
                let row = format!(
                    "{},{off},{label},{},{:.4},{:.4},{:.3},{:.4},{:.4},{:.2},{:.3},{},{ms}",
                    t.name,
                    s.jobs,
                    s.mean_wait_h,
                    s.p95_wait_h,
                    s.mean_bsld,
                    s.wide_wait_h,
                    s.utilization,
                    s.makespan_h,
                    s.extensions_per_job,
                    s.unplaced
                );
                writeln!(csv, "{row}").unwrap();
                if ["walltime", "q0.70", "oracle"].contains(&label.as_str())
                    || label.starts_with("bc")
                {
                    writeln!(csv_b, "{row}").unwrap();
                }
                eprintln!(
                    "E1 {} @{off} {label}: wait {:.2}h bsld {:.1} ({ms} ms)",
                    t.name, s.mean_wait_h, s.mean_bsld
                );
                if let Some((_, shield)) = shields.iter().find(|(l, _)| *l == label) {
                    eprintln!("   shield: {:?}", shield.stats());
                }
            }
        }
    }
    if run("e1") {
        std::fs::write(format!("{out}/e1_runtime_quantiles.csv"), &csv)?;
        std::fs::write(format!("{out}/e1b_bias_corrected.csv"), &csv_b)?;
    }

    // ---------------- E2: time-varying capacity ---------------------------
    let mut csv2 = String::from("trace,offset,scenario,planner,jobs,mean_wait_h,mean_bsld,utilization,over_cap_core_h,peak_over_cap_cores,kg_co2,unplaced\n");
    for t in traces.iter().filter(|_| run("e2")) {
        let p = t.max_procs;
        // Demand-response power cap: 60% of cores 16:00-20:00 UTC daily.
        let power = CapacityProfile::daily(vec![
            (0, cores(p)),
            (16 * 3600, cores(p * 6 / 10)),
            (20 * 3600, cores(p)),
        ]);
        let power_floor = p * 6 / 10;
        // Carbon window: 75% of cores during the six dirtiest hours (17:00-23:00).
        let carbon = CapacityProfile::daily(vec![
            (0, cores(p)),
            (17 * 3600, cores(p * 3 / 4)),
            (23 * 3600, cores(p)),
        ]);
        let carbon_floor = p * 3 / 4;
        for &off in &offsets {
            let w = t.window(off, n);
            for (scenario, prof, floor) in [
                ("power-cap-60pct-16to20", &power, power_floor),
                ("carbon-75pct-17to23", &carbon, carbon_floor),
            ] {
                // unaware: planner ignores the cap; static: operator derates the
                // machine to the cap all day (the time-blind way to comply);
                // aware: Gnomon's time-varying capacity.
                for planner in ["unaware", "static", "aware"] {
                    let mut m = Machine::single(if planner == "static" { floor } else { p });
                    if planner == "aware" {
                        m.profile = Some(prof.clone());
                    }
                    let mut sim = Simulator::new(m, Arc::new(BackfillPolicy::new(WINDOW_SECS)));
                    sim.record = false;
                    let r = sim.run(&w);
                    let s = metrics::summarize(&r, p);
                    let e = metrics::energy(&r, prof, p, &CARBON, 10.0);
                    writeln!(
                        csv2,
                        "{},{off},{scenario},{planner},{},{:.4},{:.3},{:.4},{:.1},{},{:.1},{}",
                        t.name,
                        s.jobs,
                        s.mean_wait_h,
                        s.mean_bsld,
                        s.utilization,
                        e.over_cap_core_h,
                        e.peak_over_cap_cores,
                        e.kg_co2,
                        s.unplaced
                    )
                    .unwrap();
                    eprintln!("E2 {} @{off} {scenario} {planner}: over-cap {:.0} core-h, co2 {:.0} kg, wait {:.2}h",
                        t.name, e.over_cap_core_h, e.kg_co2, s.mean_wait_h);
                }
            }
        }
    }
    if run("e2") {
        std::fs::write(format!("{out}/e2_capacity_profiles.csv"), &csv2)?;
    }

    // ---------------- E3: shadow mode -------------------------------------
    if run("e3") {
        let t = &traces[0];
        let w = t.window(offsets[0], n);
        let hist = t.history(offsets[0], 2000);
        let shadow = Arc::new(ShadowPolicy::new(
            Arc::new(BackfillPolicy::new(WINDOW_SECS)),
            quantile_policy(0.9, &hist),
        ));
        let mut sim = Simulator::new(Machine::single(t.max_procs), shadow.clone());
        sim.record = false;
        let r = sim.run(&w);
        let live = Simulator::new(
            Machine::single(t.max_procs),
            Arc::new(BackfillPolicy::new(WINDOW_SECS)),
        );
        let r_live = live.run(&w);
        let identical = r.outcomes == r_live.outcomes;
        let agreement = shadow.agreement();
        let d = shadow.diffs.lock().unwrap().clone();
        let start_now_diffs = d
            .iter()
            .filter(|x| {
                matches!(x.candidate, gnomon_sched::Decision::Start { .. })
                    && !matches!(x.primary, gnomon_sched::Decision::Start { .. })
            })
            .count();
        writeln!(
            md,
            "## E3 — shadow mode ({}, candidate q0.90 alongside live walltime policy)\n",
            t.name
        )
        .unwrap();
        writeln!(md, "| measure | value |\n|---|---|").unwrap();
        writeln!(
            md,
            "| decisions compared | {} |",
            shadow.decisions.load(std::sync::atomic::Ordering::Relaxed)
        )
        .unwrap();
        writeln!(md, "| agreement | {:.1}% |", agreement * 100.0).unwrap();
        writeln!(
            md,
            "| candidate would start a job the live policy deferred/reserved | {start_now_diffs} |"
        )
        .unwrap();
        writeln!(
            md,
            "| live schedule unchanged by shadowing | {identical} |\n"
        )
        .unwrap();
        eprintln!(
            "E3: agreement {:.1}% identical={identical}",
            agreement * 100.0
        );
    }
    // ---------------- E4: candidate priority plugin ----------------------
    if let Some((path, plugin)) = plugin.as_ref().filter(|_| run("e4")) {
        let mut csv4 = String::from("trace,offset,priorities,jobs,mean_wait_h,p95_wait_h,mean_bsld,wide_wait_h,utilization,fcfs_agreement,plugin_failures\n");
        writeln!(md, "## E4 — priority plugin `{path}` vs FCFS\n").unwrap();
        writeln!(md, "| trace @offset | priorities | mean wait h | p95 wait h | mean bsld | wide wait h | FCFS pair agreement | plugin failures |\n|---|---|---|---|---|---|---|---|").unwrap();
        for t in &traces {
            for &off in &offsets {
                let fcfs = t.window(off, n);
                let mut cand = fcfs.clone();
                let mut failures = 0u64;
                // As the controller does: plugin at submit time, FCFS (0) on failure.
                priority::assign(&mut cand, |j, now| {
                    plugin.priority(j, now).unwrap_or_else(|_| {
                        failures += 1;
                        0
                    })
                });
                for (label, w, fails) in [("fcfs", &fcfs, 0), ("plugin", &cand, failures)] {
                    let mut sim = Simulator::new(
                        Machine::single(t.max_procs),
                        Arc::new(BackfillPolicy::new(WINDOW_SECS)),
                    );
                    sim.record = false;
                    let s = metrics::summarize(&sim.run(w), t.max_procs);
                    let agree = priority::fcfs_agreement(w);
                    writeln!(
                        csv4,
                        "{},{off},{label},{},{:.4},{:.4},{:.3},{:.4},{:.4},{agree:.4},{fails}",
                        t.name,
                        s.jobs,
                        s.mean_wait_h,
                        s.p95_wait_h,
                        s.mean_bsld,
                        s.wide_wait_h,
                        s.utilization
                    )
                    .unwrap();
                    writeln!(
                        md,
                        "| {} @{off} | {label} | {:.2} | {:.2} | {:.1} | {:.2} | {:.1}% | {fails} |",
                        t.name, s.mean_wait_h, s.p95_wait_h, s.mean_bsld, s.wide_wait_h, agree * 100.0
                    )
                    .unwrap();
                    eprintln!(
                        "E4 {} @{off} {label}: wait {:.2}h bsld {:.1} agreement {:.1}%",
                        t.name,
                        s.mean_wait_h,
                        s.mean_bsld,
                        agree * 100.0
                    );
                }
            }
        }
        writeln!(md).unwrap();
        std::fs::write(format!("{out}/e4_priority_plugin.csv"), &csv4)?;
    }
    if run("e0") || run("e3") || (plugin.is_some() && run("e4")) {
        std::fs::write(format!("{out}/SUMMARY_raw.md"), md)?;
    }
    Ok(())
}
