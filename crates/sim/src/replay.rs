//! Exact replay and counterfactual extraction from an FSM journal.

use crate::SimJob;
#[allow(unused_imports)]
use gnomon_core::Clock;
use gnomon_core::{Event, JobId, ManualClock};
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
pub fn workload_from_journal(journal: &[JournalEntry]) -> Vec<SimJob> {
    let fsm = rebuild(journal);
    let ends: BTreeMap<JobId, u64> = journal
        .iter()
        .filter_map(|e| match &e.event {
            Event::JobFinished { job, end, .. } => Some((*job, *end)),
            _ => None,
        })
        .collect();
    // The accepted start is the job's planned_start in the rebuilt state.
    fsm.read(|st| {
        st.jobs
            .values()
            .filter_map(|j| {
                let end = *ends.get(&j.id)?;
                let start = j.planned_start?;
                let mut job = j.clone();
                job.state = gnomon_core::JobState::Pending;
                job.planned_start = None;
                job.planned_dur = None;
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
}
