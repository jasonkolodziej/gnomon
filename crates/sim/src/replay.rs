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
/// the FSM rejects, are ignored the same way. Jobs a lost node ended
/// ([`Event::NodeLost`]) end at the loss; a late report for them is ignored.
pub fn workload_from_journal(journal: &[JournalEntry]) -> Vec<SimJob> {
    let clock = Arc::new(ManualClock::new(journal.first().map(|e| e.at).unwrap_or(0)));
    let fsm = Fsm::with_clock(clock.clone());
    let mut ends: BTreeMap<JobId, u64> = BTreeMap::new();
    let running = |fsm: &Fsm| -> Vec<JobId> {
        fsm.read(|st| {
            st.jobs
                .values()
                .filter(|j| j.state == JobState::Running)
                .map(|j| j.id)
                .collect()
        })
    };
    let state = |fsm: &Fsm, job: &JobId| fsm.read(|st| st.jobs.get(job).map(|j| j.state));
    for e in journal {
        clock.set(e.at);
        // Completion reports count only when they end (part of) a running
        // job: a late report for a job a lost node already ended is accepted
        // only to clear bookkeeping and must not move its end.
        let ran = match &e.event {
            Event::JobFinished { job, .. } | Event::JobRankFinished { job, .. } => {
                state(&fsm, job) == Some(JobState::Running)
            }
            _ => false,
        };
        let before_loss = matches!(e.event, Event::NodeLost { .. }).then(|| running(&fsm));
        let accepted = fsm.apply_checked(e.event.clone()).accepted;
        if let Some(before) = before_loss {
            // Jobs a lost node ended run until the loss (a gang: until its
            // latest rank, the loss included). Requeued jobs are not ended.
            for job in before {
                if matches!(
                    state(&fsm, &job),
                    Some(JobState::Completed | JobState::Failed)
                ) {
                    let last = ends.entry(job).or_insert(e.at);
                    *last = (*last).max(e.at);
                }
            }
        }
        match &e.event {
            Event::JobFinished { job, end, .. } if ran => {
                ends.insert(*job, *end);
            }
            Event::JobRankFinished { job, end, .. } if accepted && ran => {
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
                // An end before the accepted start belongs to an earlier
                // attempt (never expected; dropped rather than underflowing).
                let actual = end.checked_sub(start)?;
                let mut job = j.clone();
                job.state = JobState::Pending;
                job.planned_start = None;
                job.planned_dur = None;
                job.placed_on = None;
                job.allocation = None;
                Some(SimJob { job, actual })
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
            items: vec![],
        });
        assert_ne!(fingerprint(&queued), fingerprint(&fetched));
    }

    #[test]
    fn a_lost_job_ends_at_the_loss_and_its_late_report_is_ignored() {
        let clock = Arc::new(ManualClock::new(100));
        let fsm = Fsm::recording(clock.clone());
        fsm.apply(Event::NodeRegistered {
            name: "node-0".into(),
            capacity: Resources {
                cores: 4,
                mem_mb: 0,
                gpus: 0,
            },
        });
        let job = gnomon_core::Job::new(
            JobId(1),
            "job",
            "user",
            Resources {
                cores: 1,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(Duration::from_secs(1000)),
        );
        fsm.apply(Event::JobSubmitted(job));
        fsm.apply(Event::JobStarted {
            job: JobId(1),
            vertices: vec![1],
            start: 100,
            dur: None,
        });
        fsm.apply(Event::AssignmentsFetched {
            node: "node-0".into(),
            count: 1,
            items: vec![(JobId(1), 0)],
        });
        clock.set(160);
        fsm.apply(Event::NodeLost {
            name: "node-0".into(),
            last_heartbeat: 100,
        });
        clock.set(400);
        fsm.apply(Event::JobFinished {
            job: JobId(1),
            end: 400,
            code: 0,
        });
        let journal = fsm.journal();
        let work = workload_from_journal(&journal);
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].actual, 60, "ran from 100 until the loss at 160");
        assert!(verify_exact(&fsm), "node loss replays exactly");
    }

    fn one_core(id: u64) -> gnomon_core::Job {
        gnomon_core::Job::new(
            JobId(id),
            "job",
            "user",
            Resources {
                cores: 1,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(Duration::from_secs(1000)),
        )
    }

    fn two_nodes(clock: &Arc<ManualClock>) -> Fsm {
        let fsm = Fsm::recording(clock.clone());
        for n in ["node-0", "node-1"] {
            fsm.apply(Event::NodeRegistered {
                name: n.into(),
                capacity: Resources {
                    cores: 4,
                    mem_mb: 0,
                    gpus: 0,
                },
            });
        }
        fsm
    }

    #[test]
    fn a_requeued_job_is_measured_from_its_rerun() {
        let clock = Arc::new(ManualClock::new(100));
        let fsm = two_nodes(&clock);
        fsm.apply(Event::JobSubmitted(one_core(1)));
        fsm.apply(Event::JobStarted {
            job: JobId(1),
            vertices: vec![1],
            start: 100,
            dur: None,
        });
        clock.set(150);
        // Never fetched on node-0: requeued, not ended, by the loss.
        fsm.apply(Event::NodeLost {
            name: "node-0".into(),
            last_heartbeat: 100,
        });
        clock.set(200);
        fsm.apply(Event::JobStarted {
            job: JobId(1),
            vertices: vec![2],
            start: 200,
            dur: None,
        });
        clock.set(230);
        fsm.apply(Event::JobFinished {
            job: JobId(1),
            end: 230,
            code: 0,
        });
        let work = workload_from_journal(&fsm.journal());
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].actual, 30, "the rerun, not 200 -> 150");
    }

    #[test]
    fn a_gang_ended_by_a_loss_runs_until_its_latest_rank_or_the_loss() {
        let clock = Arc::new(ManualClock::new(100));
        let fsm = two_nodes(&clock);
        let mut gang = one_core(1);
        gang.nodes = 2;
        fsm.apply(Event::JobSubmitted(gang));
        fsm.apply(Event::JobStarted {
            job: JobId(1),
            vertices: vec![1, 2],
            start: 100,
            dur: None,
        });
        for (node, rank) in [("node-0", 0), ("node-1", 1)] {
            fsm.apply(Event::AssignmentsFetched {
                node: node.into(),
                count: 1,
                items: vec![(JobId(1), rank)],
            });
        }
        clock.set(140);
        fsm.apply(Event::JobRankFinished {
            job: JobId(1),
            node: "node-0".into(),
            end: 140,
            code: 1,
        });
        clock.set(170);
        fsm.apply(Event::NodeLost {
            name: "node-1".into(),
            last_heartbeat: 100,
        });
        clock.set(500);
        // node-1's agent returns and reports its rank late.
        fsm.apply(Event::JobRankFinished {
            job: JobId(1),
            node: "node-1".into(),
            end: 500,
            code: 143,
        });
        let work = workload_from_journal(&fsm.journal());
        assert_eq!(work.len(), 1);
        assert_eq!(
            work[0].actual, 70,
            "ended at the loss (170), not the late report"
        );
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
