//! Draws an [`App`] into a ratatui [`Buffer`]: header, per-node Gantt plot
//! with a "now" cursor, pending-job table and key-help footer. Rendering is
//! a pure function of the app state, so tests snapshot it on a
//! `TestBackend` and `--once` prints it as plain text.

use crate::app::{App, Conn};
use crate::layout::{
    fill_char, fill_style, fmt_dur, fmt_rel, gantt_lanes, gutter_text, label_style, name_width,
    pending_jobs, state_color, Lane, Owner, TimeAxis,
};
use crate::model::{state_counts, ClusterSnapshot, JobState};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table, Widget};

/// Rows the Gantt plot keeps even when the pending table is long.
const MIN_GANTT_ROWS: u16 = 4;
/// Most pending jobs listed before the table is cut.
const MAX_TABLE_ROWS: u16 = 8;
/// Label of the lane holding planned jobs on unknown nodes.
pub const PENDING_LANE: &str = "pending";

/// Renders the whole screen into `buf` over `area`.
pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let pending = pending_jobs(app.visible_jobs());
    let wanted_table = 3 + (pending.len() as u16).clamp(1, MAX_TABLE_ROWS);
    let table_h = wanted_table.min(area.height.saturating_sub(3 + MIN_GANTT_ROWS));
    let [header, gantt, table, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(table_h),
        Constraint::Length(1),
    ])
    .areas(area);
    render_header(app, header, buf);
    render_gantt(app, gantt, buf);
    if table_h > 0 {
        render_pending(app, &pending, table, buf);
    }
    render_footer(app, footer, buf);
}

/// Renders a single frame as plain text (no colours, trailing spaces
/// trimmed): the `--once` output.
pub fn render_text(app: &App, width: u16, height: u16) -> String {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    render(app, area, &mut buf);
    buffer_text(&buf)
}

