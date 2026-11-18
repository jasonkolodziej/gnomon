//! The state machine's event feed, newest first: what the controller
//! decided, as it decides it.

use crate::state::State;
use crate::theme::*;
use rune::{El, col, el, row};

/// Events that drown the feed when shown (every agent poll).
fn noise(kind: &str) -> bool {
    matches!(kind, "Heartbeat" | "AssignmentsFetched")
}

fn kind_color(kind: &str) -> rune::Color {
    match kind {
        "JobSubmitted" => sky(),
        "JobStarted" => teal(),
        "JobReserved" | "ReservationsCleared" | "JobExtended" => amber(),
        "JobFinished" | "JobRankFinished" => muted(),
        "JobCancelled" => violet(),
        "NodeLost" => red(),
        "NodeRegistered" | "NodeProfileSet" => teal_light(),
        _ => dim(),
    }
}

pub fn view(s: &State) -> El {
    let shown: Vec<_> = s
        .events
        .iter()
        .rev()
        .filter(|e| !(s.hide_noise && noise(&e.kind)))
        .collect();
    let toggle = el()
        .key("noise")
        .role("switch")
        .label("hide heartbeats and fetches")
        .checked(s.hide_noise)
        .focusable(true)
        .px(9.0)
        .py(3.0)
        .radius(3.0)
        .border(1.0, if s.hide_noise { teal() } else { line() })
        .hover(slate())
        .on_press(|s: &mut State| s.hide_noise = !s.hide_noise)
        .child(ts(
            if s.hide_noise {
                "[✓] hide heartbeats & fetches"
            } else {
                "[ ] hide heartbeats & fetches"
            },
            muted(),
        ));
    let mut list = col().w_full();
    for e in shown.iter().take(300) {
        let job = e.job.map_or(String::new(), |j| format!("#{j}"));
        let id = e.job;
        let mut r = row()
            .key(format!("ev-{}", e.index))
            .w_full()
            .h(24.0)
            .px(10.0)
            .center_y()
            .gap(10.0)
            .hover(slate())
            .child(ts(format!("{:>6}", e.index), dim()).w(56.0))
            .child(ts(clock(e.at), muted()).w(64.0))
            .child(ts(e.kind.clone(), kind_color(&e.kind)).w(150.0))
            .child(ts(job, paper()).w(48.0))
            .child(ts(e.node.clone(), muted()).w(70.0))
            .child(
                ts(e.detail.clone(), if e.accepted { paper() } else { dim() })
                    .grow(1.0)
                    .truncate(),
            );
        if !e.accepted {
            r = r.child(ts("✗ no effect", amber()));
        }
        if let Some(id) = id {
            r = r.on_press(move |s: &mut State| {
                s.selected = Some(id);
                s.tab = crate::state::Tab::Queue;
            });
        }
        list = list.child(r);
    }
    if shown.is_empty() {
        list = list.child(row().padding(12.0).child(ts("no events yet", dim())));
    }
    col()
        .w_full()
        .h_full()
        .gap(10.0)
        .child(
            row()
                .w_full()
                .center_y()
                .gap(10.0)
                .child(caption(&format!(
                    "events · {} kept · newest first",
                    s.events.len()
                )))
                .child(toggle),
        )
        .child(
            col()
                .key("events-scroll")
                .w_full()
                .grow(1.0)
                .shrink()
                .fill(panel())
                .border(1.0, line())
                .radius(4.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
}
