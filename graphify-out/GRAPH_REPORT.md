# Graph Report - gnomon  (2026-11-13)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 1258 nodes · 3495 edges · 53 communities (48 shown, 5 thin omitted)
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 173 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `27a714f5`
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
- Community 24
- Community 25
- Community 26
- Community 27
- Community 28
- Community 29
- Community 30
- Community 31
- Community 32
- Community 33
- Community 34
- Community 35
- Community 36
- Community 37
- Community 38
- Community 39
- Community 40
- Community 41
- Community 42
- Community 43
- Community 44
- Community 45
- Community 46
- Community 47
- Community 48
- Community 49
- Community 51

## God Nodes (most connected - your core abstractions)
1. `Fsm` - 75 edges
2. `JobId` - 72 edges
3. `Job` - 71 edges
4. `NodeId` - 50 edges
5. `Resources` - 41 edges
6. `ResourceGraph` - 41 edges
7. `DiskStore` - 37 edges
8. `Event` - 36 edges
9. `VertexId` - 31 edges
10. `Walltime` - 30 edges

## Surprising Connections (you probably didn't know these)
- `container_image_flows_from_submit_to_job_info_and_fetch_work()` --references--> `JobId`  [INFERRED]
  crates/api/src/lib.rs → crates/core/src/lib.rs
- `fsm_with_node()` --references--> `JobId`  [INFERRED]
  crates/api/src/lib.rs → crates/core/src/lib.rs
- `job_info_falls_back_to_walltime_and_no_nodes_while_pending()` --references--> `JobId`  [INFERRED]
  crates/api/src/lib.rs → crates/core/src/lib.rs
- `job_info_reports_planned_span_and_nodes_once_placed()` --references--> `JobId`  [INFERRED]
  crates/api/src/lib.rs → crates/core/src/lib.rs
- `event()` --references--> `JobId`  [INFERRED]
  crates/consensus/src/disk.rs → crates/core/src/lib.rs

## Import Cycles
- 1-file cycle: `crates/consensus/src/wal.rs -> crates/consensus/src/wal.rs`
- 2-file cycle: `crates/agent/src/container.rs -> crates/agent/src/launch.rs -> crates/agent/src/container.rs`
- 2-file cycle: `crates/sched/src/bias.rs -> crates/sched/src/lib.rs -> crates/sched/src/bias.rs`

## Communities (53 total, 5 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.05
Nodes (63): Args, Baseline, cluster(), DEFAULT_BASELINE, gang_workload_marks_every_seventh_job(), identical_measures_do_not_regress(), improvements_and_lower_utilization_are_judged_by_direction(), m() (+55 more)

### Community 1 - "Community 1"
Cohesion: 0.06
Nodes (42): CapacityProfile, Resources, ensure_cluster_root(), backfill_finds_future_window(), capacity_profile_shapes_placement(), CapView, CapView<'a>, commit_revalidation_rejects_conflicts() (+34 more)

### Community 2 - "Community 2"
Cohesion: 0.06
Nodes (33): Args, container_directive_both_spellings(), Directives, directives_combine_per_node_shape(), main(), nodes_flag_parses_count_and_range(), parse_mem_mb(), parse_nodes() (+25 more)

### Community 3 - "Community 3"
Cohesion: 0.09
Nodes (11): apply_entries(), C, encode_image(), io_err(), restore_image(), SnapBuilder, snapshot_meta(), storage_pair() (+3 more)

### Community 4 - "Community 4"
Cohesion: 0.08
Nodes (24): container_image_flows_from_submit_to_job_info_and_fetch_work(), empty_container_means_none(), fsm_with_node(), gang_job_flows_through_submit_fetch_and_per_rank_completion(), job(), job_info(), job_info_falls_back_to_walltime_and_no_nodes_while_pending(), job_info_reports_planned_span_and_nodes_once_placed() (+16 more)

