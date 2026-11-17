//! Raft network transport over TCP.
//!
//! One frame per message: a big-endian `u32` length, then JSON (the payload
//! types carry `#[serde(default)]`/`skip_serializing_if` fields, which a
//! non-self-describing format such as bincode cannot round-trip). A
//! connection carries one request at a time; openraft opens one client per
//! replication target, so each peer pair needs only a few long-lived
//! connections, re-dialled after any error.
//!
//! Besides the three Raft RPCs the server accepts [`Rpc::Write`]: a client
//! write forwarded by a follower to the leader (see
//! [`crate::replica::write`]).

use crate::types::{NodeId, Request, TypeConfig as C};
use crate::SchedulerRaft;
use openraft::error::{
    ClientWriteError, InstallSnapshotError, NetworkError, RPCError, RaftError, RemoteError,
    Unreachable,
};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, ClientWriteResponse, InstallSnapshotRequest,
    InstallSnapshotResponse, VoteRequest, VoteResponse,
};
use openraft::BasicNode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

/// Frames above this are refused (a corrupt length must not allocate GBs).
const MAX_FRAME: usize = 256 << 20;

/// A request on the wire.
#[derive(Serialize, Deserialize)]
pub enum Rpc {
    Append(AppendEntriesRequest<C>),
    Snapshot(InstallSnapshotRequest<C>),
    Vote(VoteRequest<NodeId>),
    /// A client write forwarded to the leader.
    Write(Request),
}

pub type WriteResult =
    Result<ClientWriteResponse<C>, RaftError<NodeId, ClientWriteError<NodeId, BasicNode>>>;

/// The reply to the [`Rpc`] of the same variant.
#[derive(Serialize, Deserialize)]
pub enum Reply {
    Append(Result<AppendEntriesResponse<NodeId>, RaftError<NodeId>>),
    Snapshot(Result<InstallSnapshotResponse<NodeId>, RaftError<NodeId, InstallSnapshotError>>),
    Vote(Result<VoteResponse<NodeId>, RaftError<NodeId>>),
    Write(WriteResult),
}

async fn write_frame<T: Serialize>(s: &mut TcpStream, v: &T) -> io::Result<()> {
    let body = serde_json::to_vec(v).map_err(io::Error::other)?;
    let len = u32::try_from(body.len()).map_err(io::Error::other)?;
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&body);
    s.write_all(&buf).await?;
    s.flush().await
}

async fn read_frame<T: DeserializeOwned>(s: &mut TcpStream) -> io::Result<T> {
    let len = s.read_u32().await? as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds {MAX_FRAME}"),
        ));
    }
    let mut body = vec![0; len];
    s.read_exact(&mut body).await?;
    serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Serve Raft RPCs (and forwarded writes) for `raft` on `listener` until the
/// returned future is dropped or aborted, which also drops every open
/// connection.
pub async fn serve(listener: TcpListener, raft: SchedulerRaft) -> io::Result<()> {
    let mut conns = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (sock, _) = accepted?;
                let _ = sock.set_nodelay(true);
                conns.spawn(handle(sock, raft.clone()));
            }
            Some(_) = conns.join_next(), if !conns.is_empty() => {}
        }
    }
}

async fn handle(mut sock: TcpStream, raft: SchedulerRaft) {
    loop {
        let rpc: Rpc = match read_frame(&mut sock).await {
            Ok(rpc) => rpc,
            Err(e) => {
                if e.kind() != io::ErrorKind::UnexpectedEof {
                    eprintln!("raft transport: dropping connection: {e}");
                }
                return;
            }
        };
        let reply = match rpc {
            Rpc::Append(req) => Reply::Append(raft.append_entries(req).await),
            Rpc::Snapshot(req) => Reply::Snapshot(raft.install_snapshot(req).await),
            Rpc::Vote(req) => Reply::Vote(raft.vote(req).await),
            Rpc::Write(req) => Reply::Write(raft.client_write(req).await),
        };
        if write_frame(&mut sock, &reply).await.is_err() {
            return;
        }
    }
}

/// Why a call failed, from the caller's point of view.
#[derive(Debug)]
pub enum CallError {
    /// Could not connect: the request was never sent.
    Unreachable(io::Error),
    /// Failed after the request may have been sent (I/O error or timeout).
    Network(io::Error),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Unreachable(e) => write!(f, "unreachable: {e}"),
            CallError::Network(e) => write!(f, "network: {e}"),
        }
    }
}

impl CallError {
    fn into_rpc<E: std::error::Error>(self) -> RPCError<NodeId, BasicNode, E> {
        match self {
            CallError::Unreachable(e) => RPCError::Unreachable(Unreachable::new(&e)),
            CallError::Network(e) => RPCError::Network(NetworkError::new(&e)),
        }
    }
}

/// A connection to one peer, re-dialled lazily after any failure.
pub struct Peer {
    target: NodeId,
    addr: String,
    conn: Option<TcpStream>,
}

