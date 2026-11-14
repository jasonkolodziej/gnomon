//! Durable Raft storage: the [`RaftStorage`] implementation a controller
//! restarts from (report section 4.1: the FSM is a function of the log).
//!
//! Layout under the store directory:
//!
//! | file       | contents                                   | write discipline       |
//! |------------|--------------------------------------------|------------------------|
//! | `log`      | one framed JSON `Entry` per record          | append + `sync_data`   |
//! | `vote`     | the current `Vote`                          | atomic replace         |
//! | `meta`     | committed + last-purged log ids             | atomic replace         |
//! | `snapshot` | latest snapshot meta + FSM image            | atomic replace         |
//!
//! Conflict truncation cuts the log file at the first conflicting record's
//! offset (`set_len` + fsync). Purge records `last_purged` in `meta` first,
//! then compacts the log file by an atomic rewrite; a crash in between is
//! harmless because recovery drops entries at or below `last_purged`.
//!
//! On [`DiskStore::open`] the FSM (which must be fresh) is rebuilt from the
//! snapshot image plus every committed entry after it, so it equals the state
//! before the crash and openraft has nothing left to re-apply. Every entry is
//! applied at the timestamp the leader stamped on it, which is what makes the
//! rebuilt state fingerprint-equal rather than merely similar.
//!
//! Not yet covered: the network transport and multi-node replication
//! (the store is replication-ready, but only single-node Raft is wired).

use crate::store::{apply_entries, encode_image, io_err, snapshot_meta, FsmImage, C};
use crate::types::{NodeId, Response};
use crate::wal::{atomic_write, data_err, read_atomic, FramedLog};
use gnomon_fsm::Fsm;
use openraft::storage::{Adaptor, RaftLogReader, RaftSnapshotBuilder, RaftStorage};
use openraft::{
    BasicNode, Entry, LogId, LogState, OptionalSend, RaftTypeConfig, Snapshot, SnapshotMeta,
    StorageError, StoredMembership, Vote,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::{self, Cursor};
use std::ops::RangeBounds;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

type Membership = StoredMembership<NodeId, BasicNode>;
type Meta = SnapshotMeta<NodeId, BasicNode>;
type SharedLog = Arc<RwLock<BTreeMap<u64, Entry<C>>>>;

const LOG: &str = "log";
const VOTE: &str = "vote";
const META: &str = "meta";
const SNAPSHOT: &str = "snapshot";

/// Committed and last-purged ids, persisted together in `meta`.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
struct LogMeta {
    committed: Option<LogId<NodeId>>,
    last_purged: Option<LogId<NodeId>>,
}

/// A persisted snapshot: metadata plus the encoded [`FsmImage`].
#[derive(Clone)]
struct StoredSnapshot {
    meta: Meta,
    data: Vec<u8>,
}

impl StoredSnapshot {
    /// `meta_len: u32 LE | meta JSON | image bytes` (no JSON-escaping of the image).
    fn encode(&self) -> io::Result<Vec<u8>> {
        let meta = serde_json::to_vec(&self.meta).map_err(data_err)?;
        let len = u32::try_from(meta.len()).map_err(data_err)?;
        let mut out = Vec::with_capacity(4 + meta.len() + self.data.len());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&meta);
        out.extend_from_slice(&self.data);
        Ok(out)
    }

    fn decode(buf: &[u8]) -> io::Result<StoredSnapshot> {
        let len_bytes = buf.get(..4).ok_or_else(|| data_err("snapshot too short"))?;
        let len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
        let meta = buf
            .get(4..4 + len)
            .ok_or_else(|| data_err("snapshot meta truncated"))?;
        Ok(StoredSnapshot {
            meta: serde_json::from_slice(meta).map_err(data_err)?,
            data: buf[4 + len..].to_vec(),
        })
    }

    fn save(&self, dir: &Path) -> io::Result<()> {
        atomic_write(&dir.join(SNAPSHOT), &self.encode()?)
    }

    fn to_snapshot(&self) -> Snapshot<C> {
        Snapshot {
            meta: self.meta.clone(),
            snapshot: Box::new(Cursor::new(self.data.clone())),
        }
    }
}

