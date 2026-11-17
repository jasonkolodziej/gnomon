//! A replicated controller: Raft over [`crate::network`], durable storage
//! from [`crate::disk`], and an FSM whose writes go through the log.
//!
//! [`start`] opens the node's store, starts its Raft instance and RPC
//! server, initializes the static membership (a no-op on restart), and
//! attaches a [`RaftProposer`] so every `Fsm::apply*` call on this replica
//! becomes a `client_write`: committed by a majority, then applied by every
//! replica in log order at the leader-stamped time. Writes reaching a
//! follower are forwarded to the leader ([`write()`]) and return only once the
//! follower has applied them too, so a client reading back from the same
//! controller sees its own write.
//!
//! Delivery: a forwarded write is retried only when it provably was not sent
//! (the leader could not be dialled, or answered "not leader"). An error
//! after sending is returned, never retried, because the event may already
//! be committed. A leader that steps down while a write is in flight can
//! still answer "not leader" for an entry that later commits, so a retry can
//! apply an event twice. Every event the controller emits is idempotent or
//! rejected as a duplicate (`AssignmentsFetched` drains by `(job, rank)`), so
//! that is harmless.

use crate::disk::storage_pair_on_disk;
use crate::network::{serve, CallError, Peer, Reply, Rpc, TcpNetwork, WriteResult};
use crate::types::{NodeId, Request, TypeConfig as C};
use crate::{default_config, SchedulerRaft};
use gnomon_core::Clock;
use gnomon_fsm::{Applied, Fsm, ProposeError, Proposer};
use openraft::raft::ClientWriteResponse;
use openraft::{BasicNode, Raft};
use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// How long a write may wait for a leader before failing.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Static cluster membership: Raft node id → `host:port` of its Raft server.
pub type Peers = BTreeMap<NodeId, String>;

/// Parse `1=host:port,2=host:port,...`.
pub fn parse_peers(s: &str) -> Result<Peers, String> {
    let mut peers = Peers::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (id, addr) = part
            .split_once('=')
            .ok_or_else(|| format!("peer '{part}' is not <id>=<host:port>"))?;
        let id: NodeId = id
            .trim()
            .parse()
            .map_err(|_| format!("peer id '{id}' is not a number"))?;
        let addr = addr.trim();
        if !addr.contains(':') {
            return Err(format!("peer {id} address '{addr}' has no port"));
        }
        if peers.insert(id, addr.to_string()).is_some() {
            return Err(format!("peer {id} listed twice"));
        }
    }
    if peers.is_empty() {
        return Err("no peers given".into());
    }
    Ok(peers)
}

/// Commit `req`: locally when this node leads, else forwarded to the leader
/// (following "not leader" hints and waiting out elections until `timeout`),
/// then wait until this node has applied the entry.
pub async fn write(
    raft: &SchedulerRaft,
    req: Request,
    timeout: Duration,
) -> Result<ClientWriteResponse<C>, String> {
    let deadline = Instant::now() + timeout;
    let me = raft.metrics().borrow().id;
    // None: propose locally; Some: the leader we were pointed at.
    let mut leader: Option<(NodeId, String)> = None;
    loop {
        let res: WriteResult = match &leader {
            None => raft.client_write(req.clone()).await,
            Some((id, addr)) => {
                let left = deadline.saturating_duration_since(Instant::now());
                match Peer::new(*id, addr)
                    .call(&Rpc::Write(req.clone()), left)
                    .await
                {
                    Ok(Reply::Write(r)) => r,
                    Ok(_) => return Err("leader sent a mismatched reply".into()),
                    // Never sent: safe to retry once a leader is known again.
                    Err(CallError::Unreachable(e)) => {
                        eprintln!("raft: leader {id} at {addr} unreachable ({e}); retrying");
                        leader = None;
                        if !pause(deadline).await {
                            return Err(format!("no reachable leader within {timeout:?}"));
                        }
                        continue;
                    }
                    Err(e @ CallError::Network(_)) => {
                        return Err(format!("write to leader {id} failed: {e}"))
                    }
                }
            }
        };
        match res {
            Ok(resp) => {
                let left = deadline.saturating_duration_since(Instant::now());
                let applied = raft
                    .wait(Some(left.max(Duration::from_secs(1))))
                    .applied_index_at_least(Some(resp.log_id.index), "read your writes")
                    .await;
                if let Err(e) = applied {
                    // Committed: report success; reads here may lag briefly.
                    eprintln!("raft: write committed but not yet applied locally: {e}");
                }
                return Ok(resp);
            }
            Err(e) => {
                let Some(fwd) = e.forward_to_leader::<BasicNode>() else {
                    return Err(e.to_string());
                };
                leader = match (fwd.leader_id, &fwd.leader_node) {
                    (Some(id), Some(node)) if id != me => Some((id, node.addr.clone())),
                    // Election in progress (or we are the new leader).
                    _ => None,
                };
                if leader.is_none() && !pause(deadline).await {
                    return Err(format!("no leader elected within {timeout:?}"));
                }
            }
        }
    }
}

