//! Kill channel, agent side. The controller lists, on every `FetchWork`
//! reply, the jobs whose rank on this node must stop (a gang peer failed, or
//! the job was cancelled while running). Each local rank is tracked from the
//! moment its assignment is received until it has reported, with a flag that
//! [`Launcher::run_until`](crate::Launcher::run_until) and the gang barrier
//! watch. A listed job with no local rank (never received here, or already
//! gone) must still report, or the gang could never complete.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The ranks this agent is running or about to run, by job id.
#[derive(Debug, Default)]
pub struct LocalRanks {
    ranks: Mutex<HashMap<u64, Arc<AtomicBool>>>,
}

/// What to do about one `FetchWork` kill list.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct KillPlan {
    /// Local ranks that were signalled to stop now.
    pub signalled: Vec<u64>,
    /// Jobs with no local rank: report them as killed so the job can end.
    pub report: Vec<u64>,
}

impl LocalRanks {
    /// Start tracking `job`'s local rank; returns its kill flag.
    pub fn track(&self, job: u64) -> Arc<AtomicBool> {
        let mut ranks = self.ranks.lock().unwrap();
        Arc::clone(ranks.entry(job).or_default())
    }

    /// Stop tracking `job` (its rank has reported).
    pub fn untrack(&self, job: u64) {
        self.ranks.lock().unwrap().remove(&job);
    }

    pub fn is_tracked(&self, job: u64) -> bool {
        self.ranks.lock().unwrap().contains_key(&job)
    }

    /// Act on a kill list: set the flag of every tracked rank (idempotent:
    /// the controller repeats the list until the rank reports) and return
    /// the jobs that have no local rank.
    pub fn kill(&self, jobs: &[u64]) -> KillPlan {
        let ranks = self.ranks.lock().unwrap();
        let mut plan = KillPlan::default();
        for &job in jobs {
            match ranks.get(&job) {
                Some(flag) => {
                    if !flag.swap(true, Ordering::AcqRel) {
                        plan.signalled.push(job);
                    }
                }
                None => plan.report.push(job),
            }
        }
        plan
    }

    /// Signal every tracked rank to stop (agent shutdown); returns the jobs
    /// signalled now. Each rank then reports as killed through its own task.
    pub fn kill_all(&self) -> Vec<u64> {
        let ranks = self.ranks.lock().unwrap();
        let mut signalled: Vec<u64> = ranks
            .iter()
            .filter(|(_, flag)| !flag.swap(true, Ordering::AcqRel))
            .map(|(&job, _)| job)
            .collect();
        signalled.sort_unstable();
        signalled
    }

    /// True once every rank has reported (or was never started).
    pub fn is_empty(&self) -> bool {
        self.ranks.lock().unwrap().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracked_ranks_are_signalled_once_and_untracked_jobs_are_reported() {
        let local = LocalRanks::default();
        let flag = local.track(7);
        assert!(!flag.load(Ordering::Acquire));
        let plan = local.kill(&[7, 9]);
        assert_eq!(plan.signalled, [7]);
        assert_eq!(plan.report, [9], "no local rank: report it as killed");
        assert!(flag.load(Ordering::Acquire));
        assert!(
            local.kill(&[7]).signalled.is_empty(),
            "repeated kill lists signal only once"
        );
    }

    #[test]
    fn untracked_ranks_are_reported_after_they_finish() {
        let local = LocalRanks::default();
        local.track(3);
        local.untrack(3);
        assert!(!local.is_tracked(3));
        assert_eq!(local.kill(&[3]).report, [3]);
    }

    #[test]
    fn kill_all_signals_every_tracked_rank_once() {
        let local = LocalRanks::default();
        assert!(local.is_empty());
        let (a, b) = (local.track(5), local.track(2));
        local.kill(&[5]);
        assert_eq!(local.kill_all(), [2], "rank 5 was already signalled");
        assert!(a.load(Ordering::Acquire) && b.load(Ordering::Acquire));
        assert!(local.kill_all().is_empty());
        assert!(!local.is_empty(), "ranks stay tracked until they report");
        local.untrack(5);
        local.untrack(2);
        assert!(local.is_empty());
    }

    #[test]
    fn tracking_twice_shares_one_flag() {
        let local = LocalRanks::default();
        let a = local.track(1);
        let b = local.track(1);
        local.kill(&[1]);
        assert!(a.load(Ordering::Acquire) && b.load(Ordering::Acquire));
    }
}
