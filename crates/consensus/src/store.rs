//! Raft storage bridging openraft 0.9 to the FSM.
//!
//! Implements the v1-style [`RaftStorage`] trait and wraps it in openraft's
//! [`Adaptor`], which supplies the v2 sealed traits. [`Store`] keeps the log
//! in memory (restart = fresh cluster; used by tests and throwaway nodes);
//! [`DiskStore`](crate::disk::DiskStore) is the durable implementation of the
//! same trait. Both share the apply path and the snapshot image format below.

use crate::types::{request_of, NodeId, Response, TypeConfig};
use gnomon_core::StateIndex;
use gnomon_fsm::{Fsm, State};
use openraft::entry::RaftPayload;
use openraft::storage::{Adaptor, RaftLogReader, RaftSnapshotBuilder, RaftStorage};
use openraft::{
    Entry, ErrorSubject, ErrorVerb, LogId, LogState, OptionalSend, RaftTypeConfig, Snapshot,
    SnapshotMeta, StorageError, StorageIOError, StoredMembership, Vote,
};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;

pub(crate) type C = TypeConfig;

/// Combined log + state-machine store (RaftStorage v1 style).
pub struct Store {
    log: BTreeMap<u64, Entry<C>>,
    vote: Option<Vote<NodeId>>,
    last_purged: Option<LogId<NodeId>>,
    pub fsm: Arc<Fsm>,
    last_applied: Option<LogId<NodeId>>,
    membership: StoredMembership<NodeId, openraft::BasicNode>,
}

impl Store {
    pub fn new(fsm: Arc<Fsm>) -> Self {
        Store {
            log: BTreeMap::new(),
            vote: None,
            last_purged: None,
            fsm,
            last_applied: None,
            membership: StoredMembership::default(),
        }
    }
}

pub(crate) fn io_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> StorageError<NodeId> {
    StorageIOError::new(ErrorSubject::Store, ErrorVerb::Write, &e).into()
}

/// Full FSM image carried in a Raft snapshot: the state plus its index, so
/// installing it reproduces the exact state (fingerprint-equal), not just the
/// job table. JSON so images written before a `#[serde(default)]` field was
/// added still load.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct FsmImage {
    pub index: StateIndex,
    pub state: State,
}

/// Serialize the FSM's current state as a snapshot image.
#[allow(clippy::result_large_err)]
pub fn encode_image(fsm: &Fsm) -> Result<Vec<u8>, StorageError<NodeId>> {
    image_bytes(fsm).map_err(io_err)
}

pub(crate) fn image_bytes(fsm: &Fsm) -> serde_json::Result<Vec<u8>> {
    let snap = fsm.snapshot();
    serde_json::to_vec(&FsmImage {
        index: snap.index,
        state: snap.state().clone(),
    })
}

/// Replace the FSM's state with a snapshot image.
#[allow(clippy::result_large_err)]
pub fn restore_image(fsm: &Fsm, data: &[u8]) -> Result<(), StorageError<NodeId>> {
    let image: FsmImage = serde_json::from_slice(data).map_err(io_err)?;
    fsm.restore(image.state, image.index);
    Ok(())
}

/// The state-machine half shared by both stores: apply committed entries in
/// log order at their stamped time, tracking last-applied and membership.
pub(crate) fn apply_entries(
    fsm: &Fsm,
    last_applied: &mut Option<LogId<NodeId>>,
    membership: &mut StoredMembership<NodeId, openraft::BasicNode>,
    entries: &[Entry<C>],
) -> Vec<Response> {
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        *last_applied = Some(entry.log_id);
        if let Some(m) = entry.get_membership() {
            *membership = StoredMembership::new(Some(entry.log_id), m.clone());
        }
        let index = match request_of(entry) {
            Some(req) => fsm.apply_at(req.event.clone(), req.at).index,
            None => fsm.index(),
        };
        out.push(Response { index });
    }
    out
}

/// Snapshot metadata for a store's current applied position.
pub(crate) fn snapshot_meta(
    last_applied: Option<LogId<NodeId>>,
    membership: &StoredMembership<NodeId, openraft::BasicNode>,
) -> SnapshotMeta<NodeId, openraft::BasicNode> {
    SnapshotMeta {
        last_log_id: last_applied,
        last_membership: membership.clone(),
        snapshot_id: format!("snap-{}", last_applied.map(|l| l.index).unwrap_or(0)),
    }
}

