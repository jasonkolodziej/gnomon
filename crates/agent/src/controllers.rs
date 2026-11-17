//! The controllers an agent may talk to, with failover. `GNOMON_ADDR` may
//! list several (comma-separated): with replicated controllers any of them
//! serves an agent (followers forward writes to the leader), so on an error
//! the agent simply moves on to the next one.

use gnomon_api::pb::scheduler_client::SchedulerClient;
use std::time::Duration;
use tokio::sync::Mutex;
use tonic::transport::{Channel, Endpoint};

pub type Client = SchedulerClient<Channel>;

/// Bound on connecting and on every request, so a dead controller is
/// noticed instead of hanging the agent.
const RPC_TIMEOUT: Duration = Duration::from_secs(15);

/// Parse a comma-separated controller list; empty entries are ignored.
pub fn parse_addrs(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(String::from)
        .collect()
}

struct Current {
    /// Index into `addrs` of the controller in use (or tried next).
    idx: usize,
    /// Bumped on every failover, so concurrent failures of one connection
    /// rotate only once.
    generation: u64,
    client: Option<Client>,
}

pub struct Controllers {
    addrs: Vec<String>,
    current: Mutex<Current>,
}

impl Controllers {
    /// Panics on an empty list (the caller supplies a default address).
    pub fn new(addrs: Vec<String>) -> Controllers {
        assert!(!addrs.is_empty(), "no controller address");
        Controllers {
            addrs,
            current: Mutex::new(Current {
                idx: 0,
                generation: 0,
                client: None,
            }),
        }
    }

    pub fn addrs(&self) -> &[String] {
        &self.addrs
    }

    /// A client for the current controller, connecting (and moving down the
    /// list past unreachable ones) if needed. Returns it with its
    /// generation (to pass to [`Controllers::failed`]) and address.
    pub async fn client(&self) -> Result<(u64, Client, String), String> {
        let mut cur = self.current.lock().await;
        if let Some(c) = &cur.client {
            return Ok((cur.generation, c.clone(), self.addrs[cur.idx].clone()));
        }
        let mut errors = Vec::new();
        for _ in 0..self.addrs.len() {
            let addr = &self.addrs[cur.idx];
            match connect(addr).await {
                Ok(c) => {
                    cur.client = Some(c.clone());
                    return Ok((cur.generation, c, addr.clone()));
                }
                Err(e) => {
                    errors.push(format!("{addr}: {e}"));
                    cur.idx = (cur.idx + 1) % self.addrs.len();
                    cur.generation += 1;
                }
            }
        }
        Err(errors.join("; "))
    }

    /// The client of `generation` failed: drop it and move to the next
    /// controller (once per generation). Returns the address now in use.
    pub async fn failed(&self, generation: u64) -> String {
        let mut cur = self.current.lock().await;
        if cur.generation == generation {
            cur.client = None;
            cur.idx = (cur.idx + 1) % self.addrs.len();
            cur.generation += 1;
        }
        self.addrs[cur.idx].clone()
    }
}

async fn connect(addr: &str) -> Result<Client, tonic::transport::Error> {
    let channel = Endpoint::from_shared(addr.to_string())?
        .connect_timeout(RPC_TIMEOUT)
        .timeout(RPC_TIMEOUT)
        .connect()
        .await?;
    Ok(SchedulerClient::new(channel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_lists_are_split_and_trimmed() {
        assert_eq!(
            parse_addrs(" http://a:1, http://b:2,,"),
            ["http://a:1", "http://b:2"]
        );
        assert!(parse_addrs(" , ").is_empty());
    }

    #[tokio::test]
    async fn failures_rotate_once_per_generation() {
        let c = Controllers::new(parse_addrs("http://a:1,http://b:2,http://c:3"));
        assert_eq!(c.failed(0).await, "http://b:2");
        // A second task reporting the same broken connection: no extra hop.
        assert_eq!(c.failed(0).await, "http://b:2");
        assert_eq!(c.failed(1).await, "http://c:3");
        assert_eq!(c.failed(2).await, "http://a:1", "wraps around");
    }

    #[tokio::test]
    async fn an_unreachable_list_is_an_error_naming_every_controller() {
        // Reserved ports nothing listens on.
        let c = Controllers::new(parse_addrs("http://127.0.0.1:9,http://127.0.0.1:19"));
        let err = c.client().await.err().unwrap();
        assert!(
            err.contains("127.0.0.1:9") && err.contains("127.0.0.1:19"),
            "{err}"
        );
    }
}