/// File-backed log + vote + snapshot store driving an [`Fsm`].
pub struct DiskStore {
    dir: PathBuf,
    wal: FramedLog,
    log: SharedLog,
    /// Byte offset of each retained entry's record in `log`.
    offsets: BTreeMap<u64, u64>,
    vote: Option<Vote<NodeId>>,
    meta: LogMeta,
    pub fsm: Arc<Fsm>,
    last_applied: Option<LogId<NodeId>>,
    membership: Membership,
    snapshot: Arc<Mutex<Option<StoredSnapshot>>>,
    /// Exclusive lock on `dir`, held for the store's lifetime.
    _lock: std::fs::File,
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<Option<T>> {
    read_atomic(path)?
        .map(|b| serde_json::from_slice(&b).map_err(data_err))
        .transpose()
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> io::Result<()> {
    atomic_write(path, &serde_json::to_vec(v).map_err(data_err)?)
}

impl DiskStore {
    /// Open (or create) the store in `dir` and recover `fsm` — which must be
    /// freshly constructed — to the last committed state on disk.
    ///
    /// A torn final log record is dropped; a damaged record anywhere else,
    /// or a damaged vote/meta/snapshot file, is an error.
    pub fn open(dir: impl AsRef<Path>, fsm: Arc<Fsm>) -> io::Result<DiskStore> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let lock = crate::wal::lock_dir(&dir)?;
        let vote = read_json::<Vote<NodeId>>(&dir.join(VOTE))?;
        let meta = read_json::<LogMeta>(&dir.join(META))?.unwrap_or_default();
        let snapshot = read_atomic(&dir.join(SNAPSHOT))?
            .map(|b| StoredSnapshot::decode(&b))
            .transpose()?;
        let (wal, frames) = FramedLog::open(&dir.join(LOG))?;

        let mut log = BTreeMap::new();
        let mut offsets = BTreeMap::new();
        let purged = meta.last_purged.map(|l| l.index);
        for f in frames {
            let e: Entry<C> = serde_json::from_slice(&f.payload).map_err(data_err)?;
            let idx = e.log_id.index;
            if purged.is_some_and(|p| idx <= p) {
                continue; // purge recorded, compaction rewrite not yet done
            }
            log.insert(idx, e);
            offsets.insert(idx, f.offset);
        }

        let mut store = DiskStore {
            dir,
            wal,
            log: Arc::new(RwLock::new(log)),
            offsets,
            vote,
            meta,
            fsm,
            last_applied: None,
            membership: Membership::default(),
            snapshot: Arc::new(Mutex::new(snapshot)),
            _lock: lock,
        };
        store.recover_state_machine()?;
        Ok(store)
    }

    /// Snapshot image + committed entries after it -> the pre-crash state.
    fn recover_state_machine(&mut self) -> io::Result<()> {
        let snap = self.snapshot.lock().unwrap().clone();
        if let Some(s) = snap {
            let image: FsmImage = serde_json::from_slice(&s.data).map_err(data_err)?;
            self.fsm.restore(image.state, image.index);
            self.last_applied = s.meta.last_log_id;
            self.membership = s.meta.last_membership.clone();
        }
        let Some(committed) = self.meta.committed else {
            return Ok(());
        };
        let from = self.last_applied.map(|l| l.index + 1).unwrap_or(0);
        if from > committed.index {
            return Ok(());
        }
        let entries: Vec<Entry<C>> = {
            let log = self.log.read().unwrap();
            log.range(from..=committed.index)
                .map(|(_, e)| e.clone())
                .collect()
        };
        let contiguous = entries
            .iter()
            .enumerate()
            .all(|(i, e)| e.log_id.index == from + i as u64);
        if !contiguous || entries.len() as u64 != committed.index - from + 1 {
            return Err(data_err(format!(
                "log is missing committed entries {from}..={}",
                committed.index
            )));
        }
        apply_entries(
            &self.fsm,
            &mut self.last_applied,
            &mut self.membership,
            &entries,
        );
        Ok(())
    }

    fn save_meta(&self) -> io::Result<()> {
        write_json(&self.dir.join(META), &self.meta)
    }

    fn reader(&self) -> DiskLogReader {
        DiskLogReader {
            log: self.log.clone(),
        }
    }

    /// Drop the record at `index` and everything after it, durably.
    fn truncate_from(&mut self, index: u64) -> io::Result<()> {
        let Some((_, &offset)) = self.offsets.range(index..).next() else {
            return Ok(());
        };
        self.wal.truncate(offset)?;
        self.offsets.split_off(&index);
        self.log.write().unwrap().split_off(&index);
        Ok(())
    }
}

/// Read handle sharing the store's in-memory index of the on-disk log.
pub struct DiskLogReader {
    log: SharedLog,
}

fn read_range<RB: RangeBounds<u64>>(log: &SharedLog, range: RB) -> Vec<Entry<C>> {
    log.read()
        .unwrap()
        .range(range)
        .map(|(_, e)| e.clone())
        .collect()
}

impl RaftLogReader<C> for DiskLogReader {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<C>>, StorageError<NodeId>> {
        Ok(read_range(&self.log, range))
    }
}

impl RaftLogReader<C> for DiskStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<C>>, StorageError<NodeId>> {
        Ok(read_range(&self.log, range))
    }
}

