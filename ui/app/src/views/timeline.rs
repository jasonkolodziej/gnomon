//! The plan on the time axis: one lane per node, running spans and backfill
//! reservations, the now-line, and the queue still waiting for a place.

use super::{job_detail, job_line};
use crate::state::State;
use crate::theme::*;
use rune::{Cx, El, col, el, row};

const LABEL_W: f32 = 150.0;
const LANE_H: f32 = 30.0;
const WINDOWS: [(u64, &str); 4] = [
    (900, "15m"),
    (3600, "1h"),
    (6 * 3600, "6h"),
    (24 * 3600, "24h"),
];

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let v = s.view.as_ref().expect("checked by root");
    let width = cx.layout_size("timeline").map_or(1100.0, |[w, _]| w);
    let track_w = (width - LABEL_W).max(200.0);
    let now = s.now();
    // A quarter of the window behind now, the rest ahead: the plan is the point.
    let t0 = now.saturating_sub(s.window / 4);
    let t1 = t0 + s.window;
    let x =
        |t: u64| ((t.saturating_sub(t0)) as f32 / (t1 - t0) as f32 * track_w).clamp(0.0, track_w);

    let mut toolbar = row()
        .w_full()
        .center_y()
        .gap(6.0)
        .child(caption("plan · per-node planner spans"));
    for (secs, label) in WINDOWS {
        let on = s.window == secs;
        toolbar = toolbar.child(
            el().key(format!("win-{label}"))
                .role("button")
                .label(format!("window {label}"))
                .focusable(true)
                .px(9.0)
                .py(3.0)
                .radius(3.0)
                .border(1.0, if on { teal() } else { line() })
                .fill(if on { slate() } else { panel() })
                .hover(slate())
                .on_press(move |s: &mut State| s.window = secs)
                .child(ts(label, if on { teal_light() } else { muted() })),
        );
    }

    // Axis: ticks every sixth of the window.
    let mut axis = el().w(track_w).h(18.0);
    for i in 0..=6u64 {
        let tt = t0 + s.window * i / 6;
        axis = axis
            .child(ts(hhmm(tt), dim()).absolute((x(tt) - 18.0).clamp(0.0, track_w - 40.0), 0.0));
    }

    let mut lanes = col().w_full().gap(4.0);
    for n in &v.nodes {
        let mut track = el()
            .key(format!("lane-{}", n.name))
            .w(track_w)
            .h(LANE_H)
            .radius(3.0)
            .fill(panel())
            .border(1.0, if n.alive { line() } else { red().alpha(0.5) })
            .clip_rounded(3.0);
        for j in v
            .jobs
            .iter()
            .filter(|j| j.start > 0 && j.nodes.iter().any(|m| *m == n.name))
        {
            let end = j.start + j.span.max(1);
            if end < t0 || j.start > t1 {
                continue;
            }
            let (a, b) = (x(j.start), x(end));
            let w = (b - a).max(3.0);
            let color = state_color(&j.state);
            let selected = s.selected == Some(j.id);
            let rank = j.nodes.iter().position(|m| *m == n.name).unwrap_or(0);
            let label = if j.num_nodes > 1 {
                format!("{} r{}", job_line(j), rank)
            } else {
                job_line(j)
            };
            let reserved = j.state == "RESERVED";
            let finished = !j.is_active();
            let id = j.id;
            let mut span = el()
                .key(format!("span-{}-{}", j.id, n.name))
                .absolute(a, 3.0)
                .size(w, LANE_H - 6.0)
                .radius(2.0)
                .px(6.0)
                .center_y()
                .fill(if reserved {
                    color.alpha(0.18)
                } else if finished {
                    color.alpha(0.35)
                } else {
                    color.alpha(0.88)
                })
                .border(
                    if selected { 2.0 } else { 1.0 },
                    if selected { paper() } else { color },
                )
                .on_press(move |s: &mut State| s.selected = Some(id))
                .clip_rounded(2.0);
            if w > 40.0 {
                span = span.child(
                    ts(label, if reserved || finished { paper() } else { ink() }).truncate(),
                );
            }
            track = track.child(span);
        }
        if n.alive {
            track = track.child(
                el().absolute(x(now) - 1.0, 0.0)
                    .size(2.0, LANE_H)
                    .fill(teal_light()),
            );
        }
        let mut name = row()
            .w(LABEL_W)
            .center_y()
            .gap(6.0)
            .child(t(
                if n.alive { "●" } else { "○" },
                if n.alive { teal() } else { red() },
            ))
            .child(t(n.name.clone(), if n.alive { paper() } else { muted() }));
        if !n.kill_pending.is_empty() {
            name = name.child(ts(format!("✕{}", n.kill_pending.len()), red()));
        }
        if !n.alive {
            name = name.child(ts("lost", red()));
        }
        lanes = lanes.child(row().w_full().center_y().child(name).child(track));
    }

    let pending: Vec<_> = s
        .queue()
        .into_iter()
        .filter(|j| j.state == "PENDING")
        .collect();
    let mut queue = row().w_full().gap(6.0).flex_wrap();
    if pending.is_empty() {
        queue = queue.child(ts("nothing waiting for a place", dim()));
    }
    for j in pending.iter().take(40) {
        let id = j.id;
        queue = queue.child(
            el().key(format!("pend-{id}"))
                .px(8.0)
                .py(4.0)
                .radius(3.0)
                .border(1.0, sky().alpha(0.7))
                .hover(slate())
                .on_press(move |s: &mut State| s.selected = Some(id))
                .child(ts(job_line(j), sky())),
        );
    }

    let legend = row()
        .gap(14.0)
        .child(legend_item("running", teal(), false))
        .child(legend_item("reserved (backfill plan)", amber(), true))
        .child(legend_item("failed", red(), false))
        .child(legend_item("done", dim(), false))
        .child(
            row()
                .center_y()
                .gap(5.0)
                .child(el().size(2.0, 12.0).fill(teal_light()))
                .child(ts("now", muted())),
        );

    let mut page = col()
        .key("timeline")
        .w_full()
        .gap(10.0)
        .child(toolbar)
        .child(row().w_full().child(el().w(LABEL_W)).child(axis))
        .child(
            col()
                .key("lanes-scroll")
                .w_full()
                .max_h(560.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(lanes),
        )
        .child(legend)
        .child(caption(&format!("queue · {} pending", pending.len())))
        .child(queue);
    if let Some(id) = s.selected {
        page = page.child(row().w_full().child(job_detail(s, id).w(520.0)));
    }
    page
}

fn legend_item(label: &str, c: rune::Color, outlined: bool) -> El {
    row()
        .center_y()
        .gap(5.0)
        .child(
            el().size(18.0, 10.0)
                .radius(2.0)
                .fill(if outlined { c.alpha(0.18) } else { c })
                .border(1.0, c),
        )
        .child(ts(label, muted()))
}
