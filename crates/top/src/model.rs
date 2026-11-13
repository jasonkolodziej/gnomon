//! Client-side snapshot of the cluster plan, built from one `ListJobs` +
//! `ListNodes` round trip. Pure data: no IO, so every view can be rendered
//! from a fixed model in tests.

use gnomon_api::pb::{ListJobsReply, ListNodesReply};

/// Job lifecycle as reported on the wire (`JobInfo.state`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum JobState {
    Pending,
    Reserved,
    Running,
    Completed,
    Cancelled,
    Failed,
    /// A state name this client does not know (newer controller).
    Unknown,
}

impl JobState {
    /// Every state in header display order.
    pub const ALL: [JobState; 7] = [
        JobState::Running,
        JobState::Reserved,
        JobState::Pending,
        JobState::Completed,
        JobState::Failed,
        JobState::Cancelled,
        JobState::Unknown,
    ];

    /// Parses the API's upper-case state name.
    pub fn parse(s: &str) -> Self {
        match s {
            "PENDING" => JobState::Pending,
            "RESERVED" => JobState::Reserved,
            "RUNNING" => JobState::Running,
            "COMPLETED" => JobState::Completed,
            "CANCELLED" => JobState::Cancelled,
            "FAILED" => JobState::Failed,
            _ => JobState::Unknown,
        }
    }

    /// Short header label.
    pub fn short(self) -> &'static str {
        match self {
            JobState::Pending => "PEND",
            JobState::Reserved => "RES",
            JobState::Running => "RUN",
            JobState::Completed => "DONE",
            JobState::Cancelled => "CANC",
            JobState::Failed => "FAIL",
            JobState::Unknown => "UNK",
        }
    }

    /// Whether the job occupies the time axis (a running span or a future
    /// backfill reservation, report section 5.2).
    pub fn is_planned(self) -> bool {
        matches!(self, JobState::Running | JobState::Reserved)
    }
}

/// One node row: capacity, current allocation and liveness.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeView {
    pub name: String,
    pub cores: u32,
    pub cores_alloc: u32,
    pub gpus: u32,
    pub gpus_alloc: u32,
    pub alive: bool,
}

/// One job as gntop needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct JobView {
    pub id: u64,
    pub name: String,
    pub user: String,
    pub state: JobState,
    pub priority: u64,
    pub submit_time: u64,
    pub cores: u32,
    /// Planned (or actual) start, epoch seconds; `None` until placed.
    pub planned_start: Option<u64>,
    /// Planned span length in seconds (walltime when not predicted).
    pub planned_dur: u64,
    /// Nodes holding the job's span; empty when unknown.
    pub nodes: Vec<String>,
}

impl JobView {
    /// The `[start, end)` span to draw, or `None` for jobs off the time axis.
    /// A running job that outlived its planned span is drawn up to `now`.
    pub fn span(&self, now: u64) -> Option<(u64, u64)> {
        if !self.state.is_planned() {
            return None;
        }
        let start = self.planned_start?;
        let mut end = start.saturating_add(self.planned_dur.max(1));
        if self.state == JobState::Running {
            end = end.max(now);
        }
        Some((start, end))
    }

    /// Seconds the job has been waiting since submission.
    pub fn wait_secs(&self, now: u64) -> u64 {
        now.saturating_sub(self.submit_time)
    }
}

/// One poll's worth of cluster state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClusterSnapshot {
    /// Client wall clock when the snapshot was fetched (epoch seconds).
    pub fetched_at: u64,
    /// The API's FSM state index (Raft-style, report section 4.3).
    pub state_index: u64,
    pub nodes: Vec<NodeView>,
    pub jobs: Vec<JobView>,
}

