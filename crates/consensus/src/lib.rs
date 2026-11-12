//! Phase 2: embedded Raft replication for the FSM (report section 4.1).
//!
//! `openraft` drives a replicated log of [`Event`]s; committed entries are
//! applied to the [`Fsm`] in log order — exactly the `apply()` path the
//! single-node skeleton already used. Leader election replaces Slurm's
//! passive backup controller.

use openraft::{Config, Raft};
use std::sync::Arc;

pub mod store;
pub mod types;

use types::{NodeId, Request, TypeConfig};

pub type SchedulerRaft = Raft<TypeConfig>;

/// A replicated controller node: Raft instance + the FSM it feeds.
pub struct ConsensusNode {
    pub raft: SchedulerRaft,
    pub fsm: Arc<gnomon_fsm::Fsm>,
}

/// Propose a cluster mutation (client_write on the leader).
pub async fn propose(
    raft: &SchedulerRaft,
    ev: gnomon_core::Event,
) -> Result<
    openraft::raft::ClientWriteResponse<TypeConfig>,
    openraft::error::RaftError<
        NodeId,
        openraft::error::ClientWriteError<NodeId, openraft::BasicNode>,
    >,
> {
    raft.client_write(Request(ev)).await
}

pub fn default_config() -> Arc<Config> {
    Arc::new(
        Config {
            heartbeat_interval: 250,
            election_timeout_min: 800,
            election_timeout_max: 1600,
            ..Default::default()
        }
        .validate()
        .expect("valid raft config"),
    )
}
