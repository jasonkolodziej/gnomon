//! Pure Gantt layout: time→column mapping, lane stacking of overlapping
//! spans, label truncation and state styling. No terminal IO here; the
//! renderer only copies these decisions into a buffer.

use crate::model::{ClusterSnapshot, JobState, JobView, NodeView};
use ratatui::style::{Color, Modifier, Style};

/// Maps epoch seconds onto `width` plot columns covering `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeAxis {
    pub start: i64,
    pub end: i64,
    pub width: u16,
}

/// Candidate tick spacings, seconds.
const TICK_STEPS: [i64; 14] = [
    60, 120, 300, 600, 900, 1800, 3600, 7200, 10_800, 21_600, 43_200, 86_400, 172_800, 604_800,
];
/// Minimum columns between two tick labels.
const TICK_GAP: i64 = 12;

impl TimeAxis {
    pub fn new(start: i64, end: i64, width: u16) -> Self {
        TimeAxis {
            start,
            end: end.max(start + 1),
            width,
        }
    }

    /// Column of time `t`, or `None` when outside the window.
    pub fn col(&self, t: i64) -> Option<u16> {
        if t < self.start || t >= self.end || self.width == 0 {
            return None;
        }
        Some(self.col_floor(t) as u16)
    }

    fn col_floor(&self, t: i64) -> i64 {
        let num = (t - self.start) as i128 * self.width as i128;
        num.div_euclid((self.end - self.start) as i128) as i64
    }

    /// Columns `[x0, x1)` covered by the span `[s, e)`, clamped to the plot;
    /// `None` when the span is entirely outside. A visible span is at least
    /// one column wide so short jobs never disappear.
    pub fn span_cols(&self, s: i64, e: i64) -> Option<(u16, u16)> {
        if e <= self.start || s >= self.end || e <= s || self.width == 0 {
            return None;
        }
        let w = self.width as i64;
        let x0 = self.col_floor(s).clamp(0, w - 1);
        // Ceil of the end so a span covering part of a column claims it.
        let num = (e - self.start) as i128 * w as i128;
        let den = (self.end - self.start) as i128;
        let x1 = ((num + den - 1) / den) as i64;
        let x1 = x1.clamp(x0 + 1, w);
        Some((x0 as u16, x1 as u16))
    }

    /// Tick marks `(column, label)` at multiples of a "nice" step relative
    /// to `now`, spaced at least `TICK_GAP` (12) columns apart.
    pub fn ticks(&self, now: i64) -> Vec<(u16, String)> {
        let span = self.end - self.start;
        let w = (self.width as i64).max(1);
        let step = TICK_STEPS
            .iter()
            .copied()
            .find(|s| s * w / span >= TICK_GAP)
            .unwrap_or(*TICK_STEPS.last().unwrap());
        let first = (self.start - now).div_euclid(step);
        let last = (self.end - now).div_euclid(step);
        (first..=last)
            .filter_map(|k| {
                let t = now + k * step;
                self.col(t).map(|c| (c, fmt_rel(k * step)))
            })
            .collect()
    }
}

/// Relative time label: `now`, `-10m`, `+2h`, `+1h30m`, `+2d`.
pub fn fmt_rel(secs: i64) -> String {
    if secs == 0 {
        return "now".into();
    }
    let sign = if secs < 0 { '-' } else { '+' };
    format!("{sign}{}", fmt_dur(secs.unsigned_abs()))
}

/// Compact duration: `45s`, `10m`, `2h`, `1h30m`, `3d`, `1d2h`.
pub fn fmt_dur(secs: u64) -> String {
    let (d, h, m, s) = (
        secs / 86_400,
        secs % 86_400 / 3600,
        secs % 3600 / 60,
        secs % 60,
    );
    match (d, h, m) {
        (0, 0, 0) => format!("{s}s"),
        (0, 0, _) => format!("{m}m"),
        (0, _, 0) => format!("{h}h"),
        (0, _, _) => format!("{h}h{m:02}m"),
        (_, 0, _) => format!("{d}d"),
        _ => format!("{d}d{h}h"),
    }
}