impl Peer {
    pub fn new(target: NodeId, addr: &str) -> Peer {
        Peer {
            target,
            addr: addr.to_string(),
            conn: None,
        }
    }

    /// One request/reply exchange, bounded by `ttl` (connect included).
    pub async fn call(&mut self, rpc: &Rpc, ttl: Duration) -> Result<Reply, CallError> {
        let deadline = tokio::time::Instant::now() + ttl;
        if self.conn.is_none() {
            let sock = tokio::time::timeout_at(deadline, TcpStream::connect(&self.addr))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connect timed out"))
                .and_then(|r| r)
                .map_err(CallError::Unreachable)?;
            let _ = sock.set_nodelay(true);
            self.conn = Some(sock);
        }
        let sock = self.conn.as_mut().expect("connected above");
        let exchange = async {
            write_frame(sock, rpc).await?;
            read_frame::<Reply>(sock).await
        };
        let out = match tokio::time::timeout_at(deadline, exchange).await {
            Ok(Ok(reply)) => return Ok(reply),
            Ok(Err(e)) => e,
            Err(_) => io::Error::new(io::ErrorKind::TimedOut, "rpc timed out"),
        };
        // The stream may hold a half-read reply: never reuse it.
        self.conn = None;
        Err(CallError::Network(out))
    }

    fn remote<E: std::error::Error>(&self, e: E) -> RPCError<NodeId, BasicNode, E> {
        RPCError::RemoteError(RemoteError::new(self.target, e))
    }
}

fn mismatched<E: std::error::Error>() -> RPCError<NodeId, BasicNode, E> {
    RPCError::Network(NetworkError::new(&io::Error::new(
        io::ErrorKind::InvalidData,
        "reply does not match the request",
    )))
}

type RpcResult<T, E = RaftError<NodeId>> = Result<T, RPCError<NodeId, BasicNode, E>>;

impl RaftNetwork<C> for Peer {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> RpcResult<AppendEntriesResponse<NodeId>> {
        match self
            .call(&Rpc::Append(req), option.hard_ttl())
            .await
            .map_err(CallError::into_rpc)?
        {
            Reply::Append(r) => r.map_err(|e| self.remote(e)),
            _ => Err(mismatched()),
        }
    }

    async fn install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<C>,
        option: RPCOption,
    ) -> RpcResult<InstallSnapshotResponse<NodeId>, RaftError<NodeId, InstallSnapshotError>> {
        match self
            .call(&Rpc::Snapshot(req), option.hard_ttl())
            .await
            .map_err(CallError::into_rpc)?
        {
            Reply::Snapshot(r) => r.map_err(|e| self.remote(e)),
            _ => Err(mismatched()),
        }
    }

    async fn vote(
        &mut self,
        req: VoteRequest<NodeId>,
        option: RPCOption,
    ) -> RpcResult<VoteResponse<NodeId>> {
        match self
            .call(&Rpc::Vote(req), option.hard_ttl())
            .await
            .map_err(CallError::into_rpc)?
        {
            Reply::Vote(r) => r.map_err(|e| self.remote(e)),
            _ => Err(mismatched()),
        }
    }
}

/// Dials peers at the `addr` of their [`BasicNode`].
#[derive(Clone, Copy, Debug, Default)]
pub struct TcpNetwork;

impl RaftNetworkFactory<C> for TcpNetwork {
    type Network = Peer;

    async fn new_client(&mut self, target: NodeId, node: &BasicNode) -> Peer {
        Peer::new(target, &node.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnomon_core::Event;

    #[tokio::test]
    async fn frames_round_trip_and_oversized_lengths_are_refused() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let rpc: Rpc = read_frame(&mut s).await.unwrap();
            assert!(matches!(rpc, Rpc::Write(ref r) if r.at == 42));
            write_frame(&mut s, &rpc).await.unwrap();
            // Then a bogus 1 GiB length.
            s.write_all(&(1u32 << 30).to_be_bytes()).await.unwrap();
        });
        let mut c = TcpStream::connect(addr).await.unwrap();
        let req = Request::new(Event::JobCancelled(gnomon_core::JobId(3)), 42);
        write_frame(&mut c, &Rpc::Write(req)).await.unwrap();
        let echoed: Rpc = read_frame(&mut c).await.unwrap();
        assert!(matches!(echoed, Rpc::Write(r) if r.at == 42));
        let err = read_frame::<Rpc>(&mut c).await.err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn an_unreachable_peer_is_reported_as_never_sent() {
        // Bind then drop: nothing listens on this port any more.
        let addr = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let mut peer = Peer::new(2, &addr.to_string());
        let rpc = Rpc::Write(Request::new(Event::JobCancelled(gnomon_core::JobId(1)), 0));
        let err = peer.call(&rpc, Duration::from_secs(2)).await.err().unwrap();
        assert!(matches!(err, CallError::Unreachable(_)), "{err}");
    }
}
