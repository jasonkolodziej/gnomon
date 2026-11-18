//! The screens. Each is a pure function of the state, as Rune views are.

mod cluster;
mod events;
mod lab;
mod nodes;
mod policy;
mod queue;
mod timeline;

use crate::state::{State, Tab};
use crate::theme::*;
use rune::{Cx, El, col, el, row, text};

pub fn root(s: &State, cx: &mut Cx<State>) -> El {
    let body = match s.view.as_ref() {
        None => waiting(),
        Some(_) => match s.tab {
            Tab::Timeline => timeline::view(s, cx),
            Tab::Nodes => nodes::view(s),
            Tab::Queue => queue::view(s),
            Tab::Cluster => cluster::view(s),
            Tab::Policy => policy::view(s),
            Tab::Events => events::view(s),
            Tab::Lab => lab::view(s),
        },
    };
    let mut page = col()
        .key("page")
        .w_full()
        .h_full()
        .fill(ink())
        .child(header(s))
        .child(tabs(s))
        .child(
            col()
                .key("body")
                .w_full()
                .grow(1.0)
                .shrink()
                .padding(16.0)
                .child(body),
        )
        .child(footer(s));
    if let Some((msg, ok)) = &s.toast {
        page = page.child(toast(msg, *ok));
    }
    page
}

fn waiting() -> El {
    col()
        .w_full()
        .grow(1.0)
        .center()
        .gap(10.0)
        .child(
            row()
                .center_y()
                .gap(2.0)
                .child(text("▍").font(mono_b(26.0)).color(teal()))
                .child(text("gnomon").font(wordmark(28.0)).color(paper())),
        )
        .child(t("connecting to gnomon-web…", muted()))
        .child(ts("append ?demo to the URL to explore sample data", dim()))
}

fn header(s: &State) -> El {
    let v = s.view.as_ref();
    let (dot, status, color) = match (s.demo, v.map(|v| v.connected), s.live) {
        (true, _, _) => ("◆", "demo data".to_string(), amber()),
        (_, Some(true), true) => ("●", "live".to_string(), teal()),
        (_, Some(true), false) => ("◔", "reconnecting".to_string(), amber()),
        (_, Some(false), _) => (
            "●",
            v.and_then(|v| v.error.clone())
                .unwrap_or_else(|| "disconnected".into()),
            red(),
        ),
        (_, None, _) => ("○", "connecting".to_string(), dim()),
    };
    let mut h = row()
        .key("header")
        .w_full()
        .h(44.0)
        .px(16.0)
        .center_y()
        .gap(18.0)
        .fill(panel())
        .child(
            row()
                .center_y()
                .gap(2.0)
                .child(text("▍").font(mono_b(18.0)).color(teal()))
                .child(text("gnomon").font(wordmark(19.0)).color(paper())),
        )
        .child(
            row()
                .center_y()
                .gap(6.0)
                .child(t(dot, color))
                .child(t(status, color).max_w(420.0).truncate()),
        );
    if let Some(v) = v {
        let role = if v.cluster.replicated {
            format!(
                "raft {} · {} · term {}",
                v.cluster.node_id, v.cluster.role, v.cluster.term
            )
        } else {
            "standalone".to_string()
        };
        h = h
            .child(chip(&v.controller, muted()))
            .child(chip(
                &role,
                if v.cluster.role == "leader" {
                    teal_light()
                } else {
                    muted()
                },
            ))
            .child(chip(&format!("idx {}", v.state_index), muted()));
    }
    h.child(el().grow(1.0))
        .child(t(format!("{} UTC", clock(s.now())), muted()))
}

fn chip(label: &str, color: rune::Color) -> El {
    el().px(8.0)
        .py(3.0)
        .radius(3.0)
        .border(1.0, line())
        .child(ts(label.to_string(), color))
}

fn tabs(s: &State) -> El {
    let mut r = row()
        .key("tabs")
        .w_full()
        .h(34.0)
        .px(10.0)
        .gap(2.0)
        .fill(panel())
        .border_sides([0.0, 0.0, 1.0, 0.0], line());
    for (i, tab) in Tab::ALL.into_iter().enumerate() {
        let active = tab == s.tab;
        let label = row()
            .center_y()
            .gap(6.0)
            .child(ts(
                format!("{}", i + 1),
                if active { teal() } else { dim() },
            ))
            .child(t(tab.name(), if active { teal_light() } else { muted() }));
        let mut b = el()
            .key(format!("tab-{}", tab.name()))
            .role("tab")
            .label(tab.name())
            .checked(active)
            .focusable(true)
            .h(34.0)
            .px(14.0)
            .center()
            .hover(slate())
            .on_press(move |s: &mut State| s.tab = tab)
            .child(label);
        if active {
            b = b.fill(slate()).border_sides([0.0, 0.0, 2.0, 0.0], teal());
        }
        r = r.child(b);
    }
    r
}

