//! openraft type bindings: the replicated log carries scheduler Events.

use gnomon_core::Event;
use openraft::EntryPayload;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

pub type NodeId = u64;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request(pub Event);

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
    match &entry.payload {
        EntryPayload::Normal(Request(ev)) => Some(ev),
        _ => None,
    }
}
