//! Core domain types shared by every gnomon component.
//!
//! Vocabulary deliberately mirrors Slurm (`sbatch`-style jobs, partitions,
//! walltime) so the migration surface in the report stays honest, while the
//! data model is the graph-based one proposed in section 4.2.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Monotonic job identifier (Raft log index compatible).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct JobId(pub u64);

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Slurm-style job lifecycle. Transitions only happen through FSM events,
/// so every state change is replicated and replayable.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum JobState {
    Pending,
    /// Backfilled: has a reservation but start time is in the future.
    Reserved,
    Running,
    Completed,
    Cancelled,
    Failed,
}

/// Multi-dimensional resource request. Unlike Slurm's GRES side-attributes,
/// every dimension is first-class and matched inside the resource graph.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Resources {
    pub cores: u32,
    pub mem_mb: u64,
    pub gpus: u32,
}

impl Resources {
    pub fn fits_in(&self, avail: &Resources) -> bool {
        self.cores <= avail.cores && self.mem_mb <= avail.mem_mb && self.gpus <= avail.gpus
    }
    pub fn saturating_sub(&self, other: &Resources) -> Resources {
        Resources {
            cores: self.cores.saturating_sub(other.cores),
            mem_mb: self.mem_mb.saturating_sub(other.mem_mb),
            gpus: self.gpus.saturating_sub(other.gpus),
        }
    }
    pub fn add(&self, other: &Resources) -> Resources {
        Resources {
            cores: self.cores + other.cores,
            mem_mb: self.mem_mb + other.mem_mb,
            gpus: self.gpus + other.gpus,
        }
    }
    pub fn is_zero(&self) -> bool {
        self.cores == 0 && self.mem_mb == 0 && self.gpus == 0
    }
}

/// Piecewise-constant capacity over time (power caps, demand response,
/// carbon windows). `steps` are `(t, capacity)` breakpoints; with `period`
/// set they repeat (e.g. daily, `t` = seconds into the day). Capacity at a
/// time is the last step at or before it (wrapping for periodic profiles);
/// before the first step of a non-periodic profile it is the vertex's base
/// capacity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CapacityProfile {
    pub period: Option<u64>,
    pub steps: Vec<(u64, Resources)>,
}

impl CapacityProfile {
    pub fn daily(steps: Vec<(u64, Resources)>) -> Self {
        let mut steps = steps;
        steps.sort_by_key(|s| s.0);
        CapacityProfile {
            period: Some(86_400),
            steps,
        }
    }

    pub fn at(&self, base: &Resources, t: u64) -> Resources {
        if self.steps.is_empty() {
            return *base;
        }
        match self.period {
            Some(p) if p > 0 => {
                let ph = t % p;
                match self.steps.iter().rev().find(|s| s.0 <= ph) {
                    Some(s) => s.1,
                    None => self.steps.last().unwrap().1, // wrapped from previous period
                }
            }
            _ => self
                .steps
                .iter()
                .rev()
                .find(|s| s.0 <= t)
                .map(|s| s.1)
                .unwrap_or(*base),
        }
    }

    /// Breakpoints strictly inside `(a, b)`.
    pub fn breakpoints(&self, a: u64, b: u64, out: &mut Vec<u64>) {
        match self.period {
            Some(p) if p > 0 => {
                let mut k = a / p;
                while k * p < b {
                    for s in &self.steps {
                        let t = k * p + s.0;
                        if t > a && t < b {
                            out.push(t);
                        }
                    }
                    k += 1;
                }
            }
            _ => out.extend(self.steps.iter().map(|s| s.0).filter(|&t| t > a && t < b)),
        }
    }
}

/// Walltime estimate; drives backfill reservations (the "time axis" that
/// section 5.2 identifies as the novel optimistic-concurrency target).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Walltime(pub Duration);

/// A submitted job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub name: String,
    pub user: String,
    pub partition: String,
    pub state: JobState,
    pub request: Resources,
    pub walltime: Walltime,
    /// Slurm-multifactor-style scalar priority; policy layer computes it.
    pub priority: u64,
    pub submit_time: u64,
    /// Set by the scheduler: earliest start computed by backfill.
    pub planned_start: Option<u64>,
    /// Planned span length (runtime prediction or walltime).
    #[serde(default)]
    pub planned_dur: Option<u64>,
    /// Command line the agent executes (phase 4: real launches).
    pub command: String,
    /// Gang size (Slurm `-N/--nodes`): the job needs this many distinct node
    /// vertices, each with `request` free for the whole span, all starting
    /// together. `request` is *per node*. 0 is treated as 1 (see
    /// [`Job::node_count`]).
    #[serde(default = "one")]
    pub nodes: u32,
}

fn one() -> u32 {
    1
}

impl Job {
    pub fn new(id: JobId, name: &str, user: &str, request: Resources, walltime: Walltime) -> Self {
        Job {
            id,
            name: name.into(),
            user: user.into(),
            partition: "normal".into(),
            state: JobState::Pending,
            request,
            walltime,
            priority: 0,
            submit_time: now_secs(),
            planned_start: None,
            planned_dur: None,
            command: String::new(),
            nodes: 1,
        }
    }

    /// Effective gang size: `nodes`, with 0 treated as 1.
    pub fn node_count(&self) -> u32 {
        self.nodes.max(1)
    }