### Community 5 - "Community 5"
Cohesion: 0.08
Nodes (17): count_gpus(), detect(), fallback(), main(), connect(), Args, main(), run_interactive() (+9 more)

### Community 6 - "Community 6"
Cohesion: 0.09
Nodes (21): CONTROLLERS, CPU_PERIOD_US, create_writes_limit_files_under_job_path(), DEFAULT_ROOT, detect_enables_controllers_and_creates_base(), detect_rejects_non_cgroup2_root(), detect_reports_unwritable_root(), detect_requires_cpu_and_memory() (+13 more)

### Community 7 - "Community 7"
Cohesion: 0.11
Nodes (28): bar_label(), durations_format_compactly(), fmt_dur(), fmt_rel(), gantt_lanes(), gutter_shows_allocation_and_liveness(), gutter_text(), job() (+20 more)

### Community 8 - "Community 8"
Cohesion: 0.12
Nodes (24): bars_land_on_their_time_columns_with_labels(), buffer_text(), disconnected_with_data_keeps_the_stale_plan(), disconnected_without_data_says_retrying(), draw(), filter_prompt_replaces_the_key_help(), fixture(), header_shows_counts_index_and_window() (+16 more)

### Community 9 - "Community 9"
Cohesion: 0.10
Nodes (25): App, app(), Conn, Connected, Connecting, Disconnected, crossterm_events_map_to_keys(), errors_mark_disconnected_but_keep_the_last_snapshot() (+17 more)

### Community 10 - "Community 10"
Cohesion: 0.10
Nodes (9): LOG, META, NoNetwork, read_json(), RpcResult, start(), storage_pair_on_disk(), unreachable() (+1 more)

### Community 11 - "Community 11"
Cohesion: 0.14
Nodes (16): assignment(), confined_job_joins_cgroup_and_cleans_up(), current_user(), env_of(), Launcher, launcher(), LaunchSpec, missing_container_runtime_fails_cleanly() (+8 more)

### Community 12 - "Community 12"
Cohesion: 0.15
Nodes (21): append_reopen_returns_every_record_in_order(), atomic_write(), atomic_write_replaces_and_reads_back(), checksum_failure_on_final_record_is_a_torn_tail(), corrupt(), corrupted_middle_record_is_an_error(), crc32(), crc32_matches_the_standard_check_value() (+13 more)

### Community 13 - "Community 13"
Cohesion: 0.19
Nodes (27): die(), log(), rec_end(), rec_field(), rec_start(), result(), sc(), lib.sh script (+19 more)

### Community 14 - "Community 14"
Cohesion: 0.11
Nodes (16): Bar, job_info(), JobState, .ALL, Cancelled, Completed, Failed, Pending (+8 more)

### Community 15 - "Community 15"
Cohesion: 0.15
Nodes (7): DiskLogReader, DiskStore, LogMeta, Membership, read_range(), SharedLog, VOTE

### Community 16 - "Community 16"
Cohesion: 0.13
Nodes (13): Assignment, job_nodes_defaults_to_one_for_old_records(), JobId, NodeList, now_secs(), SystemClock, Walltime, zero_nodes_counts_as_one() (+5 more)

### Community 17 - "Community 17"
Cohesion: 0.16
Nodes (15): append_truncate_purge_round_trip_across_reopen(), await_leader(), committed_events_survive_restart(), cores(), corrupted_middle_log_record_is_an_error(), entries(), entry(), event() (+7 more)

### Community 18 - "Community 18"
Cohesion: 0.12
Nodes (12): BiasConfig, GuardMode, Block, Record, bias_policy(), bias_variants(), CARBON, cores() (+4 more)

### Community 19 - "Community 19"
Cohesion: 0.11
Nodes (8): event_of(), Request, request_of(), PartitionLimit, PolicyConfig, QosTier, wasm_priority_plugin_runs_sandboxed(), WasmPriorityPlugin

### Community 20 - "Community 20"
Cohesion: 0.14
Nodes (3): Job, BackfillPolicy, Policy

