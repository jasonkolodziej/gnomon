//! Exact replay and counterfactual extraction from an FSM journal.

use crate::SimJob;
#[allow(unused_imports)]
use gnomon_core::Clock;
use gnomon_core::{Event, JobId, JobState, ManualClock};
use gnomon_fsm::{Fsm, JournalEntry};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Re-apply `journal` to a fresh state machine (clock driven by the recorded
/// timestamps) and return a canonical rendering of the resulting state.
pub fn rebuild(journal: &[JournalEntry]) -> Fsm {
    let clock = Arc::new(ManualClock::new(journal.first().map(|e| e.at).unwrap_or(0)));
    let fsm = Fsm::with_clock(clock.clone());
    for e in journal {
        clock.set(e.at);
        fsm.apply(e.event.clone());
    }
    fsm
}

pub fn fingerprint(fsm: &Fsm) -> String {
    fsm.read(|st| format!("{st:?}"))
}

/// True if replaying the journal reproduces exactly the original state.
pub fn verify_exact(original: &Fsm) -> bool {
    fingerprint(&rebuild(&original.journal())) == fingerprint(original)
}

/// Recover the workload a journal describes: every submitted job, with its
/// true runtime taken from when it actually ran (accepted start) to its
/// completion. Jobs that never finished are dropped.
///
/// A job's end is its `JobFinished` time or, for a gang that completed
/// through per-rank reports, the latest *accepted* `JobRankFinished` (the
/// time the FSM releases it at); duplicate or foreign rank reports, which
/// the FSM rejects, are ignored the same way.
pub fn workload_from_journal(journal: &[JournalEntry]) -> Vec<SimJob> {
    let clock = Arc::new(ManualClock::new(journal.first().map(|e| e.at).unwrap_or(0)));
    let fsm = Fsm::with_clock(clock.clone());
    let mut ends: BTreeMap<JobId, u64> = BTreeMap::new();
    for e in journal {
        clock.set(e.at);
        let accepted = fsm.apply_checked(e.event.clone()).accepted;
        match &e.event {
            Event::JobFinished { job, end, .. } => {
                ends.insert(*job, *end);
            }
            Event::JobRankFinished { job, end, .. } if accepted => {
                let last = ends.entry(*job).or_insert(*end);
                *last = (*last).max(*end);
            }
            _ => {}
        }
    }
    // The accepted start is the job's planned_start in the rebuilt state;
    // only jobs that reached a terminal state count as finished.
    fsm.read(|st| {
        st.jobs
            .values()
            .filter(|j| matches!(j.state, JobState::Completed | JobState::Failed))
            .filter_map(|j| {
                let end = *ends.get(&j.id)?;
                let start = j.planned_start?;
                let mut job = j.clone();
                job.state = JobState::Pending;
                job.planned_start = None;
                job.planned_dur = None;
                job.placed_on = None;
                job.allocation = None;
                Some(SimJob {
                    job,
                    actual: end - start,
                })
            })
            .collect()
    })
}

/// Serialize a journal as JSON lines (the on-disk replay format).
pub fn to_jsonl(journal: &[JournalEntry]) -> String {
    journal
        .iter()
        .map(|e| serde_json::to_string(e).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug)]
pub struct JsonlParseError {
    pub line: usize,
    pub source: serde_json::Error,
}

impl std::fmt::Display for JsonlParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid JSONL record on line {}: {}",
            self.line, self.source
        )
    }
}

impl std::error::Error for JsonlParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

pub fn from_jsonl(s: &str) -> Result<Vec<JournalEntry>, JsonlParseError> {
    s.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(line, l)| {
            serde_json::from_str(l).map_err(|source| JsonlParseError {
                line: line + 1,
                source,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnomon_core::{Resources, Walltime};
    use std::error::Error;
    use std::time::Duration;

    fn node_fsm() -> Fsm {
        let fsm = Fsm::recording(Arc::new(ManualClock::new(100)));
        fsm.apply(Event::NodeRegistered {
            name: "node-0".into(),
            capacity: Resources {
                cores: 4,
                mem_mb: 1024,
                gpus: 1,
            },
        });
        fsm
    }

    #[test]
    fn fingerprint_includes_node_and_assignment_state() {
        let idle = node_fsm();
        let busy = node_fsm();
        busy.apply(Event::Heartbeat {
            name: "node-0".into(),
            allocated: Resources {
                cores: 1,
                mem_mb: 0,
                gpus: 0,
            },
        });
        assert_ne!(fingerprint(&idle), fingerprint(&busy));

        let make_assigned_fsm = || {
            let fsm = node_fsm();
            let job = gnomon_core::Job::new(
                JobId(1),
                "job",
                "user",
                Resources {
                    cores: 1,
                    mem_mb: 0,
                    gpus: 0,
                },
                Walltime(Duration::from_secs(10)),
            );
            fsm.apply(Event::JobSubmitted(job));
            fsm.apply(Event::JobStarted {
                job: JobId(1),
                vertices: vec![1],
                start: 100,
                dur: None,
            });
            fsm
        };
        let queued = make_assigned_fsm();
        let fetched = make_assigned_fsm();
        fetched.apply(Event::AssignmentsFetched {
            node: "node-0".into(),
            count: 1,
        });
        assert_ne!(fingerprint(&queued), fingerprint(&fetched));
    }

    #[test]
    fn jsonl_parse_errors_report_original_line_number() {
        let entry = JournalEntry {
            at: 100,
            event: Event::NodeRegistered {
                name: "node-0".into(),
                capacity: Resources::default(),
            },
        };
        let jsonl = format!("{}\n \n{{", serde_json::to_string(&entry).unwrap());

        let error = from_jsonl(&jsonl).unwrap_err();
        assert_eq!(error.line, 3);
        assert!(error.to_string().contains("line 3"));
        assert!(error.source().is_some());
    }

    #[test]
    fn live_gang_finished_by_rank_reports_is_recovered() {
        let clock = Arc::new(ManualClock::new(1000));
        let fsm = Fsm::recording(clock.clone());
        for n in ["n0", "n1"] {
            fsm.apply(Event::NodeRegistered {
                name: n.into(),
                capacity: Resources {
                    cores: 4,
                    mem_mb: 0,
                    gpus: 0,
                },
            });
        }
        let mut gang = gnomon_core::Job::new(
            JobId(1),
            "mpi",
            "u",
            Resources {
                cores: 4,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(std::time::Duration::from_secs(600)),
        );
        gang.nodes = 2;
        gang.submit_time = 1000;
        fsm.apply(Event::JobSubmitted(gang));
        let vs: Vec<u32> =
            fsm.read(|st| ["n0", "n1"].map(|n| st.graph.by_name(n).unwrap()).to_vec());
        fsm.apply(Event::JobStarted {
            job: JobId(1),
            vertices: vs,
            start: 1000,
            dur: None,
        });
        let rank = |node: &str, end: u64| Event::JobRankFinished {
            job: JobId(1),
            node: node.into(),
            end,
            code: 0,
        };
        clock.set(1100);
        fsm.apply(rank("n0", 1100));
        clock.set(1500);
        fsm.apply(rank("n0", 1500)); // duplicate: rejected, must not count
        clock.set(1200);
        fsm.apply(rank("n1", 1200));

        let w = workload_from_journal(&fsm.journal());
        assert_eq!(w.len(), 1, "the gang finished through rank reports");
        assert_eq!(w[0].actual, 200, "latest accepted rank end - start");
        assert_eq!(w[0].job.nodes, 2);
        assert!(w[0].job.allocation.is_none() && w[0].job.placed_on.is_none());
    }
}
