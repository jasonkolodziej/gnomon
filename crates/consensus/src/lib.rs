//! Phase 2: embedded Raft replication for the FSM (report section 4.1).
//!
//! `openraft` drives a replicated log of [`Event`](gnomon_core::Event)s; committed entries are
//! applied to the [`Fsm`](gnomon_fsm::Fsm) in log order — exactly the `apply()` path the
//! single-node skeleton already used. Leader election replaces Slurm's
//! passive backup controller.
//!
//! Persistence:
//! - [`disk::DiskStore`] / [`disk::storage_pair_on_disk`]: durable Raft
//!   storage — fsync'd, checksummed log; atomically replaced vote, commit
//!   index and snapshot; truncation and purge that survive restart; on open
//!   the FSM is rebuilt (snapshot + committed entries) to its exact pre-crash
//!   state.
//! - [`journal::EventJournal`]: the durable event journal the single
//!   controller (`gnomon-ctl --state-dir`) uses today, on the same framed log.
//! - [`store::Store`]: the in-memory store (tests, throwaway nodes).
//!
//! Not yet built: the Raft network transport and multi-node replication, and
//! routing the controller's writes through `client_write`.

use openraft::{Config, Raft};
use std::sync::Arc;

pub mod disk;
pub mod journal;
pub mod store;
pub mod types;
pub mod wal;

use types::{NodeId, Request, TypeConfig};

pub type SchedulerRaft = Raft<TypeConfig>;

/// A replicated controller node: Raft instance + the FSM it feeds.
pub struct ConsensusNode {
    pub raft: SchedulerRaft,
    pub fsm: Arc<gnomon_fsm::Fsm>,
}

/// Propose a cluster mutation (client_write on the leader), stamped with the
/// system clock.
#[allow(clippy::result_large_err)]
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
    use gnomon_core::Clock;
    propose_at(raft, ev, gnomon_core::SystemClock.now()).await
}

/// Propose a mutation that every replica applies as if the clock read `at`
/// (the leader passes its FSM clock; tests pass fixed times).
#[allow(clippy::result_large_err)]
pub async fn propose_at(
    raft: &SchedulerRaft,
    ev: gnomon_core::Event,
    at: u64,
) -> Result<
    openraft::raft::ClientWriteResponse<TypeConfig>,
    openraft::error::RaftError<
        NodeId,
        openraft::error::ClientWriteError<NodeId, openraft::BasicNode>,
    >,
> {
    raft.client_write(Request::new(ev, at)).await
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
