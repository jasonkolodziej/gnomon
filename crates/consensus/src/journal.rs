//! Durable event journal for the single-controller deployment.
//!
//! The controller's API handlers, broker workers and backfill pass all call
//! [`Fsm::apply`] directly; routing each of those through `Raft::client_write`
//! is the multi-node step. Until then the controller persists the same thing
//! a Raft log would — every applied event with its clock reading — through
//! the FSM's write-ahead [`EventSink`] hook, using the framed, fsync'd log of
//! [`crate::wal`].
//!
//! Directory layout: `journal` (one framed JSON [`Record`] per event) and
//! `state` (an atomically replaced [`FsmImage`]). On open the image is
//! restored, newer records are replayed at their recorded timestamps
//! ([`Fsm::apply_at`]), and the result is compacted into a fresh image with
//! an empty journal — so startup cost is bounded by one run's worth of events.

use crate::store::{image_bytes, FsmImage};
use crate::wal::{atomic_write, data_err, read_atomic, FramedLog};
use gnomon_core::{Clock, Event, StateIndex};
use gnomon_fsm::{EventSink, Fsm, JournalEntry};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const JOURNAL: &str = "journal";
const STATE: &str = "state";

/// One journaled event: the state index it produced and when it applied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub index: StateIndex,
    pub at: u64,
    pub event: Event,
}

/// Append-only, fsync'd event journal; an [`EventSink`] for the FSM.
pub struct EventJournal {
    dir: PathBuf,
    log: Mutex<FramedLog>,
}