### Community 21 - "Community 21"
Cohesion: 0.29
Nodes (19): double_start_of_same_job_is_rejected(), early_finish_frees_the_rest_of_the_window(), gang_completes_only_when_every_rank_reports(), gang_fails_if_any_rank_fails(), gang_journal_replays_to_identical_state(), gang_placement_with_wrong_vertex_count_or_duplicates_is_rejected(), gang_setup(), gang_start_enqueues_one_ranked_assignment_per_node() (+11 more)

### Community 22 - "Community 22"
Cohesion: 0.16
Nodes (7): Clock, Fsm, bench_parallel(), bench_serial(), main(), make_fsm(), submit_jobs()

### Community 23 - "Community 23"
Cohesion: 0.17
Nodes (11): drive(), EventJournal, fingerprint(), JOURNAL, journal_recovers_identical_state_across_restarts(), Record, records_already_in_the_image_are_skipped(), STATE (+3 more)

### Community 24 - "Community 24"
Cohesion: 0.23
Nodes (14): bias_correction_converges_to_target_coverage(), bias_correction_is_per_user_with_global_fallback(), CORE_BUCKETS, guard_modes_shape_the_spans(), job(), log_ratio(), MIN_RATIO, plans_walltime_without_history() (+6 more)

### Community 25 - "Community 25"
Cohesion: 0.25
Nodes (15): backfill_pass(), backfill_pass_shows_the_queue_before_clearing_reservations(), backfill_pass_starts_gang_and_counts_it_once(), cluster(), conservative_backfill_protects_gang_reservation(), gang_starts_now_or_reserves_in_both_policy_paths(), guard_span_blocks_backfill_in_front_of_a_reservation(), machine_shape_reads_the_graph() (+7 more)

### Community 26 - "Community 26"
Cohesion: 0.17
Nodes (11): JournalEntry, fingerprint(), fingerprint_includes_node_and_assignment_state(), from_jsonl(), jsonl_parse_errors_report_original_line_number(), JsonlParseError, node_fsm(), rebuild() (+3 more)

### Community 27 - "Community 27"
Cohesion: 0.14
Nodes (15): Event, AssignmentsFetched, Heartbeat, JobCancelled, JobExtended, JobFinished, JobRankFinished, JobReserved (+7 more)

### Community 28 - "Community 28"
Cohesion: 0.26
Nodes (14): container_name(), ContainerRuntime, docker_argv_is_complete_and_ordered(), has_pair(), job_env_is_passed_with_e(), kill_argv(), kill_targets_the_named_container(), podman_omits_gpus_and_keeps_user_id() (+6 more)

### Community 29 - "Community 29"
Cohesion: 0.21
Nodes (7): NodeInfo, apply_event(), finish(), place(), rank_finished(), Snapshot, State

### Community 30 - "Community 30"
Cohesion: 0.30
Nodes (12): fill_char(), fill_style(), label_style(), state_color(), states_map_to_fill_and_colour(), draw_bars(), draw_gutter(), render() (+4 more)

### Community 31 - "Community 31"
Cohesion: 0.25
Nodes (5): ConsensusNode, default_config(), propose(), propose_at(), SchedulerRaft

### Community 32 - "Community 32"
Cohesion: 0.19
Nodes (5): DiskSnapBuilder, Meta, SNAPSHOT, StoredSnapshot, data_err()

### Community 34 - "Community 34"
Cohesion: 0.22
Nodes (7): container_jobs_run_through_runtime_and_walltime_kills_container(), FAKE, spec(), exit_code_is_reported(), launcher(), spec(), walltime_kills_long_job_with_exit_124()

### Community 35 - "Community 35"
Cohesion: 0.26
Nodes (10): args(), demo(), main(), open_fsm(), requeue_pending(), restart_with_state_dir_recovers_jobs_nodes_and_reservations(), state_dir(), state_dir_flag_beats_env_and_empty_env_is_ignored() (+2 more)

