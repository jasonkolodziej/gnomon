//! Replay regression gate: every scheduling change is measured on the same workloads with the
//! production code path (`Simulator` → `backfill_pass` → FSM) and compared with a committed
//! baseline (`results/replay_baseline.json`).
//!
//! usage: `replay_gate [--traces DIR] [--baseline FILE] [--write-baseline] [--tolerance FRAC] [--summary FILE]`
//!
//! For each scenario it records the schedule quality of the default policy (mean / p95 wait,
//! bounded slowdown, wide-job wait, utilization, jobs never placed) and proves the run replays
//! exactly: the journal survives a JSONL round trip, rebuilding it gives an identical state, and
//! re-simulating the recovered history reproduces every start time.
//!
//! Exit status: 0 = no metric regressed by more than `--tolerance` (relative, default 0.01) and every
//! replay check holds; 1 = regression or replay mismatch; 2 = usage or I/O error. Scenarios missing
//! from the baseline are reported as new, not failures; SWF scenarios are skipped (and reported) when
//! `--traces` is not given. After an intended behaviour change, rerun with `--write-baseline` and
//! commit the new baseline so the diff shows the measured effect.

use gnomon_core::{Job, JobId, Resources, Walltime};
use gnomon_sched::{BackfillPolicy, Policy};
use gnomon_sim::{metrics, replay, swf, Machine, SimJob, Simulator};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

const WINDOW_SECS: u64 = 72 * 3600;
const DEFAULT_BASELINE: &str = "results/replay_baseline.json";

/// Measured outcome of one scenario. Lower is better for waits and slowdown, higher for
/// utilization; `unplaced` must not grow and both replay checks must hold.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Measure {
    jobs: usize,
    mean_wait_h: f64,
    p95_wait_h: f64,
    mean_bsld: f64,
    wide_wait_h: f64,
    utilization: f64,
    unplaced: usize,
    replay_exact: bool,
    starts_identical: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Baseline {
    scenarios: BTreeMap<String, Measure>,
}

struct Scenario {
    name: String,
    machine: Machine,
    workload: Vec<SimJob>,
}

