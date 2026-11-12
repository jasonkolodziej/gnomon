//! Exact replay and counterfactual extraction from an FSM journal.

use crate::SimJob;
use gnomon_core::{Event, JobId, ManualClock};
#[allow(unused_imports)]
use gnomon_core::Clock;
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
    fsm.read(|st| format!("{:?}|{:?}", st.jobs, st.graph))
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
    let ends: BTreeMap<JobId, u64> = journal.iter().filter_map(|e| match &e.event {
        Event::JobFinished { job, end, .. } => Some((*job, *end)), _ => None }).collect();
    // The accepted start is the job's planned_start in the rebuilt state.
    fsm.read(|st| st.jobs.values().filter_map(|j| {
        let end = *ends.get(&j.id)?;
        let start = j.planned_start?;
        let mut job = j.clone();
        job.state = gnomon_core::JobState::Pending;
        job.planned_start = None;
        job.planned_dur = None;
        Some(SimJob { job, actual: end - start })
    }).collect())
}

/// Serialize a journal as JSON lines (the on-disk replay format).
pub fn to_jsonl(journal: &[JournalEntry]) -> String {
    journal.iter().map(|e| serde_json::to_string(e).unwrap()).collect::<Vec<_>>().join("\n")
}

pub fn from_jsonl(s: &str) -> Vec<JournalEntry> {
    s.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
}
