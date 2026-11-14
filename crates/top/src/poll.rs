//! Network side of gntop: connects to the controller and polls `ListJobs`
//! and `ListNodes`. Every failure becomes an [`Update::Error`] and the next
//! tick reconnects, so a dead controller shows "disconnected, retrying"
//! instead of ending the session.

use crate::app::Update;
use crate::model::ClusterSnapshot;
use gnomon_api::pb::scheduler_client::SchedulerClient;
use gnomon_api::pb::{ListJobsRequest, ListNodesRequest};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use tonic::transport::{Channel, Endpoint};

/// Default controller address, as for the other CLI shims.
pub const DEFAULT_ADDR: &str = "http://127.0.0.1:7631";

/// Controller address: `GNOMON_ADDR` or [`DEFAULT_ADDR`].
pub fn default_addr() -> String {
    std::env::var("GNOMON_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.into())
}

/// Wall clock, epoch seconds.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Connects with bounded connect and request timeouts, so a hung controller
/// cannot freeze the poll loop.
pub async fn connect(addr: &str, timeout: Duration) -> anyhow::Result<SchedulerClient<Channel>> {
    let channel = Endpoint::from_shared(addr.to_string())?
        .connect_timeout(timeout)
        .timeout(timeout)
        .connect()
        .await?;
    Ok(SchedulerClient::new(channel))
}

/// One poll: both listings, converted to the client model.
pub async fn fetch(client: &mut SchedulerClient<Channel>) -> anyhow::Result<ClusterSnapshot> {
    let jobs = client
        .list_jobs(ListJobsRequest::default())
        .await?
        .into_inner();
    let nodes = client
        .list_nodes(ListNodesRequest::default())
        .await?
        .into_inner();
    Ok(ClusterSnapshot::from_replies(now_secs(), jobs, nodes))
}

/// Connects (if needed) and polls once; drops the client on error so the
/// next attempt reconnects.
pub async fn poll_once(
    addr: &str,
    client: &mut Option<SchedulerClient<Channel>>,
    timeout: Duration,
) -> Update {
    if client.is_none() {
        match connect(addr, timeout).await {
            Ok(c) => *client = Some(c),
            Err(e) => return Update::Error(error_text(&e)),
        }
    }
    let Some(c) = client.as_mut() else {
        return Update::Error("not connected".into());
    };
    match fetch(c).await {
        Ok(s) => Update::Snapshot(s),
        Err(e) => {
            *client = None;
            Update::Error(error_text(&e))
        }
    }
}

/// Polls every `interval` until the receiver is dropped, skipping ticks
/// while `paused` is set.
pub async fn run_poller(
    addr: String,
    interval: Duration,
    paused: Arc<AtomicBool>,
    tx: watch::Sender<Option<Update>>,
) {
    let timeout = interval.clamp(Duration::from_secs(1), Duration::from_secs(5));
    let mut client = None;
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        if paused.load(Ordering::Relaxed) {
            continue;
        }
        let update = poll_once(&addr, &mut client, timeout).await;
        if tx.send(Some(update)).is_err() {
            return;
        }
    }
}

/// The innermost cause, which is the readable part of tonic's error chains.
fn error_text(e: &anyhow::Error) -> String {
    e.chain()
        .last()
        .map(|c| c.to_string())
        .unwrap_or_else(|| e.to_string())
}