    /// Whether this is a multi-node gang job (`nodes > 1`).
    pub fn is_gang(&self) -> bool {
        self.node_count() > 1
    }
}

/// Every mutation of cluster state is an Event. The FSM applies them in
/// Raft-log order; the accounting sink and the replayable journal both
/// consume the same stream (report section 4.1).
///
/// `dur` on start/reserve is the *planned* span length the scheduler chose
/// (a runtime prediction when distribution-aware backfill is on); `None`
/// means "plan for the full walltime", the classic behaviour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    JobSubmitted(Job),
    JobCancelled(JobId),
    /// Immediate start on the given graph vertices at `start` time.
    JobStarted {
        job: JobId,
        vertices: Vec<u32>,
        start: u64,
        #[serde(default)]
        dur: Option<u64>,
    },
    /// Backfill reservation: earliest start is `start` on `vertices`.
    JobReserved {
        job: JobId,
        vertices: Vec<u32>,
        start: u64,
        #[serde(default)]
        dur: Option<u64>,
    },
    /// Releases the job's planner spans at `end` (early completion frees
    /// the rest of the reserved window for backfill).
    JobFinished {
        job: JobId,
        end: u64,
        code: i32,
    },
    /// One rank of a running gang job exited on `node`. The FSM aggregates
    /// these: the job finishes (spans released at the latest rank's `end`)
    /// once every rank has reported, and is `Failed` if any rank exited
    /// non-zero. For a single-node job this is equivalent to `JobFinished`.
    JobRankFinished {
        job: JobId,
        node: String,
        end: u64,
        code: i32,
    },
    /// A running job outlived its planned span: extend it to `new_end`
    /// (never past walltime). Emitted by distribution-aware planning.
    JobExtended {
        job: JobId,
        new_end: u64,
    },
    /// Drop every not-yet-started reservation and return those jobs to
    /// Pending so the next pass can re-plan (conservative backfill with
    /// compression).
    ReservationsCleared {
        now: u64,
    },
    NodeRegistered {
        name: String,
        capacity: Resources,
    },
    /// An agent fetched the first `count` queued assignments for `node`;
    /// drop them so work is dispatched exactly once.
    AssignmentsFetched {
        node: String,
        count: usize,
    },
    /// Set (or clear) a node's time-varying capacity (power cap, carbon
    /// window). Replicated like any other change, so replay honours it.
    NodeProfileSet {
        name: String,
        profile: Option<CapacityProfile>,
    },
    Heartbeat {
        name: String,
        allocated: Resources,
    },
}

/// Time source. Production uses wall-clock time; replay and simulation use a
/// manual clock so scheduling is a deterministic function of the event log.
pub trait Clock: Send + Sync + fmt::Debug {
    fn now(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> u64 {
        now_secs()
    }
}

/// Manually advanced clock (simulation / replay).
#[derive(Debug, Default)]
pub struct ManualClock(std::sync::atomic::AtomicU64);
impl ManualClock {
    pub fn new(t: u64) -> Self {
        ManualClock(std::sync::atomic::AtomicU64::new(t))
    }
    pub fn set(&self, t: u64) {
        self.0.store(t, std::sync::atomic::Ordering::Release)
    }
}
impl Clock for ManualClock {
    fn now(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// The Raft index of the last applied event — the optimistic-concurrency
/// token. A plan computed against index N is committed only if the FSM is
/// still at N (report section 4.3, Omega/Nomad pattern).
pub type StateIndex = u64;

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Walltime {
    pub fn secs(&self) -> u64 {
        self.0.as_secs()
    }
}

/// Node agent registry entry kept in the FSM (phase 4).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NodeInfo {
    pub name: String,
    pub capacity: Resources,
    pub allocated: Resources,
    pub last_heartbeat: u64,
}

/// Per-node work assignment handed to agents via FetchWork.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Assignment {
    pub job: JobId,
    pub command: String,
    pub start: u64,
    /// Rank of this node within the job (0 for single-node jobs); ranks follow
    /// the order of the job's placement vertices.
    #[serde(default)]
    pub rank: u32,
    /// Every node of the job in rank order (Slurm's `SLURM_JOB_NODELIST`).
    #[serde(default)]
    pub nodelist: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_nodes_defaults_to_one_for_old_records() {
        let mut v = serde_json::to_value(Job::new(
            JobId(1),
            "j",
            "u",
            Resources::default(),
            Walltime(Duration::from_secs(1)),
        ))
        .unwrap();
        v.as_object_mut().unwrap().remove("nodes");
        let j: Job = serde_json::from_value(v).unwrap();
        assert_eq!(j.nodes, 1);
        assert!(!j.is_gang());
    }

    #[test]
    fn zero_nodes_counts_as_one() {
        let mut j = Job::new(
            JobId(1),
            "j",
            "u",
            Resources::default(),
            Walltime(Duration::from_secs(1)),
        );
        j.nodes = 0;
        assert_eq!(j.node_count(), 1);
        j.nodes = 4;
        assert_eq!(j.node_count(), 4);
        assert!(j.is_gang());
    }

    #[test]
    fn assignment_rank_fields_default_for_old_records() {
        let a: Assignment = serde_json::from_str(r#"{"job":3,"command":"x","start":5}"#).unwrap();
        assert_eq!(a.rank, 0);
        assert!(a.nodelist.is_empty());
    }
}