/// The buffer's symbols, one line per row, trailing spaces trimmed.
pub fn buffer_text(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        for x in area.left()..area.right() {
            line.push_str(buf[(x, y)].symbol());
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn render_header(app: &App, area: Rect, buf: &mut Buffer) {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut top = vec![
        Span::styled("gntop", bold),
        Span::raw(format!(" {} ", app.addr)),
    ];
    if let Some(s) = &app.snapshot {
        top.push(Span::raw(format!(" idx {} ", s.state_index)));
    }
    top.push(match &app.conn {
        Conn::Connecting => Span::styled(" connecting…", Style::default().fg(Color::Yellow)),
        Conn::Connected => Span::styled(" ● connected", Style::default().fg(Color::Green)),
        Conn::Disconnected(e) => Span::styled(
            format!(" ✗ disconnected, retrying: {e}"),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    });
    if let (Conn::Disconnected(_), Some(s)) = (&app.conn, &app.snapshot) {
        let age = app.now.saturating_sub(s.fetched_at);
        top.push(Span::raw(format!(" (stale {})", fmt_dur(age))));
    }
    if app.paused {
        top.push(Span::raw(" "));
        top.push(Span::styled(
            " PAUSED ",
            Style::default().add_modifier(Modifier::REVERSED),
        ));
    }

    let mut second = Vec::new();
    for (state, n) in state_counts(app.visible_jobs()) {
        // Live states always show; finished ones only once they occur.
        if n == 0
            && !matches!(
                state,
                JobState::Running | JobState::Reserved | JobState::Pending
            )
        {
            continue;
        }
        second.push(Span::styled(
            format!("{} {n}", state.short()),
            Style::default().fg(state_color(state)),
        ));
        second.push(Span::raw("  "));
    }
    if let Some(s) = &app.snapshot {
        let up = s.nodes.iter().filter(|n| n.alive).count();
        second.push(Span::raw(format!("│ nodes {up}/{} up  ", s.nodes.len())));
    }
    let w = app.window;
    second.push(Span::raw(format!(
        "│ window {} … {}",
        fmt_rel(w.offset - w.history as i64),
        fmt_rel(w.offset + w.horizon as i64)
    )));
    if !app.filter.is_empty() {
        second.push(Span::raw(format!("  │ user={}", app.filter)));
    }
    Paragraph::new(vec![Line::from(top), Line::from(second)]).render(area, buf);
}

fn render_gantt(app: &App, area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let Some(snap) = &app.snapshot else {
        let msg = match &app.conn {
            Conn::Disconnected(e) => format!("disconnected, retrying… ({e})"),
            _ => format!("connecting to {}…", app.addr),
        };
        let y = area.y + area.height / 2;
        let x = area.x + area.width.saturating_sub(msg.chars().count() as u16) / 2;
        buf.set_stringn(
            x,
            y,
            &msg,
            area.width as usize,
            Style::default().fg(Color::Red),
        );
        return;
    };

    let name_w = name_width(&snap.nodes);
    let show_gpus = snap.nodes.iter().any(|n| n.gpus > 0);
    let gutter_w = snap
        .nodes
        .iter()
        .map(|n| gutter_text(n, name_w, show_gpus).chars().count())
        .max()
        .unwrap_or(0)
        .max(PENDING_LANE.len())
        .min(area.width as usize / 2) as u16;
    let plot_x = area.x + gutter_w + 1;
    let plot_w = area.width.saturating_sub(gutter_w + 1);
    let (start, end) = app.window.bounds(app.now);
    let axis = TimeAxis::new(start, end, plot_w);
    let now = app.now as i64;

    // Axis row.
    buf.set_stringn(area.x, area.y, "NODE", gutter_w as usize, Style::default());
    buf[(area.x + gutter_w, area.y)].set_symbol("│");
    for (col, label) in axis.ticks(now) {
        let style = if label == "now" {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let room = (plot_w - col) as usize;
        buf.set_stringn(plot_x + col, area.y, format!("┊{label}"), room, style);
    }

    let lanes = gantt_lanes(snap, app.visible_jobs(), &axis, app.now);
    let rows = area.height.saturating_sub(1) as usize;
    let hidden = lanes.len().saturating_sub(rows);
    let shown = if hidden > 0 {
        rows.saturating_sub(1)
    } else {
        rows
    };
    for (i, lane) in lanes.iter().take(shown).enumerate() {
        let y = area.y + 1 + i as u16;
        draw_gutter(snap, lane, area.x, y, gutter_w, name_w, show_gpus, buf);
        buf[(area.x + gutter_w, y)].set_symbol("│");
        draw_bars(lane, plot_x, y, buf);
    }
    if hidden > 0 {
        let y = area.y + area.height - 1;
        let more = format!("… {} more lanes", lanes.len() - shown);
        buf.set_stringn(area.x, y, more, area.width as usize, Style::default());
    }

    if let Some(col) = axis.col(now) {
        let cursor = Style::default().fg(Color::Red);
        for i in 0..shown.min(lanes.len()) {
            let cell = &mut buf[(plot_x + col, area.y + 1 + i as u16)];
            // Bars' label text (spaces included) carries a colour; only
            // blank background and bar fill give way to the cursor.
            let blank = cell.fg == Color::Reset && cell.bg == Color::Reset;
            match cell.symbol() {
                " " if blank => {
                    cell.set_symbol("│").set_style(cursor);
                }
                "█" => {
                    cell.set_symbol("│")
                        .set_style(cursor.bg(state_color(JobState::Running)));
                }
                "░" => {
                    cell.set_symbol("│").set_style(cursor);
                }
                _ => {}
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_gutter(
    snap: &ClusterSnapshot,
    lane: &Lane,
    x: u16,
    y: u16,
    width: u16,
    name_w: usize,
    show_gpus: bool,
    buf: &mut Buffer,
) {
    if lane.index > 0 {
        return;
    }
    match lane.owner {
        Owner::Node(i) => {
            let node = &snap.nodes[i];
            let style = if node.alive {
                Style::default()
            } else {
                Style::default().fg(Color::Red)
            };
            let text = gutter_text(node, name_w, show_gpus);
            buf.set_stringn(x, y, text, width as usize, style);
        }
        Owner::Pending => {
            let style = Style::default().fg(state_color(JobState::Pending));
            buf.set_stringn(x, y, PENDING_LANE, width as usize, style);
        }
    }
}

fn draw_bars(lane: &Lane, plot_x: u16, y: u16, buf: &mut Buffer) {
    for bar in &lane.bars {
        let fill = fill_char(bar.state);
        let style = fill_style(bar.state);
        for x in bar.x0..bar.x1 {
            buf[(plot_x + x, y)].set_char(fill).set_style(style);
        }
        let room = (bar.x1 - bar.x0) as usize;
        buf.set_stringn(plot_x + bar.x0, y, &bar.label, room, label_style(bar.state));
    }
}

fn render_pending(app: &App, pending: &[&crate::model::JobView], area: Rect, buf: &mut Buffer) {
    let header = Row::new(["JOBID", "USER", "NAME", "PRIO", "CPUS", "WAIT"])
        .style(Style::default().add_modifier(Modifier::BOLD));
    let rows = pending.iter().map(|j| {
        Row::new([
            j.id.to_string(),
            j.user.clone(),
            j.name.clone(),
            j.priority.to_string(),
            j.cores.to_string(),
            fmt_dur(j.wait_secs(app.now)),
        ])
    });
    let widths = [
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Min(8),
        Constraint::Length(8),
        Constraint::Length(5),
        Constraint::Length(8),
    ];
    let title = format!(" Pending jobs ({}) ", pending.len());
    Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .render(area, buf);
}

fn render_footer(app: &App, area: Rect, buf: &mut Buffer) {
    let line = match &app.input {
        Some(text) => Line::from(vec![
            Span::styled("/ user filter: ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{text}▏")),
            Span::styled(
                "  (Enter apply, empty = all; Esc cancel)",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        None => Line::styled(
            "q quit  p pause  +/- zoom  ←/→ pan  0 reset  / user filter   █ running  ░ reserved  │ now",
            Style::default().fg(Color::DarkGray),
        ),
    };
    Paragraph::new(line).render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Update;
    use crate::model::{JobView, NodeView};
    use crate::view::Window;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const NOW: u64 = 1_000_000;
    /// Gutter "n0      ●   4/8  c" is 18 wide, plus the separator.
    const PLOT_X: u16 = 19;
    /// 60 plot columns over a 3600 s window: one column per minute.
    const WIDTH: u16 = PLOT_X + 60;

    fn job(id: u64, name: &str, state: JobState, start: i64, dur: u64, nodes: &[&str]) -> JobView {
        JobView {
            id,
            name: name.into(),
            user: "alice".into(),
            state,
            priority: 0,
            submit_time: NOW - 30,
            cores: 2,
            planned_start: (start != i64::MIN).then(|| (NOW as i64 + start) as u64),
            planned_dur: dur,
            nodes: nodes.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn fixture() -> App {
        let mut wait = job(5, "wait", JobState::Pending, i64::MIN, 600, &[]);
        wait.user = "bob".into();
        wait.priority = 50;
        wait.submit_time = NOW - 125;
        let mut later = job(6, "later", JobState::Pending, i64::MIN, 600, &[]);
        later.priority = 10;
        let snap = ClusterSnapshot {
            fetched_at: NOW,
            state_index: 77,
            nodes: vec![
                NodeView {
                    name: "n0".into(),
                    cores: 8,
                    cores_alloc: 4,
                    alive: true,
                    ..Default::default()
                },
                NodeView {
                    name: "n1".into(),
                    cores: 8,
                    alive: false,
                    ..Default::default()
                },
            ],
            jobs: vec![
                job(1, "train", JobState::Running, -300, 1200, &["n0"]),
                job(2, "sim", JobState::Reserved, 600, 1800, &["n0"]),
                job(3, "post", JobState::Reserved, 900, 300, &["n0"]),
                job(4, "ghost", JobState::Reserved, 1200, 600, &["n9"]),
                wait,
                later,
                job(7, "old", JobState::Completed, -3000, 60, &["n0"]),
            ],
        };
        let mut app = App::new("http://ctl:7631", Window::new(600, 3000), NOW);
        app.apply(Update::Snapshot(snap));
        app
    }

    fn draw(app: &App, w: u16, h: u16) -> Buffer {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render(app, f.area(), f.buffer_mut()))
            .unwrap();
        term.backend().buffer().clone()
    }

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    fn text(buf: &Buffer, x: u16, y: u16, len: u16) -> String {
        (x..x + len).map(|x| buf[(x, y)].symbol()).collect()
    }

    #[test]
    fn header_shows_counts_index_and_window() {
        let buf = draw(&fixture(), WIDTH, 16);
        let top = row(&buf, 0);
        assert!(top.starts_with("gntop http://ctl:7631"), "{top}");
        assert!(
            top.contains("idx 77") && top.contains("● connected"),
            "{top}"
        );
        let counts = row(&buf, 1);
        for want in ["RUN 1", "RES 3", "PEND 2", "DONE 1", "nodes 1/2 up"] {
            assert!(counts.contains(want), "{want:?} missing in {counts}");
        }
        assert!(!counts.contains("FAIL"), "zero finished states are hidden");
        assert!(counts.contains("window -10m … +50m"), "{counts}");
    }

    #[test]
    fn bars_land_on_their_time_columns_with_labels() {
        let buf = draw(&fixture(), WIDTH, 16);
        // Axis: "now" tick at column 10 (600 s of history at 60 s/col).
        assert!(row(&buf, 2).starts_with("NODE"));
        assert_eq!(text(&buf, PLOT_X + 10, 2, 4), "┊now");
        // n0 lane 0: running job 1 over [-5m, +15m) = columns 5..25.
        assert!(
            row(&buf, 3).starts_with("n0      ●   4/8  c│"),
            "{}",
            row(&buf, 3)
        );
        assert_eq!(text(&buf, PLOT_X + 5, 3, 7), "1 train");
        assert_eq!(text(&buf, PLOT_X + 12, 3, 13), "█".repeat(13));
        assert_eq!(buf[(PLOT_X + 12, 3)].fg, Color::Green);
        assert_eq!(buf[(PLOT_X + 4, 3)].symbol(), " ");
        // Job 3 starts exactly when job 1 ends, so it shares lane 0.
        assert_eq!(text(&buf, PLOT_X + 25, 3, 5), "3 po…");
        assert_eq!(buf[(PLOT_X + 30, 3)].symbol(), " ");
        // n0 lane 1: reservation 2 (from +10m) overlaps job 1 and stacks below.
        assert!(row(&buf, 4).starts_with(&" ".repeat(18)));
        assert_eq!(text(&buf, PLOT_X + 20, 4, 5), "2 sim");
        assert_eq!(text(&buf, PLOT_X + 25, 4, 25), "░".repeat(25));
        assert_eq!(buf[(PLOT_X + 50, 4)].symbol(), " ");
        assert!(buf[(PLOT_X + 30, 4)].modifier.contains(Modifier::DIM));
        // n1 is down and idle.
        assert!(row(&buf, 5).starts_with("n1      ○   0/8  c│"));
        // Job 4's node is unknown: it lands on the pending lane.
        assert!(row(&buf, 6).starts_with("pending"));
        assert_eq!(text(&buf, PLOT_X + 30, 6, 7), "4 ghost");
        // Completed job 7 is off the time axis entirely.
        assert!(!buffer_text(&buf).contains("7 old"));
    }

    #[test]
    fn now_cursor_crosses_empty_cells_and_running_fill() {
        let mut app = fixture();
        // Same axis, but now is 3 minutes later: the cursor (column 13)
        // falls on job 1's fill rather than its label.
        app.now = NOW + 180;
        app.window.offset = -180;
        let buf = draw(&app, WIDTH, 16);
        let col = PLOT_X + 13;
        for y in [4, 5, 6] {
            assert_eq!(buf[(col, y)].symbol(), "│", "row {y}");
            assert_eq!(buf[(col, y)].fg, Color::Red);
        }
        // Running fill keeps its colour as background under the cursor.
        assert_eq!(buf[(col, 3)].symbol(), "│");
        assert_eq!(buf[(col, 3)].bg, Color::Green);
    }

    #[test]
    fn now_cursor_never_overwrites_label_text() {
        let mut app = fixture();
        // Now 4 minutes earlier: the cursor (column 6) hits the space in
        // job 1's "1 train" label.
        app.now = NOW - 240;
        app.window.offset = 240;
        let buf = draw(&app, WIDTH, 16);
        assert_eq!(text(&buf, PLOT_X + 5, 3, 7), "1 train");
        assert_eq!(buf[(PLOT_X + 6, 4)].symbol(), "│");
    }

    #[test]
    fn pending_table_lists_queue_by_priority_with_wait_time() {
        let buf = draw(&fixture(), WIDTH, 16);
        let all = buffer_text(&buf);
        assert!(all.contains("Pending jobs (2)"), "{all}");
        let lines: Vec<&str> = all.lines().collect();
        let hdr = lines.iter().position(|l| l.contains("JOBID")).unwrap();
        let first: Vec<&str> = lines[hdr + 1].split_whitespace().collect();
        assert_eq!(first, ["│5", "bob", "wait", "50", "2", "2m", "│"]);
        assert!(lines[hdr + 2].contains("later"));
        assert!(lines[15].starts_with("q quit"), "{}", lines[15]);
    }

    #[test]
    fn user_filter_hides_other_users_and_shows_in_header() {
        let mut app = fixture();
        app.filter = "bob".into();
        let all = buffer_text(&draw(&app, WIDTH, 16));
        assert!(all.contains("user=bob"));
        assert!(all.contains("Pending jobs (1)"));
        assert!(!all.contains("1 train") && !all.contains("later"));
    }

    #[test]
    fn filter_prompt_replaces_the_key_help() {
        let mut app = fixture();
        app.input = Some("bo".into());
        let buf = draw(&app, WIDTH, 16);
        assert!(row(&buf, 15).starts_with("/ user filter: bo▏"));
    }

    #[test]
    fn disconnected_without_data_says_retrying() {
        let mut app = App::new("http://ctl:7631", Window::default(), NOW);
        app.apply(Update::Error("connection refused".into()));
        let all = buffer_text(&draw(&app, 100, 12));
        assert!(all.contains("✗ disconnected, retrying: connection refused"));
        assert!(all.contains("disconnected, retrying… (connection refused)"));
    }

    #[test]
    fn disconnected_with_data_keeps_the_stale_plan() {
        let mut app = fixture();
        app.apply(Update::Error("connection refused".into()));
        app.now = NOW + 90;
        let all = buffer_text(&draw(&app, 120, 16));
        assert!(all.contains("disconnected, retrying") && all.contains("stale 1m"));
        assert!(all.contains("2 sim"));
    }

    #[test]
    fn paused_is_flagged_in_the_header() {
        let mut app = fixture();
        app.paused = true;
        assert!(row(&draw(&app, WIDTH, 16), 0).contains("PAUSED"));
    }

    #[test]
    fn overflowing_lanes_are_summarised_and_tiny_terminals_do_not_panic() {
        let all = buffer_text(&draw(&fixture(), WIDTH, 11));
        assert!(all.contains("more lanes"), "{all}");
        for (w, h) in [(1, 1), (10, 3), (30, 6), (200, 60)] {
            draw(&fixture(), w, h);
        }
    }

    #[test]
    fn once_text_is_plain_and_matches_the_rendered_frame() {
        let app = fixture();
        let out = render_text(&app, WIDTH, 16);
        assert!(!out.contains('\x1b'), "no ANSI escapes in --once output");
        assert_eq!(out.lines().count(), 16);
        assert_eq!(out, buffer_text(&draw(&app, WIDTH, 16)));
        assert!(out.contains("1 train") && out.contains("┊now"));
    }
}
