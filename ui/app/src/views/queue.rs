//! The job table (gnqueue, live), the selected job, and a new-job form.

use super::job_detail;
use crate::state::State;
use crate::theme::*;
use gnomon_web_model::SubmitForm;
use rune::{El, TextEdit, col, el, row, text_input};

const COLS: [(&str, f32); 8] = [
    ("ID", 54.0),
    ("ST", 46.0),
    ("USER", 70.0),
    ("NAME", 150.0),
    ("CPUS", 70.0),
    ("TIME", 64.0),
    ("START", 74.0),
    ("NODES", 0.0),
];

pub fn view(s: &State) -> El {
    let jobs = s.queue();
    let mut head = row()
        .w_full()
        .h(26.0)
        .px(10.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], line());
    for (name, w) in COLS {
        let cell = ts(name, dim());
        head = head.child(if w > 0.0 { cell.w(w) } else { cell.grow(1.0) });
    }
    let mut rows = col().w_full();
    for j in &jobs {
        let selected = s.selected == Some(j.id);
        let id = j.id;
        let c = state_color(&j.state);
        let cells: [(String, rune::Color); 8] = [
            (format!("{}", j.id), paper()),
            (state_tag(&j.state).to_string(), c),
            (j.user.clone(), muted()),
            (j.name.clone(), paper()),
            (
                if j.num_nodes > 1 {
                    format!("{}×{}", j.cores, j.num_nodes)
                } else {
                    j.cores.to_string()
                },
                muted(),
            ),
            (dur(j.walltime), muted()),
            (
                if j.start > 0 {
                    clock(j.start)
                } else {
                    "—".into()
                },
                muted(),
            ),
            (
                if j.nodes.is_empty() {
                    "—".into()
                } else {
                    j.nodes.join(",")
                },
                muted(),
            ),
        ];
        let mut r = row()
            .key(format!("row-{id}"))
            .role("row")
            .label(format!("job {id}"))
            .w_full()
            .h(26.0)
            .px(10.0)
            .center_y()
            .hover(slate())
            .on_press(move |s: &mut State| s.selected = Some(id));
        if selected {
            r = r.fill(slate()).border_sides([0.0, 0.0, 0.0, 2.0], teal());
        }
        for ((value, color), (_, w)) in cells.into_iter().zip(COLS) {
            let cell = ts(value, color).truncate();
            r = r.child(if w > 0.0 { cell.w(w) } else { cell.grow(1.0) });
        }
        rows = rows.child(r);
    }
    let table = col()
        .key("queue-table")
        .grow(1.0)
        .shrink()
        .h_full()
        .fill(panel())
        .border(1.0, line())
        .radius(4.0)
        .child(head)
        .child(
            col()
                .key("queue-scroll")
                .w_full()
                .grow(1.0)
                .shrink()
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(rows),
        );

    let mut side = col().key("queue-side").w(400.0).gap(14.0);
    if let Some(id) = s.selected {
        side = side.child(job_detail(s, id));
    }
    side = side.child(form(s));
    col()
        .w_full()
        .h_full()
        .gap(12.0)
        .child(caption(&format!("queue · {} jobs", jobs.len())))
        .child(
            row()
                .w_full()
                .grow(1.0)
                .shrink()
                .gap(16.0)
                .child(table)
                .child(side),
        )
}

fn input(label: &str, edit: &TextEdit, key: &str, field: fn(&mut State) -> &mut TextEdit) -> El {
    row()
        .w_full()
        .center_y()
        .gap(10.0)
        .child(ts(label.to_string(), dim()).w(84.0))
        .child(
            text_input(edit, mono(BODY))
                .key(key.to_string())
                .color(paper())
                .grow(1.0)
                .h(28.0)
                .px(8.0)
                .py(5.0)
                .radius(3.0)
                .fill(ink())
                .border(1.0, line())
                .on_edit(field),
        )
}

fn number(e: &TextEdit) -> u64 {
    e.text.trim().parse().unwrap_or(0)
}

fn submit(s: &mut State) {
    let f = &s.form;
    let form = SubmitForm {
        name: f.name.text.trim().to_string(),
        user: f.user.text.trim().to_string(),
        partition: String::new(),
        cores: number(&f.cores) as u32,
        mem_mb: 0,
        gpus: 0,
        nodes: number(&f.nodes).max(1) as u32,
        walltime_secs: number(&f.walltime),
        priority: 0,
        command: f.command.text.trim().to_string(),
        container: f.container.text.trim().to_string(),
    };
    crate::net::submit(s, form);
}

fn form(s: &State) -> El {
    let f = &s.form;
    let enabled = s.writable();
    panel_box()
        .key("submit-form")
        .child(tb("new job", paper()))
        .child(input("name", &f.name, "f-name", |s| &mut s.form.name))
        .child(input("user", &f.user, "f-user", |s| &mut s.form.user))
        .child(input("cores", &f.cores, "f-cores", |s| &mut s.form.cores))
        .child(input("nodes", &f.nodes, "f-nodes", |s| &mut s.form.nodes))
        .child(input("walltime s", &f.walltime, "f-wall", |s| {
            &mut s.form.walltime
        }))
        .child(input("command", &f.command, "f-cmd", |s| {
            &mut s.form.command
        }))
        .child(input("container", &f.container, "f-image", |s| {
            &mut s.form.container
        }))
        .child(
            row()
                .w_full()
                .center_y()
                .gap(10.0)
                .mt(4.0)
                .child(
                    el().key("submit")
                        .role("button")
                        .label("submit job")
                        .focusable(enabled)
                        .h(30.0)
                        .px(14.0)
                        .center()
                        .radius(3.0)
                        .fill(if enabled { teal() } else { slate() })
                        .on_press(|s: &mut State| {
                            if s.writable() || s.demo {
                                submit(s)
                            }
                        })
                        .child(tb("submit ⏎", if enabled { ink() } else { dim() })),
                )
                .child(ts(
                    if enabled {
                        "nodes > 1: a gang, all ranks start together"
                    } else if s.demo {
                        "demo data: read-only"
                    } else {
                        "read-only: gnomon-web --read-only"
                    },
                    dim(),
                )),
        )
}
