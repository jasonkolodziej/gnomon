# Graph Report - gnomon  (2026-11-13)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 550 nodes · 1315 edges · 18 communities (16 shown, 2 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 28 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `89f69092`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4
- Community 5
- Community 6
- Community 7
- Community 8
- Community 9
- Community 10
- Community 11
- Community 12
- Community 13
- Community 14
- Community 15
- Community 17

## God Nodes (most connected - your core abstractions)
1. `Fsm` - 46 edges
2. `Job` - 42 edges
3. `Resources` - 38 edges
4. `JobId` - 38 edges
5. `ResourceGraph` - 31 edges
6. `Event` - 28 edges
7. `Store` - 27 edges
8. `VertexId` - 25 edges
9. `NodeId` - 20 edges
10. `Walltime` - 18 edges

## Surprising Connections (you probably didn't know these)
- `demo()` --references--> `Walltime`  [INFERRED]
  crates/ctl/src/main.rs → crates/core/src/lib.rs
- `job()` --references--> `Walltime`  [INFERRED]
  crates/fsm/src/lib.rs → crates/core/src/lib.rs
- `declarative_priority_and_limits()` --references--> `Walltime`  [INFERRED]
  crates/policy/src/lib.rs → crates/core/src/lib.rs
- `submit_jobs()` --references--> `Walltime`  [INFERRED]
  crates/sched/src/bin/bench.rs → crates/core/src/lib.rs
- `setup()` --references--> `Walltime`  [INFERRED]
  crates/sched/src/bin/occ_bench.rs → crates/core/src/lib.rs

## Import Cycles
- None detected.

## Communities (18 total, 2 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.06
Nodes (35): demo(), main(), Fsm, JournalEntry, bench_parallel(), bench_serial(), main(), make_fsm() (+27 more)

### Community 1 - "Community 1"
Cohesion: 0.07
Nodes (30): CapacityProfile, Resources, backfill_finds_future_window(), capacity_profile_shapes_placement(), CapView, CapView<'a>, commit_revalidation_rejects_conflicts(), earliest_fit_respects_reservations() (+22 more)

### Community 2 - "Community 2"
Cohesion: 0.06
Nodes (24): Job, Walltime, backfill_pass(), BackfillPolicy, broker_coalesces_duplicate_evals(), ConflictMode, GlobalIndex, PerVertex (+16 more)

### Community 3 - "Community 3"
Cohesion: 0.09
Nodes (7): C, io_err(), SnapBuilder, storage_pair(), Store, event_of(), NodeId

### Community 4 - "Community 4"
Cohesion: 0.07
Nodes (30): Assignment, Event, AssignmentsFetched, Heartbeat, JobCancelled, JobExtended, JobFinished, JobReserved (+22 more)

### Community 5 - "Community 5"
Cohesion: 0.09
Nodes (3): job_info(), SchedulerSvc, state_name()

### Community 6 - "Community 6"
Cohesion: 0.08
Nodes (12): Response, Clock, JobState, Cancelled, Completed, Failed, Pending, Reserved (+4 more)

### Community 7 - "Community 7"
Cohesion: 0.20
Nodes (25): die(), log(), rec_end(), rec_field(), rec_start(), result(), sc(), lib.sh script (+17 more)

### Community 8 - "Community 8"
Cohesion: 0.13
Nodes (14): count_gpus(), detect(), fallback(), main(), Args, main(), parse_mem_mb(), parse_slurm_time() (+6 more)

### Community 9 - "Community 9"
Cohesion: 0.13
Nodes (10): main(), node_state(), node_state_reflects_liveness_and_allocation(), resources(), declarative_priority_and_limits(), PartitionLimit, PolicyConfig, QosTier (+2 more)

### Community 10 - "Community 10"
Cohesion: 0.12
Nodes (6): CARBON, cores(), main(), Oracle, quantile_policy(), WINDOW_SECS

### Community 11 - "Community 11"
Cohesion: 0.19
Nodes (5): ConsensusNode, default_config(), propose(), SchedulerRaft, Request

### Community 12 - "Community 12"
Cohesion: 0.44
Nodes (11): gnomon-agent, gnomon-api, gnomon-cli, gnomon-consensus, gnomon-core, gnomon-ctl, gnomon-fsm, gnomon-graph (+3 more)

### Community 13 - "Community 13"
Cohesion: 0.39
Nodes (7): Outcome, SimResult, energy(), EnergySummary, pct(), summarize(), Summary

### Community 14 - "Community 14"
Cohesion: 0.36
Nodes (3): load(), main(), med()

### Community 15 - "Community 15"
Cohesion: 0.60
Nodes (4): GNOMON_HEARTBEAT_SECS, local-cluster.sh script, start(), stop()

## Knowledge Gaps
- **33 isolated node(s):** `Cluster`, `Core`, `Gpu`, `Node`, `Rack` (+28 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 146 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Fsm` connect `Community 0` to `Community 2`, `Community 3`, `Community 4`, `Community 5`, `Community 6`?**
  _High betweenness centrality (0.168) - this node is a cross-community bridge._
- **Are the 7 inferred relationships involving `JobId` (e.g. with `.cancel_job()` and `.get_job()`) actually correct?**
  _`JobId` has 7 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Cluster`, `Core`, `Gpu` to the rest of the system?**
  _33 weakly-connected nodes found - possible missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.05642080517190714 - nodes in this community are weakly interconnected._
- **Why does `Job` connect `Community 2` to `Community 0`, `Community 1`, `Community 4`, `Community 5`, `Community 6`, `Community 9`, `Community 10`?**
  _High betweenness centrality (0.112) - this node is a cross-community bridge._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.06701754385964913 - nodes in this community are weakly interconnected._
- **Why does `Resources` connect `Community 1` to `Community 0`, `Community 2`, `Community 4`, `Community 6`, `Community 9`, `Community 10`?**
  _High betweenness centrality (0.107) - this node is a cross-community bridge._