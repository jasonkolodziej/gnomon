//! The controller group: this replica's Raft state and how far each member
//! has replicated the log.

use crate::state::State;
use crate::theme::*;
use rune::{El, col, el, row};

pub fn view(s: &State) -> El {
    let v = s.view.as_ref().expect("checked by root");
    let c = &v.cluster;
    let stat = |k: &str, val: String, color: rune::Color| {
        col()
            .gap(4.0)
            .padding(12.0)
            .w(180.0)
            .radius(4.0)
            .fill(panel())
            .border(1.0, line())
            .child(ts(k.to_string(), dim()))
            .child(tb(val, color))
    };
    if !c.replicated {
        return col().w_full().gap(14.0)
            .child(caption("controller · standalone"))
            .child(row().gap(12.0)
                .child(stat("mode", "standalone".into(), paper()))
                .child(stat("state index", c.fsm_index.to_string(), paper())))
            .child(panel_box().w(720.0)
                .child(t("This controller runs without Raft: one process, its state on disk with --state-dir.", muted()))
                .child(t("Run three as one fault-tolerant group:", muted()))
                .child(t("gnomon-ctl --raft-id 1 --raft-peers 1=ctl1:7700,2=ctl2:7700,3=ctl3:7700 --state-dir DIR", teal_light()))
                .child(ts("followers forward writes to the leader · only the leader schedules · agents fail over via GNOMON_ADDR", dim())));
    }
    let leader = c
        .leader
        .map_or("none (election)".into(), |l| format!("node {l}"));
    let mut table = col()
        .w(760.0)
        .fill(panel())
        .border(1.0, line())
        .radius(4.0)
        .child(
            row()
                .h(26.0)
                .px(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], line())
                .child(ts("ID", dim()).w(50.0))
                .child(ts("ADDRESS", dim()).w(180.0))
                .child(ts("ROLE", dim()).w(110.0))
                .child(ts("REPLICATED (leader's view)", dim()).grow(1.0)),
        );
    let top = c
        .members
        .iter()
        .filter_map(|m| m.matched)
        .max()
        .unwrap_or(c.last_log_index)
        .max(1);
    for m in &c.members {
        let role = if m.leader {
            ("◈ leader", teal_light())
        } else if m.id == c.node_id {
            ("this replica", paper())
        } else {
            ("follower", muted())
        };
        let progress = match m.matched {
            Some(i) => format!(
                "{} {i} (lag {})",
                bar(i as f64, top as f64, 18),
                top - i.min(top)
            ),
            None if c.role == "leader" => "unknown".into(),
            None => "only the leader tracks this".into(),
        };
        table = table.child(
            row()
                .key(format!("member-{}", m.id))
                .h(30.0)
                .px(12.0)
                .center_y()
                .fill(if m.id == c.node_id { slate() } else { panel() })
                .child(t(m.id.to_string(), paper()).w(50.0))
                .child(t(m.addr.clone(), muted()).w(180.0))
                .child(t(role.0, role.1).w(110.0))
                .child(ts(progress, if m.matched.is_some() { teal() } else { dim() }).grow(1.0)),
        );
    }
    col().w_full().gap(14.0)
        .child(caption(&format!("controller group · raft · {} members", c.members.len())))
        .child(row().gap(12.0).flex_wrap()
            .child(stat("this replica", format!("node {} · {}", c.node_id, c.role), if c.role == "leader" { teal_light() } else { paper() }))
            .child(stat("leader", leader, teal_light()))
            .child(stat("term", c.term.to_string(), paper()))
            .child(stat("log / applied", format!("{} / {}", c.last_log_index, c.last_applied), paper()))
            .child(stat("fsm index", c.fsm_index.to_string(), paper())))
        .child(table)
        .child(el().h(4.0))
        .child(ts("every write commits through the log · plan read sets travel with it, so all replicas accept or reject a plan alike · ADR 0001", dim()))
}