/// Builds a snapshot from an image captured under the store's exclusive
/// borrow, persists it atomically, and makes it the current snapshot.
pub struct DiskSnapBuilder {
    dir: PathBuf,
    meta: Meta,
    data: Result<Vec<u8>, StorageError<NodeId>>,
    current: Arc<Mutex<Option<StoredSnapshot>>>,
}

impl RaftSnapshotBuilder<C> for DiskSnapBuilder {
    async fn build_snapshot(&mut self) -> Result<Snapshot<C>, StorageError<NodeId>> {
        let stored = StoredSnapshot {
            meta: self.meta.clone(),
            data: self.data.clone()?,
        };
        stored.save(&self.dir).map_err(io_err)?;
        let snap = stored.to_snapshot();
        *self.current.lock().unwrap() = Some(stored);
        Ok(snap)
    }
}

impl RaftStorage<C> for DiskStore {
    type LogReader = DiskLogReader;
    type SnapshotBuilder = DiskSnapBuilder;

    async fn save_vote(&mut self, vote: &Vote<NodeId>) -> Result<(), StorageError<NodeId>> {
        write_json(&self.dir.join(VOTE), vote).map_err(io_err)?;
        self.vote = Some(*vote);
        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<NodeId>>, StorageError<NodeId>> {
        Ok(self.vote)
    }

    async fn save_committed(
        &mut self,
        committed: Option<LogId<NodeId>>,
    ) -> Result<(), StorageError<NodeId>> {
        self.meta.committed = committed;
        self.save_meta().map_err(io_err)
    }

    async fn read_committed(&mut self) -> Result<Option<LogId<NodeId>>, StorageError<NodeId>> {
        Ok(self.meta.committed)
    }

    async fn get_log_state(&mut self) -> Result<LogState<C>, StorageError<NodeId>> {
        let last = self.log.read().unwrap().values().last().map(|e| e.log_id);
        Ok(LogState {
            last_purged_log_id: self.meta.last_purged,
            last_log_id: last.or(self.meta.last_purged),
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.reader()
    }

    async fn append_to_log<I>(&mut self, entries: I) -> Result<(), StorageError<NodeId>>
    where
        I: IntoIterator<Item = Entry<C>> + OptionalSend,
    {
        let entries: Vec<Entry<C>> = entries.into_iter().collect();
        let Some(first) = entries.first() else {
            return Ok(());
        };
        // Overwriting an index means everything from it on is stale.
        self.truncate_from(first.log_id.index).map_err(io_err)?;
        let payloads = entries
            .iter()
            .map(serde_json::to_vec)
            .collect::<Result<Vec<_>, _>>()
            .map_err(io_err)?;
        let offsets = self
            .wal
            .append(payloads.iter().map(Vec::as_slice))
            .map_err(io_err)?;
        let mut log = self.log.write().unwrap();
        for (e, off) in entries.into_iter().zip(offsets) {
            self.offsets.insert(e.log_id.index, off);
            log.insert(e.log_id.index, e);
        }
        Ok(())
    }

    async fn delete_conflict_logs_since(
        &mut self,
        log_id: LogId<NodeId>,
    ) -> Result<(), StorageError<NodeId>> {
        self.truncate_from(log_id.index).map_err(io_err)
    }

    async fn purge_logs_upto(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        self.meta.last_purged = Some(log_id);
        self.save_meta().map_err(io_err)?;
        let kept = {
            let mut log = self.log.write().unwrap();
            *log = log.split_off(&(log_id.index + 1));
            log.values()
                .map(|e| serde_json::to_vec(e).map(|p| (e.log_id.index, p)))
                .collect::<Result<Vec<_>, _>>()
                .map_err(io_err)?
        };
        let offsets = self
            .wal
            .rewrite(kept.iter().map(|(_, p)| p.as_slice()))
            .map_err(io_err)?;
        self.offsets = kept.iter().map(|(i, _)| *i).zip(offsets).collect();
        Ok(())
    }

    async fn last_applied_state(
        &mut self,
    ) -> Result<(Option<LogId<NodeId>>, Membership), StorageError<NodeId>> {
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
        DiskSnapBuilder {
            dir: self.dir.clone(),
            meta: snapshot_meta(self.last_applied, &self.membership),
            data: encode_image(&self.fsm),
            current: self.snapshot.clone(),
        }
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<<C as RaftTypeConfig>::SnapshotData>, StorageError<NodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &Meta,
        snapshot: Box<<C as RaftTypeConfig>::SnapshotData>,
    ) -> Result<(), StorageError<NodeId>> {
        let stored = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };
        stored.save(&self.dir).map_err(io_err)?;
        crate::store::restore_image(&self.fsm, &stored.data)?;
        self.last_applied = meta.last_log_id;
        self.membership = meta.last_membership.clone();
        *self.snapshot.lock().unwrap() = Some(stored);
        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<C>>, StorageError<NodeId>> {
        Ok(self
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .map(StoredSnapshot::to_snapshot))
    }
}

/// Open a [`DiskStore`] in `dir` (recovering `fsm`) and wrap it into the
/// (log_store, state_machine) pair [`openraft::Raft::new`] expects.
pub fn storage_pair_on_disk(
    dir: impl AsRef<Path>,
    fsm: Arc<Fsm>,
) -> io::Result<(Adaptor<C, DiskStore>, Adaptor<C, DiskStore>)> {
    Ok(Adaptor::new(DiskStore::open(dir, fsm)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Request;
    use crate::wal::testdir::TestDir;
    use gnomon_core::{Event, Job, JobId, ManualClock, Resources, Walltime};
    use openraft::testing::log_id;
    use openraft::EntryPayload;
    use std::time::Duration;

    fn fingerprint(fsm: &Fsm) -> String {
        fsm.read(|st| format!("{st:?}"))
    }

    fn fresh_fsm() -> Arc<Fsm> {
        Arc::new(Fsm::with_clock(Arc::new(ManualClock::new(0))))
    }

    fn cores(n: u32) -> Resources {
        Resources {
            cores: n,
            mem_mb: 0,
            gpus: 0,
        }
    }

    /// Entry `i` of a deterministic workload: register a node, then
    /// alternately submit and start jobs, stamped at 1000 + 10 i.
    fn event(i: u64) -> Event {
        match i {
            0 => Event::NodeRegistered {
                name: "n0".into(),
                capacity: cores(64),
            },
            i if i % 2 == 1 => Event::JobSubmitted(Job::new(
                JobId(i),
                "j",
                "u",
                cores(1),
                Walltime(Duration::from_secs(500)),
            )),
            i => Event::JobStarted {
                job: JobId(i - 1),
                vertices: vec![1],
                start: 1000 + 10 * i,
                dur: None,
            },
        }
    }

    fn entry(term: u64, i: u64) -> Entry<C> {
        Entry {
            log_id: log_id(term, 1, i),
            payload: EntryPayload::Normal(Request::new(event(i), 1000 + 10 * i)),
        }
    }

    fn entries(term: u64, r: std::ops::Range<u64>) -> Vec<Entry<C>> {
        r.map(|i| entry(term, i)).collect()
    }

    async fn indexes(s: &mut DiskStore) -> Vec<u64> {
        s.try_get_log_entries(..)
            .await
            .unwrap()
            .iter()
            .map(|e| e.log_id.index)
            .collect()
    }

    #[tokio::test]
    async fn append_truncate_purge_round_trip_across_reopen() {
        let d = TestDir::new("disk-log");
        let mut s = DiskStore::open(&d.0, fresh_fsm()).unwrap();
        s.append_to_log(entries(1, 0..10)).await.unwrap();
        s.delete_conflict_logs_since(log_id(1, 1, 7)).await.unwrap();
        drop(s);

        let mut s = DiskStore::open(&d.0, fresh_fsm()).unwrap();
        assert_eq!(indexes(&mut s).await, (0..7).collect::<Vec<_>>());
        s.purge_logs_upto(log_id(1, 1, 3)).await.unwrap();
        // A new leader's entries replace the truncated suffix.
        s.append_to_log(entries(2, 7..9)).await.unwrap();
        drop(s);

        let mut s = DiskStore::open(&d.0, fresh_fsm()).unwrap();
        assert_eq!(indexes(&mut s).await, vec![4, 5, 6, 7, 8]);
        let st = s.get_log_state().await.unwrap();
        assert_eq!(st.last_purged_log_id, Some(log_id(1, 1, 3)));
        assert_eq!(st.last_log_id, Some(log_id(2, 1, 8)));
        // Overwriting an index without an explicit truncate also sticks.
        s.append_to_log(entries(3, 6..7)).await.unwrap();
        drop(s);
        let mut s = DiskStore::open(&d.0, fresh_fsm()).unwrap();
        assert_eq!(indexes(&mut s).await, vec![4, 5, 6]);
        let mut r = s.get_log_reader().await;
        let tail = r.try_get_log_entries(6..).await.unwrap();
        assert_eq!(tail[0].log_id, log_id(3, 1, 6));
    }

    #[tokio::test]
    async fn purge_recorded_but_not_compacted_is_honoured() {
        let d = TestDir::new("disk-purge-crash");
        let mut s = DiskStore::open(&d.0, fresh_fsm()).unwrap();
        s.append_to_log(entries(1, 0..5)).await.unwrap();
        // Crash after `meta` was written but before the log was rewritten.
        s.meta.last_purged = Some(log_id(1, 1, 2));
        s.save_meta().unwrap();
        drop(s);
        let mut s = DiskStore::open(&d.0, fresh_fsm()).unwrap();
        assert_eq!(indexes(&mut s).await, vec![3, 4]);
    }

    #[tokio::test]
    async fn torn_final_log_record_is_dropped() {
        let d = TestDir::new("disk-torn");
        let mut s = DiskStore::open(&d.0, fresh_fsm()).unwrap();
        s.append_to_log(entries(1, 0..4)).await.unwrap();
        drop(s);
        let p = d.0.join(LOG);
        let len = std::fs::metadata(&p).unwrap().len();
        let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.set_len(len - 5).unwrap();
        drop(f);
        let mut s = DiskStore::open(&d.0, fresh_fsm()).unwrap();
        assert_eq!(indexes(&mut s).await, vec![0, 1, 2]);
        s.append_to_log(entries(1, 3..5)).await.unwrap();
        drop(s);
        let mut s = DiskStore::open(&d.0, fresh_fsm()).unwrap();
        assert_eq!(indexes(&mut s).await, vec![0, 1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn corrupted_middle_log_record_is_an_error() {
        let d = TestDir::new("disk-corrupt");
        let mut s = DiskStore::open(&d.0, fresh_fsm()).unwrap();
        s.append_to_log(entries(1, 0..4)).await.unwrap();
        drop(s);
        let p = d.0.join(LOG);
        let mut buf = std::fs::read(&p).unwrap();
        buf[12] ^= 0x55; // inside the first record's payload
        std::fs::write(&p, buf).unwrap();
        assert!(DiskStore::open(&d.0, fresh_fsm()).is_err());
    }

    #[test]
    fn a_store_directory_can_be_opened_by_one_store_at_a_time() {
        let d = TestDir::new("disk-lock");
        let first = DiskStore::open(&d.0, fresh_fsm()).unwrap();
        let err = DiskStore::open(&d.0, fresh_fsm())
            .err()
            .expect("locked by the first store");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        drop(first);
        assert!(DiskStore::open(&d.0, fresh_fsm()).is_ok());
    }

    #[tokio::test]
    async fn vote_snapshot_and_committed_state_survive_reopen() {
        let d = TestDir::new("disk-recover");
        let fsm = fresh_fsm();
        let mut s = DiskStore::open(&d.0, fsm.clone()).unwrap();
        let vote = Vote::new_committed(3, 1);
        s.save_vote(&vote).await.unwrap();
        let all = entries(1, 0..9);
        s.append_to_log(all.clone()).await.unwrap();
        s.save_committed(Some(log_id(1, 1, 6))).await.unwrap();
        s.apply_to_state_machine(&all[..4]).await.unwrap();
        let snap = s
            .get_snapshot_builder()
            .await
            .build_snapshot()
            .await
            .unwrap();
        assert_eq!(snap.meta.last_log_id, Some(log_id(1, 1, 3)));
        s.purge_logs_upto(log_id(1, 1, 3)).await.unwrap();
        s.apply_to_state_machine(&all[4..7]).await.unwrap();
        let (fp, idx) = (fingerprint(&fsm), fsm.index());
        drop(s);

        let fsm2 = fresh_fsm();
        let mut s = DiskStore::open(&d.0, fsm2.clone()).unwrap();
        assert_eq!(s.read_vote().await.unwrap(), Some(vote));
        assert_eq!(s.read_committed().await.unwrap(), Some(log_id(1, 1, 6)));
        let cur = s.get_current_snapshot().await.unwrap().expect("snapshot");
        assert_eq!(cur.meta, snap.meta);
        // Snapshot (0..=3) + committed entries 4..=6; 7 and 8 are uncommitted.
        assert_eq!(fingerprint(&fsm2), fp);
        assert_eq!(fsm2.index(), idx);
        let (applied, _) = s.last_applied_state().await.unwrap();
        assert_eq!(applied, Some(log_id(1, 1, 6)));
    }

    #[tokio::test]
    async fn installed_snapshot_is_persisted() {
        let d = TestDir::new("disk-install");
        let src = fresh_fsm();
        let mut a = DiskStore::open(d.0.join("a"), src.clone()).unwrap();
        let all = entries(1, 0..5);
        a.append_to_log(all.clone()).await.unwrap();
        a.apply_to_state_machine(&all).await.unwrap();
        let snap = a
            .get_snapshot_builder()
            .await
            .build_snapshot()
            .await
            .unwrap();

        let mut b = DiskStore::open(d.0.join("b"), fresh_fsm()).unwrap();
        b.install_snapshot(&snap.meta, snap.snapshot).await.unwrap();
        drop(b);
        let fsm = fresh_fsm();
        let mut b = DiskStore::open(d.0.join("b"), fsm.clone()).unwrap();
        assert_eq!(fingerprint(&fsm), fingerprint(&src));
        let (applied, _) = b.last_applied_state().await.unwrap();
        assert_eq!(applied, Some(log_id(1, 1, 4)));
    }

    mod single_node {
        use super::*;
        use crate::types::TypeConfig;
        use crate::{propose_at, SchedulerRaft};
        use openraft::error::{InstallSnapshotError, RPCError, RaftError, Unreachable};
        use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
        use openraft::raft::{
            AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest,
            InstallSnapshotResponse, VoteRequest, VoteResponse,
        };
        use openraft::{Config, Raft};

        type RpcResult<T, E = RaftError<NodeId>> = Result<T, RPCError<NodeId, BasicNode, E>>;

        /// A one-member cluster never sends RPCs; every call is unreachable.
        struct NoNetwork;

        fn unreachable<E: std::error::Error>() -> RPCError<NodeId, BasicNode, E> {
            RPCError::Unreachable(Unreachable::new(&io::Error::other("single node")))
        }

        impl RaftNetwork<TypeConfig> for NoNetwork {
            async fn append_entries(
                &mut self,
                _: AppendEntriesRequest<TypeConfig>,
                _: RPCOption,
            ) -> RpcResult<AppendEntriesResponse<NodeId>> {
                Err(unreachable())
            }
            async fn install_snapshot(
                &mut self,
                _: InstallSnapshotRequest<TypeConfig>,
                _: RPCOption,
            ) -> RpcResult<InstallSnapshotResponse<NodeId>, RaftError<NodeId, InstallSnapshotError>>
            {
                Err(unreachable())
            }
            async fn vote(
                &mut self,
                _: VoteRequest<NodeId>,
                _: RPCOption,
            ) -> RpcResult<VoteResponse<NodeId>> {
                Err(unreachable())
            }
        }

        impl RaftNetworkFactory<TypeConfig> for NoNetwork {
            type Network = NoNetwork;
            async fn new_client(&mut self, _: NodeId, _: &BasicNode) -> NoNetwork {
                NoNetwork
            }
        }

        async fn start(dir: &Path, fsm: Arc<Fsm>) -> SchedulerRaft {
            let cfg = Arc::new(
                Config {
                    heartbeat_interval: 50,
                    election_timeout_min: 100,
                    election_timeout_max: 200,
                    ..Default::default()
                }
                .validate()
                .unwrap(),
            );
            let (log, sm) = storage_pair_on_disk(dir, fsm).unwrap();
            Raft::new(1, cfg, NoNetwork, log, sm).await.unwrap()
        }

        async fn await_leader(raft: &SchedulerRaft) {
            raft.wait(Some(Duration::from_secs(30)))
                .current_leader(1, "single node elects itself")
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn committed_events_survive_restart() {
            let d = TestDir::new("disk-raft");
            let fsm = fresh_fsm();
            let raft = start(&d.0, fsm.clone()).await;
            let mut members = BTreeMap::new();
            members.insert(1, BasicNode::default());
            raft.initialize(members).await.unwrap();
            await_leader(&raft).await;
            for i in 0..6 {
                propose_at(&raft, event(i), 1000 + 10 * i).await.unwrap();
            }
            raft.trigger().snapshot().await.unwrap();
            raft.wait(Some(Duration::from_secs(30)))
                .metrics(|m| m.snapshot.is_some(), "snapshot built")
                .await
                .unwrap();
            for i in 6..9 {
                propose_at(&raft, event(i), 1000 + 10 * i).await.unwrap();
            }
            let (fp, idx) = (fingerprint(&fsm), fsm.index());
            assert_eq!(idx, 9);
            raft.shutdown().await.unwrap();
            drop(raft);
            assert!(d.0.join(SNAPSHOT).exists());

            // Reopen on a fresh FSM: recovery alone restores the state ...
            let fsm2 = fresh_fsm();
            let raft = start(&d.0, fsm2.clone()).await;
            assert_eq!(fingerprint(&fsm2), fp);
            assert_eq!(fsm2.index(), idx);
            // ... and the restarted node keeps committing on top of it.
            await_leader(&raft).await;
            propose_at(&raft, event(9), 1090).await.unwrap();
            assert_eq!(fsm2.index(), idx + 1);
            raft.shutdown().await.unwrap();
        }
    }
}
