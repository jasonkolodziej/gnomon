//! Scheduling metrics over simulation outcomes.

use crate::{Outcome, SimResult};
use gnomon_core::{CapacityProfile, Resources};

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Summary {
    pub policy: String,
    pub jobs: usize,
    pub mean_wait_h: f64,
    pub p95_wait_h: f64,
    /// Mean bounded slowdown, tau = 10 s (Feitelson's standard metric).
    pub mean_bsld: f64,
    /// Mean wait of "wide" jobs (>= 25% of the machine) — guards against
    /// aggressive backfill starving large jobs.
    pub wide_wait_h: f64,
    pub utilization: f64,
    pub makespan_h: f64,
    pub extensions_per_job: f64,
    pub unplaced: usize,
}

fn pct(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[(((v.len() - 1) as f64) * q).round() as usize]
}

pub fn summarize(r: &SimResult, machine_cores: u32) -> Summary {
    let o = &r.outcomes;
    if o.is_empty() {
        return Summary {
            policy: r.policy.clone(),
            ..Default::default()
        };
    }
    let n = o.len() as f64;
    let mut waits: Vec<f64> = o.iter().map(|x| (x.start - x.submit) as f64).collect();
    let mean_wait = waits.iter().sum::<f64>() / n;
    let bsld = o
        .iter()
        .map(|x| {
            let run = (x.end - x.start) as f64;
            (((x.start - x.submit) as f64 + run) / run.max(10.0)).max(1.0)
        })
        .sum::<f64>()
        / n;
    let wide: Vec<&Outcome> = o.iter().filter(|x| x.cores * 4 >= machine_cores).collect();
    let wide_wait = if wide.is_empty() {
        0.0
    } else {
        wide.iter()
            .map(|x| (x.start - x.submit) as f64)
            .sum::<f64>()
            / wide.len() as f64
    };
    let t0 = o.iter().map(|x| x.submit).min().unwrap();
    let t1 = o.iter().map(|x| x.end).max().unwrap();
    let work: f64 = o
        .iter()
        .map(|x| (x.end - x.start) as f64 * x.cores as f64)
        .sum();
    Summary {
        policy: r.policy.clone(),
        jobs: o.len(),
        mean_wait_h: mean_wait / 3600.0,
        p95_wait_h: pct(&mut waits, 0.95) / 3600.0,
        mean_bsld: bsld,
        wide_wait_h: wide_wait / 3600.0,
        utilization: work / ((t1 - t0).max(1) as f64 * machine_cores as f64),
        makespan_h: (t1 - t0) as f64 / 3600.0,
        extensions_per_job: r.extensions as f64 / n,
        unplaced: r.unplaced,
    }
}

/// Core-hours run above a (true) capacity profile, and emissions (kg CO2)
/// under an hourly carbon-intensity curve (g/kWh) at `watts_per_core`.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct EnergySummary {
    pub over_cap_core_h: f64,
    pub peak_over_cap_cores: u32,
    pub kg_co2: f64,
}

pub fn energy(
    r: &SimResult,
    cap: &CapacityProfile,
    base: u32,
    carbon_g_per_kwh: &[f64; 24],
    watts_per_core: f64,
) -> EnergySummary {
    // Step function of usage.
    let mut ev: Vec<(u64, i64)> = Vec::new();
    for x in &r.outcomes {
        ev.push((x.start, x.cores as i64));
        ev.push((x.end, -(x.cores as i64)));
    }
    ev.sort();
    let base_r = Resources {
        cores: base,
        mem_mb: 0,
        gpus: 0,
    };
    let mut pts: Vec<u64> = ev.iter().map(|e| e.0).collect();
    if let (Some(&a), Some(&b)) = (pts.first(), pts.last()) {
        cap.breakpoints(a, b, &mut pts);
        let mut h = (a / 3600 + 1) * 3600;
        while h < b {
            pts.push(h);
            h += 3600;
        }
    }
    pts.sort_unstable();
    pts.dedup();
    let (mut used, mut ei) = (0i64, 0usize);
    let mut out = EnergySummary::default();
    for w in pts.windows(2) {
        while ei < ev.len() && ev[ei].0 <= w[0] {
            used += ev[ei].1;
            ei += 1;
        }
        let dt = (w[1] - w[0]) as f64 / 3600.0;
        let c = cap.at(&base_r, w[0]).cores as i64;
        if used > c {
            out.over_cap_core_h += (used - c) as f64 * dt;
            out.peak_over_cap_cores = out.peak_over_cap_cores.max((used - c) as u32);
        }
        let hour = ((w[0] % 86_400) / 3600) as usize;
        out.kg_co2 += used as f64 * watts_per_core / 1000.0 * dt * carbon_g_per_kwh[hour] / 1000.0;
    }
    out
}
