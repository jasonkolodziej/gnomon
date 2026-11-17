# 0001 — Route controller writes through Raft with a Proposer hook on the FSM

**Status**: Accepted
**Date**: 2026-11-17

## Context

The controller mutated its FSM directly from many call sites: the gRPC handlers (submit, cancel,
register, heartbeat, fetch work, report completion), the plan applier (per-vertex optimistic commits
with a read set) and the 1 s backfill pass. Durable Raft storage (`DiskStore`) existed, but nothing
replicated it. Multi-node replication requires every mutation to become a committed log entry that
each replica applies in the same order, at the same timestamp, with the same outcome.

## Options Considered

1. **Make every call site async and call `client_write`.** Explicit, but it touches the scheduler's
   synchronous worker threads and `backfill_pass`, which the simulator and replay gate share. It
   would also fork the production and simulated scheduling paths.
2. **A `Proposer` hook on the FSM.** Once a proposer is attached, `Fsm::apply*` proposes the event and
   blocks until it is committed and applied locally. The committed-entry path (`apply_at`, used by
   Raft and recovery) bypasses the hook. Call sites, the simulator and replay stay unchanged. The
   cost is that synchronous callers block a thread per write (`block_in_place` on runtime workers).
3. **Leader-only controller with standby replicas (active/passive).** Simpler, but followers could not
   serve clients, and failover would still need the replicated log underneath.

## Decision

Option 2. The log entry carries the event, the leader-stamped time, and the plan's per-vertex read set
(`Request.expect`). Every replica therefore revalidates a plan against identical state and reaches the
same accept/reject outcome, which `Response.accepted` reports back. Followers forward writes to the
leader over the Raft TCP transport and wait until they have applied the entry, so a client reading
from the same controller sees its own write. Only the leader runs the scheduling loops, and a new
leader re-plans everything pending. Job ids stay allocated by the receiving replica. Taken ids are
rejected by the FSM and retried by the API, which also closes a pre-existing race between concurrent
submissions.

## Consequences

- One code path for single-node, replicated, simulated and replayed scheduling. The replay gate
  stays the regression oracle for all of them.
- Writes cost a quorum round trip and an fsync. Throughput-sensitive paths (`occ_bench`) are
  unaffected when not replicated: the hook is a single `OnceLock` check.
- Delivery is at-least-once across a leader change: a write is retried only when it was provably not
  sent, but a leader stepping down mid-write may answer "not leader" for an entry that later
  commits. Every controller event therefore tolerates a second application: duplicates are no-ops
  or rejected, and `AssignmentsFetched` names the `(job, rank)`s it drains instead of a count.
- Membership is static (`--raft-peers`). Adding or removing controllers at runtime is the next step.
- The same reasoning keeps the gang kill channel in FSM state (`State::kills`), not in an agent-side
  RPC, so it is replicated and replays exactly.
