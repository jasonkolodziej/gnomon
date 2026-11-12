//! Raft storage bridging openraft 0.9 to the FSM.
//!
//! Implements the v1-style [`RaftStorage`] trait and wraps it in openraft's
//! [`Adaptor`], which supplies the v2 sealed traits. The log is in-memory in
//! this skeleton (restart = fresh cluster); persistence is a drop-in change
//! behind the same trait.

use crate::types::{event_of, NodeId, Response, TypeConfig};
use openraft::storage::{Adaptor, RaftLogReader, RaftSnapshotBuilder, RaftStorage};
use openraft::{
    Entry, ErrorSubject, ErrorVerb, LogId, LogState, OptionalSend, RaftTypeConfig,
    Snapshot, SnapshotMeta, StorageError, StorageIOError, StoredMembership, Vote,
};
use openraft::entry::RaftPayload;
use gnomon_fsm::Fsm;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;

type C = TypeConfig;

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

fn io_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> StorageError<NodeId> {
    StorageIOError::new(ErrorSubject::Store, ErrorVerb::Write, &e).into()
}

impl RaftLogReader<C> for Store {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self, range: RB,
    ) -> Result<Vec<Entry<C>>, StorageError<NodeId>> {
        Ok(self.log.range(range).map(|(_, e)| e.clone()).collect())
    }
}

pub struct SnapBuilder {
    fsm: Arc<Fsm>,
    last_applied: Option<LogId<NodeId>>,
    membership: StoredMembership<NodeId, openraft::BasicNode>,
}

impl RaftSnapshotBuilder<C> for SnapBuilder {
    async fn build_snapshot(&mut self) -> Result<Snapshot<C>, StorageError<NodeId>> {
        let snap = self.fsm.snapshot();
        let data = bincode::serialize(&snap.state().jobs).map_err(io_err)?;
        let meta = SnapshotMeta {
            last_log_id: self.last_applied,
            last_membership: self.membership.clone(),
            snapshot_id: format!("snap-{}", self.last_applied.map(|l| l.index).unwrap_or(0)),
        };
        Ok(Snapshot { meta, snapshot: Box::new(Cursor::new(data)) })
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
    where I: IntoIterator<Item = Entry<C>> + OptionalSend {
        for e in entries { self.log.insert(e.log_id.index, e); }
        Ok(())
    }

    async fn delete_conflict_logs_since(&mut self, log_id: LogId<NodeId>)
        -> Result<(), StorageError<NodeId>>
    {
        let keys: Vec<u64> = self.log.range(log_id.index..).map(|(k, _)| *k).collect();
        for k in keys { self.log.remove(&k); }
        Ok(())
    }

    async fn purge_logs_upto(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        self.last_purged = Some(log_id);
        let keys: Vec<u64> = self.log.range(..=log_id.index).map(|(k, _)| *k).collect();
        for k in keys { self.log.remove(&k); }
        Ok(())
    }

    async fn last_applied_state(&mut self)
        -> Result<(Option<LogId<NodeId>>, StoredMembership<NodeId, openraft::BasicNode>), StorageError<NodeId>>
    {
        Ok((self.last_applied, self.membership.clone()))
    }

    async fn apply_to_state_machine(&mut self, entries: &[Entry<C>]) -> Result<Vec<Response>, StorageError<NodeId>> {
        let mut out = Vec::new();
        for entry in entries {
            self.last_applied = Some(entry.log_id);
            if let Some(m) = entry.get_membership() {
                self.membership = StoredMembership::new(Some(entry.log_id), m.clone());
            }
            if let Some(ev) = event_of(entry) {
                let idx = self.fsm.apply(ev.clone());
                out.push(Response { index: idx });
            } else {
                out.push(Response { index: self.fsm.index() });
            }
        }
        Ok(out)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        SnapBuilder {
            fsm: self.fsm.clone(),
            last_applied: self.last_applied,
            membership: self.membership.clone(),
        }
    }

    async fn begin_receiving_snapshot(&mut self)
        -> Result<Box<<C as RaftTypeConfig>::SnapshotData>, StorageError<NodeId>>
    {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(&mut self, meta: &SnapshotMeta<NodeId, openraft::BasicNode>,
        snapshot: Box<<C as RaftTypeConfig>::SnapshotData>) -> Result<(), StorageError<NodeId>>
    {
        let jobs: BTreeMap<gnomon_core::JobId, gnomon_core::Job> =
            bincode::deserialize(&snapshot.into_inner()).map_err(io_err)?;
        for (_, job) in jobs {
            self.fsm.apply(gnomon_core::Event::JobSubmitted(job));
        }
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
