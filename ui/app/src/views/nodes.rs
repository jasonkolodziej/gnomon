//! One card per node agent: liveness, capacity in use (from the running
//! jobs placed there), the ranks it runs, and ranks on the kill channel.

use crate::state::State;
use crate::theme::*;
use rune::{El, col, el, row};

pub fn view(s: &State) -> El {
    let v = s.view.as_ref().expect("checked by root");
    let up = v.nodes.iter().filter(|n| n.alive).count();
    let mut grid = row().w_full().gap(12.0).flex_wrap();
    for n in &v.nodes {
        let used = s.used_cores(&n.name);
        let jobs: Vec<_> = v
            .jobs
            .iter()
            .filter(|j| j.state == "RUNNING" && j.nodes.iter().any(|m| *m == n.name))
            .collect();
        let gpus_used: u32 = jobs.iter().map(|j| j.gpus).sum();
        let mem_used: u64 = jobs.iter().map(|j| j.mem_mb).sum();
        let status = if n.alive {
            ts(
                format!(
                    "● up · hb {}s ago",
                    s.now().saturating_sub(n.last_heartbeat)
                ),
                teal(),
            )
        } else {
            ts(
                format!(
                    "○ lost · {}s without heartbeat",
                    s.now().saturating_sub(n.last_heartbeat)
                ),
                red(),
            )
        };
        let meter = |label: &str, used: f64, cap: f64, unit: &str| {
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(ts(label.to_string(), dim()).w(40.0))
                .child(ts(
                    bar(used, cap, 16),
                    if used >= cap && cap > 0.0 {
                        amber()
                    } else {
                        teal()
                    },
                ))
                .child(ts(format!("{used:.0}/{cap:.0}{unit}"), muted()))
        };
        let mut card = col()
            .key(format!("node-{}", n.name))
            .w(330.0)
            .gap(8.0)
            .padding(12.0)
            .radius(4.0)
            .fill(panel())
            .border(1.0, if n.alive { line() } else { red().alpha(0.6) })
            .child(
                row()
                    .w_full()
                    .center_y()
                    .child(tb(n.name.clone(), if n.alive { paper() } else { muted() }).grow(1.0))
                    .child(status),
            )
            .child(meter("cpu", f64::from(used), f64::from(n.cores), ""))
            .child(meter(
                "mem",
                mem_used as f64 / 1024.0,
                n.mem_mb as f64 / 1024.0,
                "G",
            ));
        if n.gpus > 0 {
            card = card.child(meter("gpu", f64::from(gpus_used), f64::from(n.gpus), ""));
        }
        card = card.child(el().w_full().h(1.0).fill(line()));
        if jobs.is_empty() {
            card = card.child(ts("idle", dim()));
        }
        for j in jobs.iter().take(5) {
            let rank = j.nodes.iter().position(|m| *m == n.name).unwrap_or(0);
            let id = j.id;
            card = card.child(
                row()
                    .key(format!("node-{}-job-{id}", n.name))
                    .w_full()
                    .gap(8.0)
                    .hover(slate())
                    .on_press(move |s: &mut State| {
                        s.selected = Some(id);
                        s.tab = crate::state::Tab::Queue;
                    })
                    .child(ts("▸", teal()))
                    .child(
                        ts(format!("#{} {}", j.id, j.name), paper())
                            .grow(1.0)
                            .truncate(),
                    )
                    .child(ts(
                        if j.num_nodes > 1 {
                            format!("rank {rank}/{}", j.num_nodes)
                        } else {
                            format!("{}c", j.cores)
                        },
                        muted(),
                    )),
            );
        }
        for k in &n.kill_pending {
            card = card.child(row().gap(8.0).child(ts("✕", red())).child(ts(
                format!("job #{k}: rank being stopped (kill channel)"),
                red(),
            )));
        }
        grid = grid.child(card);
    }
    col()
        .w_full()
        .h_full()
        .gap(12.0)
        .child(caption(&format!("nodes · {up}/{} up", v.nodes.len())))
        .child(
            col()
                .key("nodes-scroll")
                .w_full()
                .grow(1.0)
                .shrink()
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(grid),
        )
}