/// Assigns each span `[s, e)` the lowest lane free at its start (greedy
/// interval colouring by start time), so overlapping spans on one node row
/// stack instead of hiding each other. Returns one lane per input span.
pub fn stack_lanes(spans: &[(i64, i64)]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| (spans[i].0, spans[i].1, i));
    let mut lane_end: Vec<i64> = Vec::new();
    let mut lanes = vec![0; spans.len()];
    for i in order {
        let (s, e) = spans[i];
        let lane = match lane_end.iter().position(|&end| end <= s) {
            Some(l) => l,
            None => {
                lane_end.push(i64::MIN);
                lane_end.len() - 1
            }
        };
        lane_end[lane] = e;
        lanes[i] = lane;
    }
    lanes
}

/// Truncates `s` to `width` characters, marking a cut with `…`.
pub fn truncate(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n <= width {
        return s.to_string();
    }
    match width {
        0 => String::new(),
        1 => "…".into(),
        _ => s.chars().take(width - 1).chain(['…']).collect(),
    }
}

/// Bar label `"<id> <name>"` fitted to a bar `width` columns wide; empty for
/// bars too narrow to carry a readable label.
pub fn bar_label(id: u64, name: &str, width: usize) -> String {
    if width < 2 {
        return String::new();
    }
    let full = if name.is_empty() {
        id.to_string()
    } else {
        format!("{id} {name}")
    };
    truncate(&full, width)
}

/// Accent colour of a job state (header counts, table, bars).
pub fn state_color(state: JobState) -> Color {
    match state {
        JobState::Running => Color::Green,
        JobState::Reserved => Color::Yellow,
        JobState::Pending => Color::Cyan,
        JobState::Completed => Color::DarkGray,
        JobState::Failed => Color::Red,
        JobState::Cancelled => Color::Magenta,
        JobState::Unknown => Color::Gray,
    }
}

/// Fill character of a bar: solid for running spans, hatched for
/// reservations.
pub fn fill_char(state: JobState) -> char {
    match state {
        JobState::Running => '█',
        _ => '░',
    }
}

/// Style of a bar's fill cells.
pub fn fill_style(state: JobState) -> Style {
    let s = Style::default().fg(state_color(state));
    if state == JobState::Running {
        s
    } else {
        s.add_modifier(Modifier::DIM)
    }
}

/// Style of a bar's label cells: dark text on a solid running bar, coloured
/// text over a hatched reservation.
pub fn label_style(state: JobState) -> Style {
    if state == JobState::Running {
        Style::default()
            .fg(Color::Black)
            .bg(state_color(state))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(state_color(state))
    }
}

/// A job bar on one lane, in plot columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bar {
    pub x0: u16,
    pub x1: u16,
    pub job: u64,
    pub state: JobState,
    pub label: String,
}

/// What a Gantt lane belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// Index into [`ClusterSnapshot::nodes`].
    Node(usize),
    /// Planned jobs whose nodes are unknown.
    Pending,
}

/// One terminal row of the Gantt plot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lane {
    pub owner: Owner,
    /// 0 for the owner's first lane (which carries the gutter text).
    pub index: usize,
    pub bars: Vec<Bar>,
}

/// Lays out every node's planned jobs on stacked lanes, followed by a
/// "pending" lane for planned jobs whose nodes are unknown. Each node gets
/// at least one lane; the pending lane appears only when needed.
pub fn gantt_lanes<'a>(
    snap: &ClusterSnapshot,
    jobs: impl Iterator<Item = &'a JobView> + Clone,
    axis: &TimeAxis,
    now: u64,
) -> Vec<Lane> {
    let mut out = Vec::new();
    for (i, node) in snap.nodes.iter().enumerate() {
        let on_node = jobs.clone().filter(|j| j.nodes.contains(&node.name));
        push_owner_lanes(&mut out, Owner::Node(i), on_node, axis, now, true);
    }
    let known = |n: &String| snap.nodes.iter().any(|x| x.name == *n);
    let unplaced = jobs.filter(|j| !j.nodes.iter().any(known));
    push_owner_lanes(&mut out, Owner::Pending, unplaced, axis, now, false);
    out
}

