//! openraft type bindings: the replicated log carries scheduler Events.

use gnomon_core::Event;
use openraft::EntryPayload;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

pub type NodeId = u64;

/// A replicated log payload: one scheduler event plus the clock reading the
/// leader stamped on it. Every replica (and every restart) applies the event
/// at `at` via [`Fsm::apply_at`](gnomon_fsm::Fsm::apply_at), so state is a
/// deterministic function of the log rather than of each node's clock.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub event: Event,
    pub at: u64,
}

impl Request {
    pub fn new(event: Event, at: u64) -> Self {
        Request { event, at }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    /// FSM index after applying (the optimistic-concurrency token).
    pub index: u64,
}

openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
        NodeId = NodeId,
        Node = openraft::BasicNode,
        Entry = openraft::Entry<TypeConfig>,
        SnapshotData = Cursor<Vec<u8>>,
        AsyncRuntime = openraft::TokioRuntime,
);

/// Helper: does this entry carry a scheduler event?
pub fn event_of(entry: &openraft::Entry<TypeConfig>) -> Option<&Event> {
    request_of(entry).map(|r| &r.event)
}

/// Helper: the stamped request carried by a normal entry.
pub fn request_of(entry: &openraft::Entry<TypeConfig>) -> Option<&Request> {
    match &entry.payload {
        EntryPayload::Normal(req) => Some(req),
        _ => None,
    }
}
