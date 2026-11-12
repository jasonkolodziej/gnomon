# Graph Report - gnomon  (2026-11-12)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 492 nodes · 1195 edges · 13 communities (12 shown, 1 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 23 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `a1e4344d`
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

## God Nodes (most connected - your core abstractions)
1. `Fsm` - 45 edges
2. `Job` - 42 edges
3. `Resources` - 38 edges
4. `JobId` - 37 edges
5. `ResourceGraph` - 31 edges
6. `Event` - 28 edges
7. `Store` - 27 edges
8. `VertexId` - 25 edges
9. `NodeId` - 20 edges
10. `Policy` - 18 edges

## Surprising Connections (you probably didn't know these)
- `declarative_priority_and_limits()` --references--> `JobId`  [INFERRED]
  policy/src/lib.rs → core/src/lib.rs
- `demo()` --references--> `Walltime`  [INFERRED]
  ctl/src/main.rs → core/src/lib.rs
- `job()` --references--> `Walltime`  [INFERRED]
  fsm/src/lib.rs → core/src/lib.rs
- `declarative_priority_and_limits()` --references--> `Walltime`  [INFERRED]
  policy/src/lib.rs → core/src/lib.rs
- `submit_jobs()` --references--> `Walltime`  [INFERRED]
  sched/src/bin/bench.rs → core/src/lib.rs

## Import Cycles
- None detected.

## Communities (13 total, 1 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.05
Nodes (32): Job, JobId, StateIndex, Walltime, Snapshot, run(), setup(), backfill_pass() (+24 more)

### Community 1 - "Community 1"
Cohesion: 0.07
Nodes (30): Resources, ensure_cluster_root(), backfill_finds_future_window(), capacity_profile_shapes_placement(), CapView, CapView<'a>, commit_revalidation_rejects_conflicts(), earliest_fit_respects_reservations() (+22 more)

### Community 2 - "Community 2"
Cohesion: 0.09
Nodes (6): C, io_err(), SnapBuilder, storage_pair(), Store, NodeId

### Community 3 - "Community 3"
Cohesion: 0.09
Nodes (20): Clock, ManualClock, NodeInfo, Applied, apply_event(), double_start_of_same_job_is_rejected(), early_finish_frees_the_rest_of_the_window(), Fsm (+12 more)

### Community 4 - "Community 4"
Cohesion: 0.08
Nodes (10): job_info(), SchedulerSvc, state_name(), JobState, Cancelled, Completed, Failed, Pending (+2 more)

### Community 5 - "Community 5"
Cohesion: 0.08
Nodes (11): ConsensusNode, default_config(), propose(), SchedulerRaft, demo(), main(), bench_parallel(), bench_serial() (+3 more)

### Community 6 - "Community 6"
Cohesion: 0.13
Nodes (17): CapacityProfile, PassOptions, capacity_profile_is_honoured_by_aware_planner(), journal_replay_is_exact_and_counterfactual_reproduces_history(), Machine, Outcome, predictions_never_oversubscribe_even_when_wrong(), set_profiles() (+9 more)

### Community 7 - "Community 7"
Cohesion: 0.08
Nodes (18): event_of(), Request, Response, Assignment, Event, AssignmentsFetched, Heartbeat, JobCancelled (+10 more)

### Community 8 - "Community 8"
Cohesion: 0.11
Nodes (15): count_gpus(), detect(), fallback(), main(), Args, main(), parse_mem_mb(), parse_slurm_time() (+7 more)

### Community 9 - "Community 9"
Cohesion: 0.12
Nodes (11): CARBON, cores(), main(), Oracle, quantile_policy(), WINDOW_SECS, SimJob, header_num() (+3 more)

### Community 10 - "Community 10"
Cohesion: 0.17
Nodes (6): declarative_priority_and_limits(), PartitionLimit, PolicyConfig, QosTier, wasm_priority_plugin_runs_sandboxed(), WasmPriorityPlugin

### Community 11 - "Community 11"
Cohesion: 0.44
Nodes (11): gnomon-agent, gnomon-api, gnomon-cli, gnomon-consensus, gnomon-core, gnomon-ctl, gnomon-fsm, gnomon-graph (+3 more)

## Knowledge Gaps
- **30 isolated node(s):** `GlobalIndex`, `PerVertex`, `Defer`, `Reserve`, `Start` (+25 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 136 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Fsm` connect `Community 3` to `Community 0`, `Community 2`, `Community 4`, `Community 5`, `Community 6`?**
  _High betweenness centrality (0.195) - this node is a cross-community bridge._
- **Are the 6 inferred relationships involving `JobId` (e.g. with `.cancel_job()` and `.get_job()`) actually correct?**
  _`JobId` has 6 INFERRED edges - model-reasoned connections that need verification._
- **What connects `GlobalIndex`, `PerVertex`, `Defer` to the rest of the system?**
  _30 weakly-connected nodes found - possible missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.050436953807740326 - nodes in this community are weakly interconnected._
- **Why does `Job` connect `Community 0` to `Community 1`, `Community 3`, `Community 4`, `Community 6`, `Community 7`, `Community 9`, `Community 10`?**
  _High betweenness centrality (0.137) - this node is a cross-community bridge._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.06812291743798593 - nodes in this community are weakly interconnected._
- **Why does `Resources` connect `Community 1` to `Community 0`, `Community 3`, `Community 6`, `Community 7`, `Community 9`, `Community 10`?**
  _High betweenness centrality (0.127) - this node is a cross-community bridge._