impl ClusterSnapshot {
    /// Converts the two API replies into the client model.
    pub fn from_replies(fetched_at: u64, jobs: ListJobsReply, nodes: ListNodesReply) -> Self {
        let nodes = nodes
            .nodes
            .into_iter()
            .map(|n| {
                let cap = n.capacity.unwrap_or_default();
                let alloc = n.allocated.unwrap_or_default();
                NodeView {
                    name: n.name,
                    cores: cap.cores,
                    cores_alloc: alloc.cores,
                    gpus: cap.gpus,
                    gpus_alloc: alloc.gpus,
                    alive: n.alive,
                }
            })
            .collect();
        let state_index = jobs.state_index;
        let jobs = jobs
            .jobs
            .into_iter()
            .map(|j| {
                let planned_dur = if j.planned_dur > 0 {
                    j.planned_dur
                } else {
                    j.walltime_secs
                };
                JobView {
                    id: j.id,
                    state: JobState::parse(&j.state),
                    priority: j.priority,
                    submit_time: j.submit_time,
                    cores: j.request.map(|r| r.cores).unwrap_or(0),
                    planned_start: (j.planned_start > 0).then_some(j.planned_start),
                    planned_dur,
                    nodes: j.nodes,
                    name: j.name,
                    user: j.user,
                }
            })
            .collect();
        ClusterSnapshot {
            fetched_at,
            state_index,
            nodes,
            jobs,
        }
    }
}

/// Number of jobs per state, in [`JobState::ALL`] order.
pub fn state_counts<'a>(jobs: impl Iterator<Item = &'a JobView>) -> Vec<(JobState, usize)> {
    let mut counts: Vec<(JobState, usize)> = JobState::ALL.iter().map(|&s| (s, 0)).collect();
    for j in jobs {
        if let Some(c) = counts.iter_mut().find(|c| c.0 == j.state) {
            c.1 += 1;
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnomon_api::pb::{JobInfo, NodeInfo, Resources};

    fn job_info(id: u64, state: &str, planned_start: u64, dur: u64, wall: u64) -> JobInfo {
        JobInfo {
            id,
            name: format!("j{id}"),
            user: "alice".into(),
            state: state.into(),
            request: Some(Resources {
                cores: 4,
                mem_mb: 0,
                gpus: 0,
            }),
            walltime_secs: wall,
            planned_start,
            planned_dur: dur,
            nodes: vec!["n0".into()],
            ..Default::default()
        }
    }

    #[test]
    fn replies_convert_with_walltime_fallback_and_unplaced_start() {
        let jobs = ListJobsReply {
            jobs: vec![
                job_info(1, "RUNNING", 100, 50, 900),
                job_info(2, "PENDING", 0, 0, 300),
            ],
            state_index: 42,
        };
        let nodes = ListNodesReply {
            nodes: vec![NodeInfo {
                name: "n0".into(),
                capacity: Some(Resources {
                    cores: 8,
                    mem_mb: 0,
                    gpus: 2,
                }),
                allocated: Some(Resources {
                    cores: 4,
                    mem_mb: 0,
                    gpus: 1,
                }),
                last_heartbeat: 0,
                alive: true,
            }],
        };
        let s = ClusterSnapshot::from_replies(7, jobs, nodes);
        assert_eq!(s.state_index, 42);
        assert_eq!(s.jobs[0].planned_dur, 50);
        assert_eq!(s.jobs[0].planned_start, Some(100));
        assert_eq!(s.jobs[1].planned_dur, 300, "walltime fallback");
        assert_eq!(s.jobs[1].planned_start, None);
        assert_eq!(s.nodes[0].cores_alloc, 4);
        assert_eq!(s.nodes[0].gpus, 2);
        let counts = state_counts(s.jobs.iter());
        assert!(counts.contains(&(JobState::Running, 1)));
        assert!(counts.contains(&(JobState::Pending, 1)));
    }

    #[test]
    fn running_span_extends_to_now_on_overrun_and_pending_has_none() {
        let s = ClusterSnapshot::from_replies(
            0,
            ListJobsReply {
                jobs: vec![
                    job_info(1, "RUNNING", 100, 50, 900),
                    job_info(2, "RESERVED", 500, 60, 900),
                    job_info(3, "PENDING", 0, 0, 900),
                ],
                state_index: 0,
            },
            ListNodesReply::default(),
        );
        assert_eq!(s.jobs[0].span(120), Some((100, 150)));
        assert_eq!(s.jobs[0].span(200), Some((100, 200)));
        assert_eq!(s.jobs[1].span(200), Some((500, 560)));
        assert_eq!(s.jobs[2].span(200), None);
    }

    #[test]
    fn unknown_state_names_are_tolerated() {
        assert_eq!(JobState::parse("SUSPENDED"), JobState::Unknown);
        assert_eq!(JobState::parse("RESERVED"), JobState::Reserved);
    }
}
