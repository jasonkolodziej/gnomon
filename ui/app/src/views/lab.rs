//! Policy lab: one synthetic workload under each scheduling policy, run on
//! the server with the production code (`gnomon-sim`), side by side.

use crate::state::State;
use crate::theme::*;
use gnomon_web_model::{LabRequest, LabRow};
use rune::{El, col, el, row};

fn stepper(
    label: &str,
    value: String,
    key: &str,
    dec: fn(&mut LabRequest),
    inc: fn(&mut LabRequest),
) -> El {
    let btn = |k: String, glyph: &str, f: fn(&mut LabRequest)| {
        el().key(k)
            .role("button")
            .label(glyph.to_string())
            .focusable(true)
            .size(26.0, 26.0)
            .center()
            .radius(3.0)
            .border(1.0, line())
            .hover(slate())
            .on_press(move |s: &mut State| f(&mut s.lab))
            .child(t(glyph.to_string(), teal_light()))
    };
    col()
        .gap(6.0)
        .padding(10.0)
        .radius(4.0)
        .fill(panel())
        .border(1.0, line())
        .child(ts(label.to_string(), dim()))
        .child(
            row()
                .center_y()
                .gap(8.0)
                .child(btn(format!("{key}-dec"), "−", dec))
                .child(tb(value, paper()).w(56.0))
                .child(btn(format!("{key}-inc"), "+", inc)),
        )
}

/// One metric as horizontal bars, lower is better unless `higher_better`.
fn metric(
    rows: &[LabRow],
    title: &str,
    unit: &str,
    get: fn(&LabRow) -> f64,
    higher_better: bool,
) -> El {
    let max = rows.iter().map(get).fold(0.0_f64, f64::max).max(1e-9);
    let best = if higher_better {
        rows.iter().map(get).fold(f64::MIN, f64::max)
    } else {
        rows.iter().map(get).fold(f64::MAX, f64::min)
    };
    let mut c = col()
        .w(540.0)
        .gap(5.0)
        .padding(12.0)
        .radius(4.0)
        .fill(panel())
        .border(1.0, line())
        .child(
            row()
                .w_full()
                .child(tb(title.to_string(), paper()).grow(1.0))
                .child(ts(
                    if higher_better {
                        "higher is better"
                    } else {
                        "lower is better"
                    },
                    dim(),
                )),
        );
    for r in rows {
        let v = get(r);
        let is_best = (v - best).abs() < 1e-9;
        c = c.child(
            row()
                .w_full()
                .center_y()
                .gap(8.0)
                .child(
                    ts(
                        r.label.clone(),
                        if is_best { teal_light() } else { muted() },
                    )
                    .w(250.0)
                    .truncate(),
                )
                .child(ts(bar(v, max, 14), if is_best { teal() } else { dim() }))
                .child(ts(
                    format!("{v:.2}{unit}"),
                    if is_best { teal_light() } else { paper() },
                )),
        );
    }
    c
}

pub fn view(s: &State) -> El {
    let q = &s.lab;
    let params = row()
        .gap(10.0)
        .flex_wrap()
        .child(stepper(
            "nodes",
            q.nodes.to_string(),
            "lab-nodes",
            |r| r.nodes = (r.nodes / 2).max(1),
            |r| r.nodes = (r.nodes * 2).min(64),
        ))
        .child(stepper(
            "cores/node",
            q.cores_per_node.to_string(),
            "lab-cores",
            |r| r.cores_per_node = (r.cores_per_node / 2).max(1),
            |r| r.cores_per_node = (r.cores_per_node * 2).min(256),
        ))
        .child(stepper(
            "jobs",
            q.jobs.to_string(),
            "lab-jobs",
            |r| r.jobs = (r.jobs.saturating_sub(200)).max(100),
            |r| r.jobs = (r.jobs + 200).min(3000),
        ))
        .child(stepper(
            "gang every",
            if q.gang_every == 0 {
                "off".into()
            } else {
                q.gang_every.to_string()
            },
            "lab-gang",
            |r| r.gang_every = r.gang_every.saturating_sub(1),
            |r| r.gang_every = (r.gang_every + 1).min(20),
        ))
        .child(stepper(
            "seed",
            q.seed.to_string(),
            "lab-seed",
            |r| r.seed = r.seed.saturating_sub(1),
            |r| r.seed += 1,
        ));
    let run = el()
        .key("lab-run")
        .role("button")
        .label("run lab")
        .focusable(!s.lab_running)
        .h(34.0)
        .px(18.0)
        .center()
        .radius(3.0)
        .fill(if s.lab_running { slate() } else { teal() })
        .on_press(|s: &mut State| {
            if !s.lab_running {
                let req = s.lab.clone();
                crate::net::run_lab(s, req);
            }
        })
        .child(tb(
            if s.lab_running {
                "running…"
            } else {
                "run ▶"
            },
            if s.lab_running { muted() } else { ink() },
        ));
    let mut page = col().w_full().gap(14.0)
        .child(caption("policy lab · same workload, every policy, production scheduling code"))
        .child(ts("synthetic jobs overestimate their walltime (actual 5-100% of it), as users do; every n-th job is a 2-8 node gang", dim()))
        .child(row().w_full().center_y().gap(14.0).child(params).child(run));
    match &s.lab_report {
        None => page = page.child(panel_box().w(640.0).child(t("Run the lab to compare walltime backfill, runtime prediction, the bias-corrected predictor and the wide-job shield.", muted()))),
        Some(rep) => {
            let r = &rep.rows;
            page = page
                .child(ts(
                    format!(
                        "{} jobs on {}×{} cores · seed {} · {} ms",
                        rep.request.jobs, rep.request.nodes, rep.request.cores_per_node, rep.request.seed, rep.millis
                    ),
                    muted(),
                ))
                .child(row().gap(14.0).flex_wrap()
                    .child(metric(r, "mean wait", "h", |x| x.mean_wait_h, false))
                    .child(metric(r, "wide-job wait", "h", |x| x.wide_wait_h, false))
                    .child(metric(r, "p95 wait", "h", |x| x.p95_wait_h, false))
                    .child(metric(r, "bounded slowdown", "", |x| x.mean_bsld, false))
                    .child(metric(r, "utilization", "", |x| x.utilization, true)));
        }
    }
    col()
        .key("lab-scroll")
        .w_full()
        .h_full()
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}