### Community 36 - "Community 36"
Cohesion: 0.28
Nodes (7): Cgroups, enable_controllers(), Unavailable, Disabled, MissingController, NotCgroup2, NotWritable

### Community 37 - "Community 37"
Cohesion: 0.19
Nodes (4): all_spans(), FixedSpans, RuntimeEstimator, Spans

### Community 38 - "Community 38"
Cohesion: 0.39
Nodes (12): gnomon-agent, gnomon-api, gnomon-cli, gnomon-consensus, gnomon-core, gnomon-ctl, gnomon-fsm, gnomon-graph (+4 more)

### Community 39 - "Community 39"
Cohesion: 0.24
Nodes (6): Decision, Defer, Reserve, Start, Plan, ShadowDiff

### Community 40 - "Community 40"
Cohesion: 0.29
Nodes (6): Cell, core_bucket(), log2_bucket(), State, UserCells, wall_bucket()

### Community 41 - "Community 41"
Cohesion: 0.24
Nodes (3): estimator_learns_user_ratio_and_respects_walltime(), shadow_records_disagreements_without_acting(), ShadowPolicy

### Community 42 - "Community 42"
Cohesion: 0.28
Nodes (4): FsmImage, StateIndex, EventSink, VecSink

### Community 43 - "Community 43"
Cohesion: 0.28
Nodes (4): broker_coalesces_duplicate_evals(), BrokerInner, Eval, EvalBroker

### Community 44 - "Community 44"
Cohesion: 0.36
Nodes (3): load(), main(), med()

### Community 45 - "Community 45"
Cohesion: 0.36
Nodes (5): run(), ConflictMode, GlobalIndex, PerVertex, PlanApplier

### Community 47 - "Community 47"
Cohesion: 0.53
Nodes (4): bad_durations_are_rejected_by_the_parser(), gntop(), once_honours_window_and_user_flags(), once_prints_a_plain_disconnected_frame_and_fails()

### Community 48 - "Community 48"
Cohesion: 0.60
Nodes (4): GNOMON_HEARTBEAT_SECS, local-cluster.sh script, start(), stop()

### Community 49 - "Community 49"
Cohesion: 0.70
Nodes (4): slurm-local.sh script, start(), stop(), write_conf()

## Knowledge Gaps
- **58 isolated node(s):** `Cluster`, `Core`, `Gpu`, `Node`, `Rack` (+53 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 232 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **5 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Fsm` connect `Community 22` to `Community 32`, `Community 0`, `Community 3`, `Community 4`, `Community 35`, `Community 10`, `Community 42`, `Community 15`, `Community 16`, `Community 17`, `Community 21`, `Community 23`, `Community 25`, `Community 26`, `Community 27`, `Community 29`?**
  _High betweenness centrality (0.077) - this node is a cross-community bridge._
- **Are the 19 inferred relationships involving `JobId` (e.g. with `container_image_flows_from_submit_to_job_info_and_fetch_work()` and `fsm_with_node()`) actually correct?**
  _`JobId` has 19 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Cluster`, `Core`, `Gpu` to the rest of the system?**
  _58 weakly-connected nodes found - possible missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.051839464882943144 - nodes in this community are weakly interconnected._
- **Why does `Job` connect `Community 20` to `Community 0`, `Community 1`, `Community 33`, `Community 4`, `Community 37`, `Community 39`, `Community 40`, `Community 41`, `Community 43`, `Community 46`, `Community 16`, `Community 18`, `Community 19`, `Community 21`, `Community 24`, `Community 25`, `Community 27`, `Community 29`?**
  _High betweenness centrality (0.063) - this node is a cross-community bridge._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.05948399426660296 - nodes in this community are weakly interconnected._
- **Why does `Resources` connect `Community 1` to `Community 0`, `Community 10`, `Community 16`, `Community 17`, `Community 18`, `Community 19`, `Community 20`, `Community 27`, `Community 29`?**
  _High betweenness centrality (0.059) - this node is a cross-community bridge._