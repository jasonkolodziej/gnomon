# 0004 — Node-score plugins choose where, never when

**Status**: Accepted
**Date**: 2026-11-17

## Context

[ADR 0002](./0002-wasm-policy-plugins.md) listed `node-score` as the last plugin world: a plugin that
ranks candidate nodes inside `Policy::place`. It noted that this runs on the hot path, "per job per
candidate node", and must be measured before being accepted.

Placement in `gnomon-sched` is earliest-fit. `BackfillPolicy` asks the resource graph for the earliest
start at which the job fits (`match_request`, or `match_gang` for gangs), and takes the first
feasible nodes of a job-seeded rotated scan. The start time also decides reservations, so a scoring
hook that changed *when* a job starts could delay jobs or break the reservation guarantees of
conservative backfill.

The obvious interface, one call per candidate node, costs one sandbox instantiation per node per
placement. That is thousands of calls per job on a large cluster.

## Options Considered

1. **One call per candidate node.** Matches ADR 0002's wording. Simplest plugin, but the cost grows
   with `jobs × nodes` instantiations, and the plugin cannot compare nodes (only score them in
   isolation).
2. **One call per placement with every candidate (`list<node>` in, `list<s64>` out).** One
   instantiation per placement. The plugin can rank relatively. The host must lower the candidate
   list into plugin memory.
3. **Let the plugin pick the start time too.** Most expressive, but it reopens the timing decisions
   conservative backfill depends on, and every reservation would need revalidating against plugin
   output.

## Decision

Option 2, applied *after* the inner policy has decided:

```wit
record node { name: string, cores: u32, mem-mb: u64, gpus: u32,
              free-cores: u32, free-mem-mb: u64, free-gpus: u32 }
world node-score {
  use types.{job, node};
  export score: func(job: job, candidates: list<node>, now: u64) -> list<s64>;
}
```

`ScoredPolicy` (`crates/policy/src/plugin/node_score.rs`) wraps any `Policy`:

1. The inner policy places the job (start, planned span, nodes). Deferred jobs are left alone.
2. The candidates are the inner policy's nodes, plus other schedulable nodes that can hold the job
   for its **walltime** from that same start. Walltime is the conservative span, so every candidate
   is valid whatever span the estimator planned. `free-*` is capacity minus the peak use of other jobs
   over that span (`Planner::peak_usage`).
3. If there are no more candidates than nodes needed, there is nothing to choose and the plugin is
   not called. Otherwise one call scores all of them. The job takes the highest-scoring nodes. Ties
   go to the inner policy's choice, then to vertex order, so a constant scorer changes nothing.
4. The start, span and decision kind (start or reserve) are the inner policy's. The plugin chooses
   where, never when.
5. A failed call (fuel, memory, trap, or a score list of the wrong length) keeps the inner policy's
   placement and is counted and logged.

**Candidate cap.** Measured with `plugin_bench` (below), offering every feasible node costs about
175 µs per placement at 1024 nodes, which makes a full pass 7× slower. Most of that is `earliest_fit`
on every node plus lowering the list. `ScoredPolicy` therefore offers at most
`DEFAULT_MAX_CANDIDATES` (64) nodes besides the default choice. It finds them with a scan from a
job-seeded offset, as `match_gang` does, so capped calls do not all see the lowest-numbered nodes.
`--node-score-candidates N|all` changes the cap.

Like every plugin, node-score runs where the plan is built (the leader's scheduling loops). The plan
event records the chosen vertices, and replicas apply them through the per-vertex OCC check of
ADR 0001. Plans built with a node-score plugin commit through the same validation as any other.

## Measurements

`cargo run --release -p gnomon-policy --bin plugin_bench`: one full `backfill_pass` over `4 × nodes`
pending jobs (a quarter of them 2-node gangs) on 32-core nodes. The plugins are the hand-written
fixtures (`half_walltime`, `most_free_cores`), so this is the host's per-call overhead. Results are
from an Apple-silicon laptop. CI publishes them for its runners in the Benchmarks workflow.

| nodes | walltime | + estimator | + node-score, all candidates | + node-score, cap 64 | both (cap 64) |
|---:|---:|---:|---:|---:|---:|
| 64 | 1 ms | 5 ms | 8 ms | 8 ms | 12 ms |
| 256 | 7 ms | 18 ms | 54 ms | 32 ms | 43 ms |
| 1024 | 103 ms | 146 ms | 718 ms | 211 ms | 275 ms |

The estimator costs one call per job (about 10–20 µs), after which the estimate is cached. Capped
node scoring costs about 25–35 µs per placement with a choice. That is acceptable for the 1 s
backfill pass up to a few thousand nodes, and it is the budget ADR 0002 asked for.

## Consequences

- **Bounded influence.** A scorer cannot delay a job, break a reservation or place a job where it
  does not fit. It can only make placements worse (for example by fragmenting nodes). The
  overcommit test (`a_scored_backfill_pass_never_overcommits_a_node`) runs real passes with spread
  and pack scorers.
- **Capped scoring sees a sample.** With more than 64 other feasible nodes, the plugin chooses among a
  seeded sample, not the global best. Sites that need the global best can set `all` and pay for it.
- **The single-vertex SWF model cannot evaluate it.** `experiments` models a trace's machine as one
  vertex, so there is no node choice to make there. A multi-node trace model is a prerequisite for
  judging scorers offline. `ShadowPolicy` already compares placements live and works with a
  `ScoredPolicy` as its candidate.
- **The example** (`examples/policy-plugins`, feature `node-score`) is best fit: fewest cores left
  free, and jobs without GPUs avoid GPU nodes.