impl RaftLogReader<C> for Store {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<C>>, StorageError<NodeId>> {
        Ok(self.log.range(range).map(|(_, e)| e.clone()).collect())
    }
}

/// In-memory snapshot builder: the image is captured when the builder is
/// created (under the store's exclusive borrow), so it matches `meta`.
pub struct SnapBuilder {
    meta: SnapshotMeta<NodeId, openraft::BasicNode>,
    data: Result<Vec<u8>, StorageError<NodeId>>,
}

impl RaftSnapshotBuilder<C> for SnapBuilder {
    async fn build_snapshot(&mut self) -> Result<Snapshot<C>, StorageError<NodeId>> {
        let data = self.data.clone()?;
        Ok(Snapshot {
            meta: self.meta.clone(),
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

impl RaftStorage<C> for Store {
    type LogReader = Store;
    type SnapshotBuilder = SnapBuilder;

    async fn save_vote(&mut self, vote: &Vote<NodeId>) -> Result<(), StorageError<NodeId>> {
        self.vote = Some(*vote);
        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<NodeId>>, StorageError<NodeId>> {
        Ok(self.vote)
    }

    async fn get_log_state(&mut self) -> Result<LogState<C>, StorageError<NodeId>> {
        Ok(LogState {
            last_purged_log_id: self.last_purged,
            last_log_id: self.log.values().last().map(|e| e.log_id),
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        Store {
            log: self.log.clone(),
            vote: self.vote,
            last_purged: self.last_purged,
            fsm: self.fsm.clone(),
            last_applied: self.last_applied,
            membership: self.membership.clone(),
        }
    }

    async fn append_to_log<I>(&mut self, entries: I) -> Result<(), StorageError<NodeId>>
    where
        I: IntoIterator<Item = Entry<C>> + OptionalSend,
    {
        for e in entries {
            self.log.insert(e.log_id.index, e);
        }
        Ok(())
    }

    async fn delete_conflict_logs_since(
        &mut self,
        log_id: LogId<NodeId>,
    ) -> Result<(), StorageError<NodeId>> {
        let keys: Vec<u64> = self.log.range(log_id.index..).map(|(k, _)| *k).collect();
        for k in keys {
            self.log.remove(&k);
        }
        Ok(())
    }

    async fn purge_logs_upto(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        self.last_purged = Some(log_id);
        let keys: Vec<u64> = self.log.range(..=log_id.index).map(|(k, _)| *k).collect();
        for k in keys {
            self.log.remove(&k);
        }
        Ok(())
    }

    async fn last_applied_state(
        &mut self,
    ) -> Result<
        (
            Option<LogId<NodeId>>,
            StoredMembership<NodeId, openraft::BasicNode>,
        ),
        StorageError<NodeId>,
    > {
        Ok((self.last_applied, self.membership.clone()))
    }

    async fn apply_to_state_machine(
        &mut self,
        entries: &[Entry<C>],
    ) -> Result<Vec<Response>, StorageError<NodeId>> {
        Ok(apply_entries(
            &self.fsm,
            &mut self.last_applied,
            &mut self.membership,
            entries,
        ))
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        SnapBuilder {
            meta: snapshot_meta(self.last_applied, &self.membership),
            data: encode_image(&self.fsm),
        }
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<<C as RaftTypeConfig>::SnapshotData>, StorageError<NodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<NodeId, openraft::BasicNode>,
        snapshot: Box<<C as RaftTypeConfig>::SnapshotData>,
    ) -> Result<(), StorageError<NodeId>> {
        restore_image(&self.fsm, &snapshot.into_inner())?;
        self.last_applied = meta.last_log_id;
        self.membership = meta.last_membership.clone();
        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<C>>, StorageError<NodeId>> {
        Ok(None)
    }
}

/// Wrap a Store into the (log_store, state_machine) pair Raft expects.
pub fn storage_pair(fsm: Arc<Fsm>) -> (Adaptor<C, Store>, Adaptor<C, Store>) {
    Adaptor::new(Store::new(fsm))
}
