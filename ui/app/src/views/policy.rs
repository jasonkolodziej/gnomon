//! Scheduling policy and its sandboxed WASM plugins (ADR 0002-0004), drawn
//! as the two pipelines they plug into: submission and planning.

use crate::state::State;
use crate::theme::*;
use gnomon_web_model::PluginView;
use rune::{El, col, el, row};

/// The flag that configures a world, for the "not configured" hint.
fn flag(world: &str) -> &'static str {
    match world {
        "admission" => "--admission-plugin",
        "priority" => "--priority-plugin or --policy-config",
        "shadow-priority" => "--shadow-priority-plugin",
        "estimator" => "--estimator-plugin",
        "node-score" => "--node-score-plugin",
        _ => "",
    }
}

fn what(world: &str) -> &'static str {
    match world {
        "admission" => "accept · reject · rewrite",
        "priority" => "queue order",
        "shadow-priority" => "compared, never applied",
        "estimator" => "runtime estimate",
        "node-score" => "which nodes, never when",
        _ => "",
    }
}

fn find<'a>(plugins: &'a [PluginView], world: &str) -> Option<&'a PluginView> {
    plugins
        .iter()
        .find(|p| p.world == world || (world == "priority" && p.world == "declarative"))
}

fn stage(plugins: &[PluginView], world: &str) -> El {
    let p = find(plugins, world);
    let on = p.is_some();
    let mut b = col()
        .key(format!("stage-{world}"))
        .w(200.0)
        .gap(5.0)
        .padding(10.0)
        .radius(4.0)
        .fill(if on { slate() } else { panel() })
        .border(1.0, if on { teal() } else { line() })
        .child(
            row()
                .w_full()
                .center_y()
                .gap(6.0)
                .child(ts(
                    if on { "◆" } else { "◇" },
                    if on { teal() } else { dim() },
                ))
                .child(tb(world.to_string(), if on { paper() } else { muted() })),
        )
        .child(ts(what(world), dim()));
    match p {
        Some(p) => {
            let file = p.source.rsplit('/').next().unwrap_or(&p.source).to_string();
            b = b.child(ts(file, teal_light()).truncate());
            if p.mode != what(world) {
                b = b.child(ts(p.mode.clone(), muted()).truncate());
            }
            let counters = p
                .counters
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(" · ");
            b = b.child(ts(counters, paper()).max_lines(3));
            let fails = p
                .counters
                .iter()
                .find(|(k, _)| k == "failures")
                .map_or(0, |c| c.1);
            if fails > 0 {
                b = b.child(ts(format!("{fails} failed → fallback used"), amber()));
            }
        }
        None => {
            b = b
                .child(ts("not configured", dim()))
                .child(ts(flag(world), dim()).max_lines(2));
        }
    }
    b
}

fn fixed(label: &str, sub: &str) -> El {
    col()
        .w(160.0)
        .gap(4.0)
        .padding(10.0)
        .radius(4.0)
        .border(1.0, line())
        .child(tb(label.to_string(), paper()))
        .child(ts(sub.to_string(), dim()).w_full().max_lines(3))
}

fn arrow() -> El {
    el().px(6.0).center().child(t("─▶", teal()))
}

pub fn view(s: &State) -> El {
    let v = s.view.as_ref().expect("checked by root");
    let pl = &v.policy.plugins;
    let submit = row()
        .center_y()
        .flex_wrap()
        .gap(4.0)
        .child(fixed("submit", "SubmitJob"))
        .child(arrow())
        .child(stage(pl, "admission"))
        .child(arrow())
        .child(
            col()
                .gap(8.0)
                .child(stage(pl, "priority"))
                .child(stage(pl, "shadow-priority")),
        )
        .child(arrow())
        .child(fixed(
            "JobSubmitted",
            "the prepared job is what the log records",
        ));
    let plan = row()
        .center_y()
        .flex_wrap()
        .gap(4.0)
        .child(fixed("queue", "pending, by priority"))
        .child(arrow())
        .child(stage(pl, "estimator"))
        .child(arrow())
        .child(fixed("backfill", "earliest fit · reservations"))
        .child(arrow())
        .child(stage(pl, "node-score"))
        .child(arrow())
        .child(fixed(
            "JobStarted",
            "plan + read set, revalidated by every replica",
        ));
    let configured = pl.len();
    col().w_full().gap(14.0)
        .child(caption(&format!("policy · {} · {configured} plugin world(s) loaded", v.policy.name)))
        .child(ts("submission — runs once on the controller that takes the request; replicas and replay never call it", dim()))
        .child(submit)
        .child(el().h(6.0))
        .child(ts("planning — runs where plans are built (the leader); the plan event records the result", dim()))
        .child(plan)
        .child(el().h(6.0))
        .child(caption("sandbox"))
        .child(row().gap(12.0).flex_wrap()
            .child(panel_box().w(250.0).child(ts("fuel per call", dim())).child(tb(format!("{}", v.policy.fuel), paper())))
            .child(panel_box().w(250.0).child(ts("memory per call", dim())).child(tb(format!("{} MiB", v.policy.memory_mb), paper())))
            .child(panel_box().w(700.0)
                .child(ts("isolation", dim()))
                .child(t("WASM components · fresh store per call · imports: types only", paper()))
                .child(ts("a trap, fuel or memory overrun counts as a failure and the declarative fallback decides", dim()))))
}
