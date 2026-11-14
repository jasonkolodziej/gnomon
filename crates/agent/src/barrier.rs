//! Gang release barrier, agent side. A gang's ranks are fetched by their
//! agents on independent polls; each rank waits here until the controller
//! reports every rank fetched (`GangReady`), so the ranks start together
//! (within one barrier poll, ~200 ms) instead of one agent poll interval
//! apart. The barrier state itself is replicated FSM state, so replay is exact.

use std::fmt::Display;
use std::future::Future;
use std::time::Duration;
use tokio::time::Instant;

/// Exit code reported when a rank gave up waiting for its peers.
pub const BARRIER_TIMEOUT_EXIT: i32 = 125;

/// How often a waiting rank asks the controller.
pub const BARRIER_POLL: Duration = Duration::from_millis(200);

/// Outcome of waiting at the barrier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// Every rank has been fetched: start the job.
    Go,
    /// The job is no longer running (finished, failed or unknown): do not
    /// start it and do not report it.
    Gone,
    /// Peers never all arrived (e.g. a node died): fail this rank.
    TimedOut,
}

/// The controller's view of the barrier: `(ready, active)`.
pub type Probe = (bool, bool);

/// Ask `probe` every `poll` until the gang is released, gone, or `timeout`
/// passes. Transient errors (a controller restart) are logged and retried.
pub async fn await_release<F, Fut, E>(mut probe: F, poll: Duration, timeout: Duration) -> Release
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Probe, E>>,
    E: Display,
{
    let deadline = Instant::now() + timeout;
    loop {
        match probe().await {
            Ok((true, true)) => return Release::Go,
            Ok((_, false)) => return Release::Gone,
            Ok((false, true)) => {}
            Err(e) => eprintln!("gang barrier: {e}; retrying"),
        }
        if Instant::now() >= deadline {
            return Release::TimedOut;
        }
        tokio::time::sleep(poll).await;
    }
}

/// How long a rank waits for its peers: its walltime, or 10 minutes for
/// jobs without one.
pub fn wait_limit(walltime_secs: u64) -> Duration {
    Duration::from_secs(if walltime_secs > 0 {
        walltime_secs
    } else {
        600
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A probe that replays `script` (then repeats its last answer).
    fn scripted(
        script: Vec<Result<Probe, &'static str>>,
    ) -> impl FnMut() -> std::future::Ready<Result<Probe, &'static str>> {
        let script = RefCell::new(
            script
                .into_iter()
                .collect::<std::collections::VecDeque<_>>(),
        );
        move || {
            let mut s = script.borrow_mut();
            let next = if s.len() > 1 {
                s.pop_front().unwrap()
            } else {
                *s.front().unwrap()
            };
            std::future::ready(next)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn waits_until_every_rank_is_fetched() {
        let probe = scripted(vec![Ok((false, true)), Ok((false, true)), Ok((true, true))]);
        let t0 = Instant::now();
        let r = await_release(probe, BARRIER_POLL, Duration::from_secs(60)).await;
        assert_eq!(r, Release::Go);
        assert_eq!(t0.elapsed(), BARRIER_POLL * 2, "two polls of waiting");
    }

    #[tokio::test(start_paused = true)]
    async fn a_finished_job_is_not_started() {
        let probe = scripted(vec![Ok((false, true)), Ok((false, false))]);
        let r = await_release(probe, BARRIER_POLL, Duration::from_secs(60)).await;
        assert_eq!(r, Release::Gone);
    }

    #[tokio::test(start_paused = true)]
    async fn missing_peers_time_out_at_the_limit() {
        let probe = scripted(vec![Ok((false, true))]);
        let t0 = Instant::now();
        let r = await_release(probe, BARRIER_POLL, Duration::from_secs(5)).await;
        assert_eq!(r, Release::TimedOut);
        assert!(t0.elapsed() >= Duration::from_secs(5));
        assert!(t0.elapsed() < Duration::from_secs(5) + BARRIER_POLL * 2);
    }

    #[tokio::test(start_paused = true)]
    async fn controller_errors_are_retried() {
        let probe = scripted(vec![
            Err("unavailable"),
            Err("unavailable"),
            Ok((true, true)),
        ]);
        let r = await_release(probe, BARRIER_POLL, Duration::from_secs(60)).await;
        assert_eq!(r, Release::Go);
    }

    #[test]
    fn wait_limit_is_the_walltime_or_ten_minutes() {
        assert_eq!(wait_limit(90), Duration::from_secs(90));
        assert_eq!(wait_limit(0), Duration::from_secs(600));
    }
}