impl EventJournal {
    /// Recover an FSM from `dir` (creating it if absent) and attach a journal
    /// so every later event is persisted before it is applied.
    pub fn open_fsm(dir: impl AsRef<Path>, clock: Arc<dyn Clock>) -> io::Result<Arc<Fsm>> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let fsm = Arc::new(Fsm::with_clock(clock));
        if let Some(bytes) = read_atomic(&dir.join(STATE))? {
            let image: FsmImage = serde_json::from_slice(&bytes).map_err(data_err)?;
            fsm.restore(image.state, image.index);
        }
        let (mut log, frames) = FramedLog::open(&dir.join(JOURNAL))?;
        for f in &frames {
            let r: Record = serde_json::from_slice(&f.payload).map_err(data_err)?;
            if r.index <= fsm.index() {
                continue; // already folded into the state image
            }
            if r.index != fsm.index() + 1 {
                return Err(data_err(format!(
                    "journal gap: expected index {}, found {}",
                    fsm.index() + 1,
                    r.index
                )));
            }
            fsm.apply_at(r.event, r.at);
        }
        if !frames.is_empty() {
            // Image first, then empty the journal: a crash in between leaves
            // records the image already covers, which the index check skips.
            atomic_write(&dir.join(STATE), &image_bytes(&fsm).map_err(data_err)?)?;
            log.rewrite(std::iter::empty())?;
        }
        let journal = Arc::new(EventJournal {
            dir,
            log: Mutex::new(log),
        });
        if fsm.set_sink(journal).is_err() {
            unreachable!("fresh FSM has no sink");
        }
        Ok(fsm)
    }

    /// Persist one record (write + `sync_data`).
    pub fn append(&self, record: &Record) -> io::Result<()> {
        let payload = serde_json::to_vec(record).map_err(data_err)?;
        self.log.lock().unwrap().append([payload.as_slice()])?;
        Ok(())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl EventSink for EventJournal {
    /// Fail-stop: if the event cannot be made durable the controller must not
    /// apply it (state would run ahead of the log), so this panics. The panic
    /// happens under the FSM write lock, poisoning it and halting all further
    /// mutation; a supervisor restart recovers from the journal.
    fn record(&self, index: StateIndex, entry: &JournalEntry) {
        let record = Record {
            index,
            at: entry.at,
            event: entry.event.clone(),
        };
        if let Err(e) = self.append(&record) {
            panic!(
                "gnomon: cannot persist event {index} to {}: {e}",
                self.dir.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wal::testdir::TestDir;
    use gnomon_core::{Job, JobId, ManualClock, Resources, Walltime};
    use std::time::Duration;

    fn fingerprint(fsm: &Fsm) -> String {
        fsm.read(|st| format!("{st:?}"))
    }

    /// Register a node if there is none, then submit and start `jobs`.
    fn drive(fsm: &Fsm, clock: &ManualClock, jobs: std::ops::Range<u64>) {
        if fsm.read(|st| st.nodes.is_empty()) {
            fsm.apply(Event::NodeRegistered {
                name: "n0".into(),
                capacity: Resources {
                    cores: 8,
                    mem_mb: 0,
                    gpus: 0,
                },
            });
        }
        for i in jobs {
            clock.set(1000 + i * 10);
            fsm.apply(Event::JobSubmitted(Job::new(
                JobId(i),
                "j",
                "u",
                Resources {
                    cores: 1,
                    mem_mb: 0,
                    gpus: 0,
                },
                Walltime(Duration::from_secs(100)),
            )));
            fsm.apply(Event::JobStarted {
                job: JobId(i),
                vertices: vec![1],
                start: 1000 + i * 10,
                dur: None,
            });
        }
    }

    #[test]
    fn journal_recovers_identical_state_across_restarts() {
        let d = TestDir::new("journal-restart");
        let clock = Arc::new(ManualClock::new(1000));
        let fsm = EventJournal::open_fsm(&d.0, clock.clone()).unwrap();
        drive(&fsm, &clock, 1..3);
        let (fp, idx) = (fingerprint(&fsm), fsm.index());
        drop(fsm);

        // Recovery replays at recorded times, whatever the clock says now.
        let later = Arc::new(ManualClock::new(9_999));
        let fsm = EventJournal::open_fsm(&d.0, later.clone()).unwrap();
        assert_eq!(fingerprint(&fsm), fp);
        assert_eq!(fsm.index(), idx);

        // Second generation: image + new journal records.
        drive(&fsm, &later, 3..5);
        let (fp2, idx2) = (fingerprint(&fsm), fsm.index());
        drop(fsm);
        let fsm = EventJournal::open_fsm(&d.0, later).unwrap();
        assert_eq!(fingerprint(&fsm), fp2);
        assert_eq!(fsm.index(), idx2);
    }

    #[test]
    fn records_already_in_the_image_are_skipped() {
        let d = TestDir::new("journal-compact-crash");
        let clock = Arc::new(ManualClock::new(1000));
        let fsm = EventJournal::open_fsm(&d.0, clock.clone()).unwrap();
        drive(&fsm, &clock, 1..3);
        let fp = fingerprint(&fsm);
        // Simulate a crash after the image was written but before the
        // journal was emptied.
        atomic_write(&d.0.join(STATE), &image_bytes(&fsm).unwrap()).unwrap();
        drop(fsm);
        let fsm = EventJournal::open_fsm(&d.0, clock).unwrap();
        assert_eq!(fingerprint(&fsm), fp);
    }

    #[test]
    fn torn_last_journal_record_loses_only_that_event() {
        let d = TestDir::new("journal-torn");
        let clock = Arc::new(ManualClock::new(1000));
        let fsm = EventJournal::open_fsm(&d.0, clock.clone()).unwrap();
        drive(&fsm, &clock, 1..2);
        let fp = fingerprint(&fsm);
        drop(fsm);
        let path = d.0.join(JOURNAL);
        let mut buf = std::fs::read(&path).unwrap();
        let rec = Record {
            index: 3,
            at: 1,
            event: Event::JobCancelled(JobId(1)),
        };
        let frame = crate::wal::encode_frame(&serde_json::to_vec(&rec).unwrap()).unwrap();
        buf.extend_from_slice(&frame[..frame.len() - 3]);
        std::fs::write(&path, buf).unwrap();
        let fsm = EventJournal::open_fsm(&d.0, clock).unwrap();
        assert_eq!(fingerprint(&fsm), fp);
    }
}