struct Args {
    traces: Option<String>,
    baseline: String,
    write: bool,
    tolerance: f64,
    summary: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        traces: None,
        baseline: DEFAULT_BASELINE.into(),
        write: false,
        tolerance: 0.01,
        summary: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| it.next().ok_or(format!("{flag} needs a value"));
        match arg.as_str() {
            "--traces" => a.traces = Some(value("--traces")?),
            "--baseline" => a.baseline = value("--baseline")?,
            "--write-baseline" => a.write = true,
            "--tolerance" => {
                a.tolerance = value("--tolerance")?
                    .parse()
                    .map_err(|e| format!("--tolerance: {e}"))?
            }
            "--summary" => a.summary = Some(value("--summary")?),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}

/// Deterministic xorshift64 stream: the synthetic workloads must be identical on every machine.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// `n` jobs arriving every `gap` seconds with heavily over-estimated walltimes (the common case
/// on real systems), up to `max_cores` cores each, from five users. Mean runtime is about 4,000 s,
/// so offered load ≈ (max_cores / 2) × 4000 / (gap × machine cores); the scenarios use ~85%.
fn synthetic(seed: u64, n: u64, gap: u64, max_cores: u32) -> Vec<SimJob> {
    let mut r = Rng(seed);
    (0..n)
        .map(|i| {
            let wall = 600 + r.next() % 14_400;
            let actual = (wall * (5 + r.next() % 95) / 100).max(1);
            let cores = 1 + (r.next() % u64::from(max_cores)) as u32;
            let mut job = Job::new(
                JobId(i + 1),
                "synthetic",
                &format!("u{}", r.next() % 5),
                Resources {
                    cores,
                    mem_mb: 0,
                    gpus: 0,
                },
                Walltime(Duration::from_secs(wall)),
            );
            job.submit_time = 10_000 + i * gap;
            SimJob { job, actual }
        })
        .collect()
}

fn cluster(nodes: usize, cores: u32) -> Machine {
    Machine {
        nodes: (0..nodes)
            .map(|i| {
                (
                    format!("node{i}"),
                    Resources {
                        cores,
                        mem_mb: 0,
                        gpus: 0,
                    },
                )
            })
            .collect(),
        profile: None,
    }
}

fn scenarios(traces: Option<&str>, skipped: &mut Vec<String>) -> Result<Vec<Scenario>, String> {
    let mut out = vec![
        Scenario {
            name: "synthetic/single-128".into(),
            machine: Machine::single(128),
            workload: synthetic(42, 3000, 1200, 64),
        },
        Scenario {
            name: "synthetic/cluster-16x16".into(),
            machine: cluster(16, 16),
            workload: synthetic(7, 3000, 160, 16),
        },
    ];
    let swf = [
        ("SDSC-SP2", "SDSC-SP2-1998-4.2-cln.swf"),
        ("CTC-SP2", "CTC-SP2-1996-3.1-cln.swf"),
        ("HPC2N-2002", "HPC2N-2002-2.2-cln.swf"),
    ];
    for (name, file) in swf {
        let scenario = format!("swf/{name}@10000x5000");
        let Some(dir) = traces else {
            skipped.push(scenario);
            continue;
        };
        let path = format!("{dir}/{file}");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
        let t = swf::parse(name, &text);
        out.push(Scenario {
            name: scenario,
            machine: Machine::single(t.max_procs),
            workload: t.window(10_000, 5000),
        });
    }
    Ok(out)
}

fn measure(s: &Scenario) -> Result<Measure, String> {
    let policy: Arc<dyn Policy> = Arc::new(BackfillPolicy::new(WINDOW_SECS));
    let sim = Simulator::new(s.machine.clone(), policy);
    let r = sim.run(&s.workload);
    let summary = metrics::summarize(&r, s.machine.total_cores());

    let journal = replay::from_jsonl(&replay::to_jsonl(&r.journal)).map_err(|e| e.to_string())?;
    let replay_exact = journal == r.journal
        && replay::fingerprint(&replay::rebuild(&journal))
            == replay::fingerprint(&replay::rebuild(&r.journal));
    let again = sim.run(&replay::workload_from_journal(&journal));
    let starts_identical = r.outcomes.len() == again.outcomes.len()
        && r.outcomes
            .iter()
            .zip(&again.outcomes)
            .all(|(a, b)| a.id == b.id && a.start == b.start);

    Ok(Measure {
        jobs: summary.jobs,
        mean_wait_h: summary.mean_wait_h,
        p95_wait_h: summary.p95_wait_h,
        mean_bsld: summary.mean_bsld,
        wide_wait_h: summary.wide_wait_h,
        utilization: summary.utilization,
        unplaced: summary.unplaced,
        replay_exact,
        starts_identical,
    })
}

/// Relative change of `new` against `old` (0 when both are ~0).
fn rel(old: f64, new: f64) -> f64 {
    if old.abs() < 1e-12 {
        if new.abs() < 1e-12 {
            0.0
        } else {
            f64::INFINITY
        }
    } else {
        (new - old) / old.abs()
    }
}

/// Regressions of `new` against `old`, as human-readable reasons.
fn regressions(old: &Measure, new: &Measure, tol: f64) -> Vec<String> {
    let mut bad = Vec::new();
    let lower_better = [
        ("mean wait", old.mean_wait_h, new.mean_wait_h),
        ("p95 wait", old.p95_wait_h, new.p95_wait_h),
        ("bounded slowdown", old.mean_bsld, new.mean_bsld),
        ("wide-job wait", old.wide_wait_h, new.wide_wait_h),
    ];
    for (what, o, n) in lower_better {
        let d = rel(o, n);
        if d > tol {
            bad.push(format!("{what} {o:.4} → {n:.4} (+{:.2}%)", d * 100.0));
        }
    }
    let d = rel(old.utilization, new.utilization);
    if d < -tol {
        bad.push(format!(
            "utilization {:.4} → {:.4} ({:.2}%)",
            old.utilization,
            new.utilization,
            d * 100.0
        ));
    }
    if new.unplaced > old.unplaced {
        bad.push(format!(
            "jobs never placed {} → {}",
            old.unplaced, new.unplaced
        ));
    }
    if !new.replay_exact {
        bad.push("journal replay is not exact".into());
    }
    if !new.starts_identical {
        bad.push("recovered history does not reproduce start times".into());
    }
    bad
}

fn pct_cell(old: Option<f64>, new: f64) -> String {
    match old {
        Some(o) => {
            let d = rel(o, new);
            if d.abs() < 1e-9 {
                format!("{new:.3}")
            } else {
                format!("{new:.3} ({:+.1}%)", d * 100.0)
            }
        }
        None => format!("{new:.3}"),
    }
}

fn report(
    results: &[(String, Measure)],
    base: &Baseline,
    tol: f64,
    skipped: &[String],
) -> (String, bool) {
    let mut md = String::from("## Replay gate\n\n");
    let _ = writeln!(
        md,
        "Default policy (walltime backfill), production scheduling path; Δ vs `{DEFAULT_BASELINE}`, \
         tolerance {:.1}%.\n",
        tol * 100.0
    );
    md.push_str("| scenario | jobs | mean wait (h) | p95 wait (h) | bsld | wide wait (h) | util | replay | verdict |\n");
    md.push_str("|---|---|---|---|---|---|---|---|---|\n");
    let mut ok = true;
    let mut reasons = String::new();
    for (name, m) in results {
        let old = base.scenarios.get(name);
        let verdict = match old {
            None if m.replay_exact && m.starts_identical => "new".to_string(),
            None => "**replay mismatch**".to_string(),
            Some(o) => {
                let bad = regressions(o, m, tol);
                if bad.is_empty() {
                    "ok".into()
                } else {
                    ok = false;
                    let _ = writeln!(reasons, "- `{name}`: {}", bad.join("; "));
                    "**regressed**".into()
                }
            }
        };
        if old.is_none() && !(m.replay_exact && m.starts_identical) {
            ok = false;
        }
        let _ = writeln!(
            md,
            "| {name} | {} | {} | {} | {} | {} | {} | {} | {verdict} |",
            m.jobs,
            pct_cell(old.map(|o| o.mean_wait_h), m.mean_wait_h),
            pct_cell(old.map(|o| o.p95_wait_h), m.p95_wait_h),
            pct_cell(old.map(|o| o.mean_bsld), m.mean_bsld),
            pct_cell(old.map(|o| o.wide_wait_h), m.wide_wait_h),
            pct_cell(old.map(|o| o.utilization), m.utilization),
            if m.replay_exact && m.starts_identical {
                "exact"
            } else {
                "MISMATCH"
            },
        );
    }
    if !reasons.is_empty() {
        let _ = write!(md, "\nRegressions:\n{reasons}");
    }
    if !skipped.is_empty() {
        let _ = writeln!(md, "\nSkipped (no `--traces`): {}", skipped.join(", "));
    }
    (md, ok)
}

fn run() -> Result<bool, String> {
    let args = parse_args().map_err(|e| {
        if e.is_empty() {
            "usage: replay_gate [--traces DIR] [--baseline FILE] [--write-baseline] [--tolerance FRAC] [--summary FILE]".to_string()
        } else {
            e
        }
    })?;
    let mut skipped = Vec::new();
    let mut results = Vec::new();
    for s in scenarios(args.traces.as_deref(), &mut skipped)? {
        eprintln!("replay_gate: {} ({} jobs)", s.name, s.workload.len());
        results.push((s.name.clone(), measure(&s)?));
    }

    let base: Baseline = match std::fs::read_to_string(&args.baseline) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", args.baseline))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Baseline::default(),
        Err(e) => return Err(format!("{}: {e}", args.baseline)),
    };
    let (md, ok) = report(&results, &base, args.tolerance, &skipped);
    println!("{md}");
    if let Some(path) = &args.summary {
        let mut existing = std::fs::read_to_string(path).unwrap_or_default();
        existing.push_str(&md);
        std::fs::write(path, existing).map_err(|e| format!("{path}: {e}"))?;
    }
    if args.write {
        // Keep baseline entries for scenarios not measured in this run (e.g. SWF without traces).
        let mut merged = base;
        for (name, m) in results {
            merged.scenarios.insert(name, m);
        }
        let json = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;
        std::fs::write(&args.baseline, json + "\n")
            .map_err(|e| format!("{}: {e}", args.baseline))?;
        eprintln!("replay_gate: wrote {}", args.baseline);
        return Ok(true);
    }
    Ok(ok)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("replay_gate: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(wait: f64, util: f64) -> Measure {
        Measure {
            jobs: 10,
            mean_wait_h: wait,
            p95_wait_h: wait * 3.0,
            mean_bsld: 1.0 + wait,
            wide_wait_h: wait * 2.0,
            utilization: util,
            unplaced: 0,
            replay_exact: true,
            starts_identical: true,
        }
    }