/// Back off briefly; false once `deadline` has passed.
async fn pause(deadline: Instant) -> bool {
    if Instant::now() >= deadline {
        return false;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    true
}

/// Run `f` to completion from synchronous code on `handle`'s runtime. The
/// FSM's callers are synchronous (scheduler threads, and API handlers that
/// run on a multi-threaded runtime), so this blocks in place there.
fn block_on<F: Future>(handle: &Handle, f: F) -> F::Output {
    match Handle::try_current() {
        Ok(h) if h.runtime_flavor() == RuntimeFlavor::CurrentThread => {
            panic!("a replicated FSM needs a multi-threaded tokio runtime")
        }
        Ok(h) => tokio::task::block_in_place(|| h.block_on(f)),
        Err(_) => handle.block_on(f),
    }
}

/// The [`Proposer`] of a replicated FSM: every write becomes a Raft entry
/// stamped with this replica's clock.
pub struct RaftProposer {
    raft: SchedulerRaft,
    handle: Handle,
    clock: Arc<dyn Clock>,
    timeout: Duration,
}

impl RaftProposer {
    pub fn new(raft: SchedulerRaft, handle: Handle, clock: Arc<dyn Clock>) -> RaftProposer {
        RaftProposer {
            raft,
            handle,
            clock,
            timeout: WRITE_TIMEOUT,
        }
    }
}

impl Proposer for RaftProposer {
    fn propose(
        &self,
        ev: gnomon_core::Event,
        expect: &[(u32, u64)],
    ) -> Result<Applied, ProposeError> {
        let req = Request::new(ev, self.clock.now()).with_expect(expect);
        let resp =
            block_on(&self.handle, write(&self.raft, req, self.timeout)).map_err(ProposeError)?;
        Ok(Applied {
            index: resp.data.index,
            accepted: resp.data.accepted,
        })
    }
}

/// Whether `raft` currently leads its cluster.
pub fn is_leader(raft: &SchedulerRaft) -> bool {
    let m = raft.metrics();
    let m = m.borrow();
    m.current_leader == Some(m.id)
}

/// A running replica. Dropping it does not stop it: call [`Replica::stop`].
pub struct Replica {
    pub id: NodeId,
    pub raft: SchedulerRaft,
    pub fsm: Arc<Fsm>,
    server: JoinHandle<io::Result<()>>,
}

impl Replica {
    pub fn is_leader(&self) -> bool {
        is_leader(&self.raft)
    }

    /// Stop serving peers and shut Raft down (the store stays on disk).
    pub async fn stop(self) {
        self.server.abort();
        let _ = self.server.await;
        let _ = self.raft.shutdown().await;
    }
}

/// Start replica `id` of `peers`, durable in `dir`, serving Raft on
/// `listener` (normally bound to `peers[id]`). On a fresh directory the
/// membership is initialized (every node may do this; it is a no-op once a
/// node has state); on restart the FSM is recovered from `dir` first.
pub async fn start(
    id: NodeId,
    peers: &Peers,
    dir: &Path,
    listener: TcpListener,
    clock: Arc<dyn Clock>,
) -> io::Result<Replica> {
    if !peers.contains_key(&id) {
        return Err(io::Error::other(format!(
            "node {id} is not in the peer list"
        )));
    }
    let fsm = Arc::new(Fsm::with_clock(clock.clone()));
    let (log, sm) = storage_pair_on_disk(dir, fsm.clone())?;
    let raft = Raft::new(id, default_config(), TcpNetwork, log, sm)
        .await
        .map_err(io::Error::other)?;
    let server = tokio::spawn(serve(listener, raft.clone()));
    let members: BTreeMap<NodeId, BasicNode> = peers
        .iter()
        .map(|(id, addr)| (*id, BasicNode::new(addr)))
        .collect();
    if let Err(e) = raft.initialize(members).await {
        // NotAllowed: this node already has a log (restart) — expected.
        if !matches!(
            e,
            openraft::error::RaftError::APIError(openraft::error::InitializeError::NotAllowed(_))
        ) {
            return Err(io::Error::other(format!("raft initialize: {e}")));
        }
    }
    let proposer = RaftProposer::new(raft.clone(), Handle::current(), clock);
    if fsm.set_proposer(Arc::new(proposer)).is_err() {
        unreachable!("fresh FSM has no proposer");
    }
    Ok(Replica {
        id,
        raft,
        fsm,
        server,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peers_parse_and_reject_malformed_lists() {
        let p = parse_peers("1=a:7700, 2=b:7700,3=10.0.0.3:7701").unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p[&3], "10.0.0.3:7701");
        for bad in ["", "1", "x=a:1", "1=a", "1=a:1,1=b:2"] {
            assert!(parse_peers(bad).is_err(), "{bad:?}");
        }
    }
}
