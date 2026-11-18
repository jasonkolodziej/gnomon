//! The JSON that `gnomon-web` serves and its browser UI reads.
//!
//! Serde only, so the UI (a separate WASM workspace) can depend on it without
//! pulling in tonic or the scheduler. `gnomon-web` converts the controller's
//! gRPC replies into these types; the field names are the wire format.
//!
//! | Endpoint | Body |
//! |---|---|
//! | `GET /api/state` | [`ClusterView`] |
//! | `GET /api/stream` (SSE) | `event: state` → [`ClusterView`]; `event: events` → `Vec<`[`EventView`]`>` |
//! | `POST /api/jobs` | [`SubmitForm`] → [`ActionReply`] |
//! | `POST /api/jobs/{id}/cancel` | → [`ActionReply`] |
//! | `POST /api/lab` | [`LabRequest`] → [`LabReport`] |

use serde::{Deserialize, Serialize};

/// Everything the UI draws, as of one controller snapshot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClusterView {
    /// The controller address this view came from.
    pub controller: String,
    /// False while no controller answers; `error` says why.
    pub connected: bool,
    #[serde(default)]
    pub error: Option<String>,
    /// Server clock when the view was taken (Unix seconds).
    pub now: u64,
    /// The FSM state index of the snapshot.
    pub state_index: u64,
    /// False when the web server refuses job submission and cancellation.
    pub writable: bool,
    pub cluster: ControllerView,
    pub policy: PolicyView,
    pub nodes: Vec<NodeView>,
    pub jobs: Vec<JobView>,
}

/// Standalone controller, or one replica's view of its Raft group.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ControllerView {
    pub replicated: bool,
    pub node_id: u64,
    /// standalone, leader, follower, candidate, learner or shutdown.
    pub role: String,
    pub leader: Option<u64>,
    pub term: u64,
    pub last_log_index: u64,
    pub last_applied: u64,
    pub fsm_index: u64,
    pub members: Vec<MemberView>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MemberView {
    pub id: u64,
    pub addr: String,
    pub leader: bool,
    /// The leader's view of this member's replicated log index.
    pub matched: Option<u64>,
}

/// The scheduling policy and its WASM plugins (ADR 0002-0004).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PolicyView {
    pub name: String,
    pub fuel: u64,
    pub memory_mb: u64,
    pub plugins: Vec<PluginView>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginView {
    /// admission, priority, declarative, shadow-priority, estimator or node-score.
    pub world: String,
    pub source: String,
    pub mode: String,
    /// Counter name and value, sorted by name.
    pub counters: Vec<(String, u64)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeView {
    pub name: String,
    pub cores: u32,
    pub mem_mb: u64,
    pub gpus: u32,
    /// Allocation the agent last reported in a heartbeat.
    pub used_cores: u32,
    pub used_mem_mb: u64,
    pub used_gpus: u32,
    pub last_heartbeat: u64,
    pub alive: bool,
    /// Jobs whose rank here is on the kill channel.
    pub kill_pending: Vec<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JobView {
    pub id: u64,
    pub name: String,
    pub user: String,
    pub partition: String,
    /// PENDING, RESERVED, RUNNING, COMPLETED, CANCELLED or FAILED.
    pub state: String,
    pub cores: u32,
    pub mem_mb: u64,
    pub gpus: u32,
    /// Gang size (1 for single-node jobs).
    pub num_nodes: u32,
    pub walltime: u64,
    pub priority: u64,
    pub submit_time: u64,
    /// Planned (or actual) start, 0 while unplanned.
    pub start: u64,
    /// Planned span in seconds (runtime prediction or walltime).
    pub span: u64,
    /// Nodes held, in rank order (empty until placed).
    pub nodes: Vec<String>,
    pub container: String,
}

impl JobView {
    pub fn is_active(&self) -> bool {
        matches!(self.state.as_str(), "PENDING" | "RESERVED" | "RUNNING")
    }
}

/// One state-machine event from the controller's event tail.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EventView {
    pub index: u64,
    pub at: u64,
    pub kind: String,
    pub job: Option<u64>,
    pub node: String,
    pub detail: String,
    pub accepted: bool,
}

/// A job submission from the UI (the same fields `gnbatch` sends).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SubmitForm {
    pub name: String,
    pub user: String,
    #[serde(default)]
    pub partition: String,
    pub cores: u32,
    #[serde(default)]
    pub mem_mb: u64,
    #[serde(default)]
    pub gpus: u32,
    #[serde(default = "one")]
    pub nodes: u32,
    pub walltime_secs: u64,
    #[serde(default)]
    pub priority: u64,
    pub command: String,
    #[serde(default)]
    pub container: String,
}

fn one() -> u32 {
    1
}

/// Result of a submit or cancel.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionReply {
    pub ok: bool,
    pub job: Option<u64>,
    pub message: String,
}

/// A policy comparison on a synthetic workload, run in the web server with
/// the production scheduling code (`gnomon-sim`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LabRequest {
    pub nodes: u32,
    pub cores_per_node: u32,
    pub jobs: u32,
    /// Every n-th job is a 2-8 node gang (0: none).
    pub gang_every: u32,
    pub seed: u64,
}

impl Default for LabRequest {
    fn default() -> Self {
        LabRequest {
            nodes: 16,
            cores_per_node: 16,
            jobs: 600,
            gang_every: 7,
            seed: 42,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LabReport {
    pub request: LabRequest,
    pub rows: Vec<LabRow>,
    /// Wall-clock time the comparison took on the server.
    pub millis: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LabRow {
    pub policy: String,
    pub label: String,
    pub mean_wait_h: f64,
    pub p95_wait_h: f64,
    pub mean_bsld: f64,
    pub wide_wait_h: f64,
    pub utilization: f64,
    pub makespan_h: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_form_defaults_fill_optional_fields() {
        let f: SubmitForm = serde_json_like(
            r#"{"name":"a","user":"u","cores":2,"walltime_secs":60,"command":"true"}"#,
        );
        assert_eq!((f.nodes, f.gpus, f.mem_mb), (1, 0, 0));
        assert!(f.container.is_empty());
    }

    /// Tiny shim so the model crate needs no serde_json at runtime.
    fn serde_json_like<T: for<'de> Deserialize<'de>>(s: &str) -> T {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn active_states() {
        let mut j = JobView::default();
        for (s, active) in [("PENDING", true), ("RUNNING", true), ("FAILED", false)] {
            j.state = s.into();
            assert_eq!(j.is_active(), active, "{s}");
        }
    }
}