    #[test]
    fn identical_measures_do_not_regress() {
        assert!(regressions(&m(2.0, 0.8), &m(2.0, 0.8), 0.0).is_empty());
    }

    #[test]
    fn worse_wait_beyond_tolerance_regresses() {
        let bad = regressions(&m(2.0, 0.8), &m(2.1, 0.8), 0.01);
        assert!(bad.iter().any(|r| r.starts_with("mean wait")), "{bad:?}");
        assert!(regressions(&m(2.0, 0.8), &m(2.01, 0.8), 0.01).is_empty());
    }

    #[test]
    fn improvements_and_lower_utilization_are_judged_by_direction() {
        assert!(regressions(&m(2.0, 0.8), &m(1.0, 0.8), 0.01).is_empty());
        let bad = regressions(&m(2.0, 0.8), &m(2.0, 0.7), 0.01);
        assert!(bad.iter().any(|r| r.starts_with("utilization")), "{bad:?}");
    }

    #[test]
    fn replay_mismatch_and_unplaced_growth_always_fail() {
        let mut new = m(2.0, 0.8);
        new.replay_exact = false;
        new.unplaced = 1;
        let bad = regressions(&m(2.0, 0.8), &new, 1.0);
        assert_eq!(bad.len(), 2, "{bad:?}");
    }