fn footer(s: &State) -> El {
    let summary = match s.view.as_ref() {
        Some(v) => {
            let count = |st: &str| v.jobs.iter().filter(|j| j.state == st).count();
            format!(
                "{} running · {} reserved · {} pending · {}/{} nodes up · {}",
                count("RUNNING"),
                count("RESERVED"),
                count("PENDING"),
                v.nodes.iter().filter(|n| n.alive).count(),
                v.nodes.len(),
                v.policy.name
            )
        }
        None => String::new(),
    };
    row()
        .key("footer")
        .w_full()
        .h(26.0)
        .px(16.0)
        .center_y()
        .fill(panel())
        .border_sides([1.0, 0.0, 0.0, 0.0], line())
        .child(ts("1-7 views  ↑↓ select  x cancel  esc dismiss", dim()))
        .child(el().grow(1.0))
        .child(ts(summary, muted()))
}

fn toast(msg: &str, ok: bool) -> El {
    let color = if ok { teal() } else { amber() };
    el().key("toast")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .child(
            col()
                .w_full()
                .h_full()
                .justify_end()
                .items_end()
                .padding(22.0)
                .child(
                    row()
                        .key("toast-box")
                        .max_w(520.0)
                        .gap(10.0)
                        .px(14.0)
                        .py(10.0)
                        .radius(4.0)
                        .fill(slate())
                        .border(1.0, color)
                        .child(t(if ok { "✓" } else { "!" }, color))
                        .child(t(msg.to_string(), paper()).grow(1.0))
                        .child(
                            el().key("toast-x")
                                .px(6.0)
                                .on_press(|s: &mut State| s.toast = None)
                                .child(t("×", muted())),
                        ),
                ),
        )
}

/// A job's one-line description, shared by the queue and the timeline.
pub fn job_line(j: &gnomon_web_model::JobView) -> String {
    let shape = if j.num_nodes > 1 {
        format!("{}c×{}", j.cores, j.num_nodes)
    } else {
        format!("{}c", j.cores)
    };
    format!("#{} {} {}", j.id, j.name, shape)
}

/// The selected job's detail card (queue and timeline).
pub fn job_detail(s: &State, id: u64) -> El {
    let Some(j) = s.job(id) else {
        return panel_box().child(t(format!("job {id} is gone"), muted()));
    };
    let field = |k: &str, v: String| {
        row()
            .w_full()
            .gap(10.0)
            .child(ts(k.to_string(), dim()).w(92.0))
            .child(t(v, paper()).grow(1.0).truncate())
    };
    let mut card = panel_box()
        .key(format!("detail-{id}"))
        .child(
            row()
                .w_full()
                .center_y()
                .gap(10.0)
                .child(
                    tb(format!("#{} {}", j.id, j.name), paper())
                        .grow(1.0)
                        .truncate(),
                )
                .child(
                    el().px(8.0)
                        .py(2.0)
                        .radius(3.0)
                        .fill(state_color(&j.state))
                        .child(ts(j.state.clone(), ink())),
                ),
        )
        .child(field("user", format!("{} · {}", j.user, j.partition)))
        .child(field(
            "request",
            format!(
                "{} cores · {} MB · {} gpus{}",
                j.cores,
                j.mem_mb,
                j.gpus,
                if j.num_nodes > 1 {
                    format!(" · ×{} nodes (gang)", j.num_nodes)
                } else {
                    String::new()
                }
            ),
        ))
        .child(field("walltime", dur(j.walltime)))
        .child(field(
            "planned",
            if j.start > 0 {
                format!(
                    "{} → {} ({})",
                    clock(j.start),
                    clock(j.start + j.span),
                    dur(j.span)
                )
            } else {
                "not placed yet".into()
            },
        ))
        .child(field(
            "nodes",
            if j.nodes.is_empty() {
                "—".into()
            } else {
                j.nodes.join(", ")
            },
        ))
        .child(field("priority", j.priority.to_string()));
    if !j.container.is_empty() {
        card = card.child(field("container", j.container.clone()));
    }
    let killing: Vec<String> = s
        .view
        .as_ref()
        .map(|v| {
            v.nodes
                .iter()
                .filter(|n| n.kill_pending.contains(&id))
                .map(|n| n.name.clone())
                .collect()
        })
        .unwrap_or_default();
    if !killing.is_empty() {
        card = card.child(
            field(
                "kill channel",
                format!("stopping ranks on {}", killing.join(", ")),
            )
            .opacity(1.0),
        );
    }
    if j.is_active() {
        let enabled = s.writable();
        card = card.child(
            el().key(format!("cancel-{id}"))
                .role("button")
                .label("cancel job")
                .focusable(enabled)
                .mt(4.0)
                .h(30.0)
                .px(12.0)
                .center()
                .radius(3.0)
                .border(1.0, if enabled { red() } else { line() })
                .hover(slate())
                .on_press(move |s: &mut State| crate::net::cancel(s, id))
                .child(t(
                    if j.state == "RUNNING" {
                        "[x] cancel — kills every rank"
                    } else {
                        "[x] cancel"
                    },
                    if enabled { red() } else { dim() },
                )),
        );
    }
    card
}