fn push_owner_lanes<'a>(
    out: &mut Vec<Lane>,
    owner: Owner,
    jobs: impl Iterator<Item = &'a JobView>,
    axis: &TimeAxis,
    now: u64,
    always: bool,
) {
    let planned: Vec<(&JobView, (i64, i64))> = jobs
        .filter_map(|j| j.span(now).map(|(s, e)| (j, (s as i64, e as i64))))
        .collect();
    let spans: Vec<(i64, i64)> = planned.iter().map(|p| p.1).collect();
    let lanes = stack_lanes(&spans);
    let n_lanes = lanes.iter().max().map_or(0, |m| m + 1);
    if n_lanes == 0 && !always {
        return;
    }
    let base = out.len();
    for index in 0..n_lanes.max(1) {
        out.push(Lane {
            owner: owner.clone(),
            index,
            bars: Vec::new(),
        });
    }
    for ((job, (s, e)), lane) in planned.iter().zip(lanes) {
        if let Some((x0, x1)) = axis.span_cols(*s, *e) {
            out[base + lane].bars.push(Bar {
                x0,
                x1,
                job: job.id,
                state: job.state,
                label: bar_label(job.id, &job.name, (x1 - x0) as usize),
            });
        }
    }
}

/// Width of the name column in the gutter.
pub fn name_width(nodes: &[NodeView]) -> usize {
    nodes
        .iter()
        .map(|n| n.name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(7, 14)
}

/// Gutter text of a node's first lane: name, liveness dot, allocated/total
/// cores and (when the cluster has GPUs) GPUs.
pub fn gutter_text(node: &NodeView, name_w: usize, show_gpus: bool) -> String {
    let dot = if node.alive { '●' } else { '○' };
    let name = truncate(&node.name, name_w);
    let mut s = format!(
        "{name:<name_w$} {dot} {:>3}/{:<3}c",
        node.cores_alloc, node.cores
    );
    if show_gpus {
        s.push_str(&format!(" {}/{}g", node.gpus_alloc, node.gpus));
    }
    s
}

/// Pending-state jobs for the bottom table, highest priority first, then
/// longest waiting.
pub fn pending_jobs<'a>(jobs: impl Iterator<Item = &'a JobView>) -> Vec<&'a JobView> {
    let mut v: Vec<&JobView> = jobs.filter(|j| j.state == JobState::Pending).collect();
    v.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(a.submit_time.cmp(&b.submit_time))
            .then(a.id.cmp(&b.id))
    });
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_maps_linearly_onto_columns() {
        let a = TimeAxis::new(0, 100, 50);
        assert_eq!(a.col(0), Some(0));
        assert_eq!(a.col(1), Some(0));
        assert_eq!(a.col(2), Some(1));
        assert_eq!(a.col(99), Some(49));
        assert_eq!(a.col(100), None);
        assert_eq!(a.col(-1), None);
    }

    #[test]
    fn spans_are_clamped_and_never_vanish() {
        let a = TimeAxis::new(1000, 1100, 100);
        assert_eq!(a.span_cols(1010, 1020), Some((10, 20)));
        assert_eq!(a.span_cols(900, 1005), Some((0, 5)));
        assert_eq!(a.span_cols(1090, 5000), Some((90, 100)));
        assert_eq!(a.span_cols(800, 900), None);
        assert_eq!(a.span_cols(1100, 1200), None);
        // Narrower than a column: still one column.
        let wide = TimeAxis::new(0, 10_000, 10);
        assert_eq!(wide.span_cols(5, 6), Some((0, 1)));
        // Partial trailing column is claimed.
        assert_eq!(wide.span_cols(0, 1001), Some((0, 2)));
    }

    #[test]
    fn ticks_are_nicely_spaced_and_include_now() {
        // -10m .. +2h over 130 columns: 7800 s, 60 s/col -> 15-minute steps.
        let a = TimeAxis::new(10_000 - 600, 10_000 + 7200, 130);
        let ticks = a.ticks(10_000);
        assert!(ticks.contains(&(10, "now".to_string())));
        assert!(ticks.contains(&(25, "+15m".to_string())));
        assert!(ticks.windows(2).all(|w| w[1].0 - w[0].0 >= TICK_GAP as u16));
    }

    #[test]
    fn durations_format_compactly() {
        assert_eq!(fmt_rel(0), "now");
        assert_eq!(fmt_rel(-600), "-10m");
        assert_eq!(fmt_rel(7200), "+2h");
        assert_eq!(fmt_rel(5400), "+1h30m");
        assert_eq!(fmt_dur(45), "45s");
        assert_eq!(fmt_dur(3 * 86_400), "3d");
        assert_eq!(fmt_dur(86_400 + 7200), "1d2h");
    }

    #[test]
    fn overlapping_spans_stack_onto_lanes() {
        let spans = [(0, 10), (5, 15), (10, 20), (12, 14), (30, 40)];
        assert_eq!(stack_lanes(&spans), vec![0, 1, 0, 2, 0]);
        assert!(stack_lanes(&[]).is_empty());
    }

    #[test]
    fn labels_truncate_with_ellipsis() {
        assert_eq!(truncate("training", 10), "training");
        assert_eq!(truncate("training", 5), "trai…");
        assert_eq!(truncate("training", 1), "…");
        assert_eq!(truncate("training", 0), "");
        assert_eq!(bar_label(12, "train", 20), "12 train");
        assert_eq!(bar_label(12, "train", 5), "12 t…");
        assert_eq!(bar_label(12, "train", 1), "");
        assert_eq!(bar_label(7, "", 4), "7");
    }

    #[test]
    fn states_map_to_fill_and_colour() {
        assert_eq!(fill_char(JobState::Running), '█');
        assert_eq!(fill_char(JobState::Reserved), '░');
        assert_eq!(state_color(JobState::Running), Color::Green);
        assert_eq!(state_color(JobState::Reserved), Color::Yellow);
        assert!(fill_style(JobState::Reserved)
            .add_modifier
            .contains(Modifier::DIM));
        assert_eq!(label_style(JobState::Running).bg, Some(Color::Green));
    }

    fn job(id: u64, state: JobState, start: u64, dur: u64, nodes: &[&str]) -> JobView {
        JobView {
            id,
            name: format!("j{id}"),
            user: "u".into(),
            state,
            priority: id,
            submit_time: 0,
            cores: 1,
            planned_start: (start > 0).then_some(start),
            planned_dur: dur,
            nodes: nodes.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn lanes_group_by_node_and_route_unknown_nodes_to_pending() {
        let snap = ClusterSnapshot {
            nodes: vec![
                NodeView {
                    name: "n0".into(),
                    ..Default::default()
                },
                NodeView {
                    name: "n1".into(),
                    ..Default::default()
                },
            ],
            jobs: vec![
                job(1, JobState::Running, 1000, 100, &["n0"]),
                job(2, JobState::Reserved, 1050, 100, &["n0"]),
                job(3, JobState::Reserved, 1200, 100, &["ghost"]),
                job(4, JobState::Pending, 0, 100, &[]),
            ],
            ..Default::default()
        };
        let axis = TimeAxis::new(1000, 1400, 40);
        let lanes = gantt_lanes(&snap, snap.jobs.iter(), &axis, 1010);
        let owners: Vec<_> = lanes.iter().map(|l| (l.owner.clone(), l.index)).collect();
        assert_eq!(
            owners,
            vec![
                (Owner::Node(0), 0),
                (Owner::Node(0), 1),
                (Owner::Node(1), 0),
                (Owner::Pending, 0)
            ]
        );
        assert_eq!(lanes[0].bars[0].x0..lanes[0].bars[0].x1, 0..10);
        assert_eq!(lanes[1].bars[0].x0..lanes[1].bars[0].x1, 5..15);
        assert!(lanes[2].bars.is_empty());
        assert_eq!(lanes[3].bars[0].job, 3);
    }

    #[test]
    fn gutter_shows_allocation_and_liveness() {
        let n = NodeView {
            name: "node-0".into(),
            cores: 16,
            cores_alloc: 8,
            gpus: 2,
            gpus_alloc: 1,
            alive: true,
        };
        assert_eq!(gutter_text(&n, 7, true), "node-0  ●   8/16 c 1/2g");
        let down = NodeView { alive: false, ..n };
        assert_eq!(gutter_text(&down, 7, false), "node-0  ○   8/16 c");
    }

    #[test]
    fn pending_table_orders_by_priority_then_age() {
        let mut a = job(1, JobState::Pending, 0, 1, &[]);
        let mut b = job(2, JobState::Pending, 0, 1, &[]);
        let c = job(3, JobState::Running, 5, 1, &["n0"]);
        a.priority = 5;
        b.priority = 5;
        a.submit_time = 20;
        b.submit_time = 10;
        let jobs = [a, b, c];
        let ids: Vec<u64> = pending_jobs(jobs.iter()).iter().map(|j| j.id).collect();
        assert_eq!(ids, vec![2, 1]);
    }
}