    #[test]
    fn zero_baseline_metric_handles_growth() {
        assert_eq!(rel(0.0, 0.0), 0.0);
        assert!(rel(0.0, 1.0).is_infinite());
    }

    #[test]
    fn report_marks_new_and_regressed_scenarios() {
        let mut base = Baseline::default();
        base.scenarios.insert("a".into(), m(1.0, 0.8));
        let results = vec![("a".into(), m(2.0, 0.8)), ("b".into(), m(1.0, 0.8))];
        let (md, ok) = report(&results, &base, 0.01, &["swf/x".into()]);
        assert!(!ok);
        assert!(md.contains("| a |") && md.contains("**regressed**"), "{md}");
        assert!(md.contains("| b |") && md.contains("| new |"), "{md}");
        assert!(md.contains("Skipped (no `--traces`): swf/x"), "{md}");
    }

    #[test]
    fn synthetic_workloads_are_deterministic() {
        assert_eq!(synthetic(42, 50, 90, 64), synthetic(42, 50, 90, 64));
        assert_ne!(synthetic(42, 50, 90, 64), synthetic(7, 50, 90, 64));
    }

    #[test]
    fn small_scenario_replays_exactly() {
        let s = Scenario {
            name: "t".into(),
            machine: cluster(4, 8),
            workload: synthetic(3, 120, 60, 8),
        };
        let r = measure(&s).unwrap();
        assert!(r.replay_exact && r.starts_identical);
        assert_eq!(r.jobs, 120);
        assert_eq!(r.unplaced, 0);
    }
}
