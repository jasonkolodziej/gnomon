# Graph Report - gnomon  (2026-11-12)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 539 nodes · 1290 edges · 26 communities (23 shown, 3 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 23 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `54134b6a`
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
- Community 16
- Community 17
- Community 18
- Community 19
- Community 20
- Community 21
- Community 22
- Community 23
- Community 25

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
- `Request` --references--> `Event`  [EXTRACTED]
  crates/consensus/src/types.rs → crates/core/src/lib.rs
- `declarative_priority_and_limits()` --references--> `JobId`  [INFERRED]
  crates/policy/src/lib.rs → crates/core/src/lib.rs
- `setup()` --references--> `JobId`  [INFERRED]
  crates/sched/src/bin/occ_bench.rs → crates/core/src/lib.rs
- `demo()` --references--> `Walltime`  [INFERRED]
  crates/ctl/src/main.rs → crates/core/src/lib.rs
- `job()` --references--> `Walltime`  [INFERRED]
  crates/fsm/src/lib.rs → crates/core/src/lib.rs

## Import Cycles
- None detected.

## Communities (26 total, 3 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.06
Nodes (31): CapacityProfile, Resources, backfill_finds_future_window(), capacity_profile_shapes_placement(), CapView, CapView<'a>, commit_revalidation_rejects_conflicts(), earliest_fit_respects_reservations() (+23 more)

### Community 1 - "Community 1"
Cohesion: 0.07
Nodes (9): C, io_err(), SnapBuilder, storage_pair(), Store, event_of(), NodeId, Request (+1 more)

### Community 2 - "Community 2"
Cohesion: 0.08
Nodes (22): demo(), main(), PassOptions, capacity_profile_is_honoured_by_aware_planner(), journal_replay_is_exact_and_counterfactual_reproduces_history(), Outcome, predictions_never_oversubscribe_even_when_wrong(), set_profiles() (+14 more)

### Community 3 - "Community 3"
Cohesion: 0.08
Nodes (10): job_info(), SchedulerSvc, state_name(), JobState, Cancelled, Completed, Failed, Pending (+2 more)

### Community 4 - "Community 4"
Cohesion: 0.20
Nodes (25): die(), log(), rec_end(), rec_field(), rec_start(), result(), sc(), lib.sh script (+17 more)

### Community 5 - "Community 5"
Cohesion: 0.11
Nodes (15): count_gpus(), detect(), fallback(), main(), Args, main(), parse_mem_mb(), parse_slurm_time() (+7 more)

### Community 6 - "Community 6"
Cohesion: 0.12
Nodes (6): CARBON, cores(), main(), Oracle, quantile_policy(), WINDOW_SECS

### Community 7 - "Community 7"
Cohesion: 0.19
Nodes (7): Assignment, NodeInfo, apply_event(), ensure_cluster_root(), place(), Snapshot, State

### Community 8 - "Community 8"
Cohesion: 0.20
Nodes (6): Fsm, bench_parallel(), bench_serial(), main(), make_fsm(), submit_jobs()

### Community 9 - "Community 9"
Cohesion: 0.20
Nodes (3): Job, BackfillPolicy, RuntimeEstimator

### Community 10 - "Community 10"
Cohesion: 0.18
Nodes (6): backfill_pass(), broker_coalesces_duplicate_evals(), EvalBroker, PassStats, ShadowDiff, ShadowPolicy

### Community 11 - "Community 11"
Cohesion: 0.15
Nodes (4): Clock, ManualClock, now_secs(), SystemClock

### Community 12 - "Community 12"
Cohesion: 0.23
Nodes (10): JobId, StateIndex, double_start_of_same_job_is_rejected(), early_finish_frees_the_rest_of_the_window(), job(), journal_replays_to_identical_state(), setup(), BrokerInner (+2 more)

### Community 13 - "Community 13"
Cohesion: 0.17
Nodes (6): declarative_priority_and_limits(), PartitionLimit, PolicyConfig, QosTier, wasm_priority_plugin_runs_sandboxed(), WasmPriorityPlugin

### Community 14 - "Community 14"
Cohesion: 0.17
Nodes (4): ConsensusNode, default_config(), propose(), SchedulerRaft

### Community 15 - "Community 15"
Cohesion: 0.15
Nodes (13): Event, AssignmentsFetched, Heartbeat, JobCancelled, JobExtended, JobFinished, JobReserved, JobStarted (+5 more)

### Community 16 - "Community 16"
Cohesion: 0.18
Nodes (5): Decision, Defer, Reserve, Start, Policy

### Community 17 - "Community 17"
Cohesion: 0.24
Nodes (5): Walltime, estimator_learns_user_ratio_and_respects_walltime(), heartbeat_does_not_invalidate_plans_in_per_vertex_mode(), optimistic_commit_detects_conflict(), shadow_records_disagreements_without_acting()

### Community 18 - "Community 18"
Cohesion: 0.44
Nodes (11): gnomon-agent, gnomon-api, gnomon-cli, gnomon-consensus, gnomon-core, gnomon-ctl, gnomon-fsm, gnomon-graph (+3 more)

### Community 19 - "Community 19"
Cohesion: 0.36
Nodes (7): JournalEntry, fingerprint(), from_jsonl(), rebuild(), to_jsonl(), verify_exact(), workload_from_journal()

### Community 20 - "Community 20"
Cohesion: 0.31
Nodes (6): run(), setup(), ConflictMode, GlobalIndex, PerVertex, PlanApplier

### Community 21 - "Community 21"
Cohesion: 0.36
Nodes (3): load(), main(), med()

### Community 23 - "Community 23"
Cohesion: 0.60
Nodes (4): GNOMON_HEARTBEAT_SECS, local-cluster.sh script, start(), stop()

## Knowledge Gaps
- **33 isolated node(s):** `Cluster`, `Core`, `Gpu`, `Node`, `Rack` (+28 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 148 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Fsm` connect `Community 8` to `Community 1`, `Community 2`, `Community 3`, `Community 7`, `Community 10`, `Community 11`, `Community 12`, `Community 15`, `Community 17`, `Community 19`, `Community 20`?**
  _High betweenness centrality (0.164) - this node is a cross-community bridge._
- **Are the 6 inferred relationships involving `JobId` (e.g. with `.cancel_job()` and `.get_job()`) actually correct?**
  _`JobId` has 6 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Cluster`, `Core`, `Gpu` to the rest of the system?**
  _33 weakly-connected nodes found - possible missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.06460206460206461 - nodes in this community are weakly interconnected._
- **Why does `Job` connect `Community 9` to `Community 0`, `Community 2`, `Community 3`, `Community 6`, `Community 7`, `Community 10`, `Community 11`, `Community 12`, `Community 13`, `Community 15`, `Community 16`, `Community 17`, `Community 22`?**
  _High betweenness centrality (0.116) - this node is a cross-community bridge._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.0715846994535519 - nodes in this community are weakly interconnected._
- **Why does `Resources` connect `Community 0` to `Community 2`, `Community 6`, `Community 7`, `Community 9`, `Community 11`, `Community 13`, `Community 15`?**
  _High betweenness centrality (0.108) - this node is a cross-community bridge._