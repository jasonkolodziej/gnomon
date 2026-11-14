# Graph Report - gnomon  (2026-11-14)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 1339 nodes · 3719 edges · 53 communities (48 shown, 5 thin omitted)
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 190 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `52b52516`
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
- Community 51

## God Nodes (most connected - your core abstractions)
1. `JobId` - 78 edges
2. `Fsm` - 75 edges
3. `Job` - 71 edges
4. `NodeId` - 50 edges
5. `Resources` - 44 edges
6. `ResourceGraph` - 41 edges
7. `Event` - 41 edges
8. `DiskStore` - 38 edges
9. `VertexId` - 31 edges
10. `Walltime` - 31 edges

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
- 2-file cycle: `crates/sched/src/bias.rs -> crates/sched/src/lib.rs -> crates/sched/src/bias.rs`
- 2-file cycle: `crates/agent/src/container.rs -> crates/agent/src/launch.rs -> crates/agent/src/container.rs`

## Communities (53 total, 5 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.05
Nodes (69): bounded_run_reports_status_and_times_out(), container_name(), container_user(), ContainerRuntime, ContainerUser, Agent, Job, detect_docker_rootless() (+61 more)

### Community 1 - "Community 1"
Cohesion: 0.05
Nodes (64): CapacityProfile, bias_policy(), bias_variants(), CARBON, cores(), main(), Oracle, quantile_policy() (+56 more)

### Community 2 - "Community 2"
Cohesion: 0.06
Nodes (40): Resources, ensure_cluster_root(), backfill_finds_future_window(), capacity_profile_shapes_placement(), CapView, CapView<'a>, commit_revalidation_rejects_conflicts(), earliest_fit_respects_reservations() (+32 more)

### Community 3 - "Community 3"
Cohesion: 0.07
Nodes (25): container_image_flows_from_submit_to_job_info_and_fetch_work(), empty_container_means_none(), fsm_with_node(), gang_job_flows_through_submit_fetch_and_per_rank_completion(), gang_ready_of(), job(), job_info(), job_info_falls_back_to_walltime_and_no_nodes_while_pending() (+17 more)

### Community 4 - "Community 4"
Cohesion: 0.07
Nodes (29): Cgroups, CONTROLLERS, CPU_PERIOD_US, create_writes_limit_files_under_job_path(), DEFAULT_ROOT, detect_enables_controllers_and_creates_base(), detect_rejects_non_cgroup2_root(), detect_reports_unwritable_root() (+21 more)

### Community 5 - "Community 5"
Cohesion: 0.09
Nodes (10): apply_entries(), encode_image(), io_err(), restore_image(), SnapBuilder, snapshot_meta(), storage_pair(), Store (+2 more)

### Community 6 - "Community 6"
Cohesion: 0.10
Nodes (34): a_second_process_cannot_open_a_journal_in_use(), drive(), EventJournal, fingerprint(), JOURNAL, journal_recovers_identical_state_across_restarts(), records_already_in_the_image_are_skipped(), STATE (+26 more)

### Community 7 - "Community 7"
Cohesion: 0.11
Nodes (28): bar_label(), durations_format_compactly(), fmt_dur(), fmt_rel(), gantt_lanes(), gutter_shows_allocation_and_liveness(), gutter_text(), job() (+20 more)

### Community 8 - "Community 8"
Cohesion: 0.09
Nodes (23): Args, container_directive_both_spellings(), Directives, directives_combine_per_node_shape(), main(), nodes_flag_parses_count_and_range(), parse_mem_mb(), parse_nodes() (+15 more)

### Community 9 - "Community 9"
Cohesion: 0.12
Nodes (24): bars_land_on_their_time_columns_with_labels(), buffer_text(), disconnected_with_data_keeps_the_stale_plan(), disconnected_without_data_says_retrying(), draw(), filter_prompt_replaces_the_key_help(), fixture(), header_shows_counts_index_and_window() (+16 more)

### Community 10 - "Community 10"
Cohesion: 0.10
Nodes (25): App, app(), Conn, Connected, Connecting, Disconnected, crossterm_events_map_to_keys(), errors_mark_disconnected_but_keep_the_last_snapshot() (+17 more)

### Community 11 - "Community 11"
Cohesion: 0.11
Nodes (10): cores(), entry(), event(), LOG, META, NoNetwork, RpcResult, start() (+2 more)

### Community 12 - "Community 12"
Cohesion: 0.10
Nodes (13): connect(), Args, main(), run_interactive(), run_once(), ui_loop(), connect(), DEFAULT_ADDR (+5 more)

### Community 13 - "Community 13"
Cohesion: 0.15
Nodes (24): all_spans(), bias_correction_converges_to_target_coverage(), bias_correction_is_per_user_with_global_fallback(), BiasConfig, Cell, core_bucket(), CORE_BUCKETS, guard_modes_shape_the_spans() (+16 more)

### Community 14 - "Community 14"
Cohesion: 0.25
Nodes (22): JobId, double_start_of_same_job_is_rejected(), early_finish_frees_the_rest_of_the_window(), fetched(), gang_completes_only_when_every_rank_reports(), gang_fails_if_any_rank_fails(), gang_is_released_only_once_every_rank_is_fetched(), gang_journal_replays_to_identical_state() (+14 more)

### Community 15 - "Community 15"
Cohesion: 0.11
Nodes (4): Job, Policy, QuantileEstimator, RuntimeEstimator

### Community 16 - "Community 16"
Cohesion: 0.19
Nodes (27): die(), log(), rec_end(), rec_field(), rec_start(), result(), sc(), lib.sh script (+19 more)

### Community 17 - "Community 17"
Cohesion: 0.11
Nodes (16): Bar, job_info(), JobState, .ALL, Cancelled, Completed, Failed, Pending (+8 more)

### Community 18 - "Community 18"
Cohesion: 0.15
Nodes (7): append_truncate_purge_round_trip_across_reopen(), DiskStore, LogMeta, Membership, read_json(), VOTE, write_json()

### Community 19 - "Community 19"
Cohesion: 0.14
Nodes (12): NodeInfo, apply_event(), finish(), GangRelease, NotRunning, Released, Waiting, place() (+4 more)

### Community 20 - "Community 20"
Cohesion: 0.19
Nodes (12): Walltime, backfill_pass_starts_gang_and_counts_it_once(), BackfillPolicy, cluster(), conservative_backfill_protects_gang_reservation(), estimator_learns_user_ratio_and_respects_walltime(), gang_starts_now_or_reserves_in_both_policy_paths(), heartbeat_does_not_invalidate_plans_in_per_vertex_mode() (+4 more)

### Community 21 - "Community 21"
Cohesion: 0.19
Nodes (10): default_window_is_ten_minutes_back_two_hours_ahead(), durations_parse_with_units(), MAX_SPAN, MIN_SPAN, pan_moves_a_quarter_window_and_zoom_scales_the_pan(), parse_duration_ms(), parse_duration_secs(), reset_restores_the_initial_window() (+2 more)

### Community 22 - "Community 22"
Cohesion: 0.15
Nodes (15): a_finished_job_is_not_started(), await_release(), BARRIER_POLL, BARRIER_TIMEOUT_EXIT, controller_errors_are_retried(), missing_peers_time_out_at_the_limit(), Probe, Release (+7 more)

### Community 23 - "Community 23"
Cohesion: 0.19
Nodes (4): Clock, Applied, apply_at_ignores_the_local_clock(), Fsm

### Community 24 - "Community 24"
Cohesion: 0.13
Nodes (8): Decision, Defer, Reserve, Start, gang_start_now(), Plan, ShadowDiff, ShadowPolicy

### Community 25 - "Community 25"
Cohesion: 0.15
Nodes (11): fingerprint(), fingerprint_includes_node_and_assignment_state(), from_jsonl(), jsonl_parse_errors_report_original_line_number(), JsonlParseError, live_gang_finished_by_rank_reports_is_recovered(), node_fsm(), rebuild() (+3 more)

### Community 26 - "Community 26"
Cohesion: 0.21
Nodes (13): a_store_directory_can_be_opened_by_one_store_at_a_time(), await_leader(), committed_events_survive_restart(), corrupted_middle_log_record_is_an_error(), entries(), fingerprint(), fresh_fsm(), indexes() (+5 more)

### Community 27 - "Community 27"
Cohesion: 0.12
Nodes (7): Assignment, job_nodes_defaults_to_one_for_old_records(), ManualClock, NodeList, now_secs(), SystemClock, zero_nodes_counts_as_one()

### Community 28 - "Community 28"
Cohesion: 0.20
Nodes (3): BiasCorrectedEstimator, SHAPE, MachineShape

### Community 29 - "Community 29"
Cohesion: 0.22
Nodes (11): backfill_pass(), backfill_pass_shows_the_queue_before_clearing_reservations(), FixedSpans, guard_span_blocks_backfill_in_front_of_a_reservation(), machine_shape_reads_the_graph(), PassOptions, PassStats, place_with() (+3 more)

### Community 30 - "Community 30"
Cohesion: 0.18
Nodes (7): declarative_priority_and_limits(), gangs_count_every_node_in_size_and_limits(), PartitionLimit, PolicyConfig, QosTier, wasm_priority_plugin_runs_sandboxed(), WasmPriorityPlugin

### Community 31 - "Community 31"
Cohesion: 0.16
Nodes (6): broker_coalesces_duplicate_evals(), ConflictMode, GlobalIndex, PerVertex, EvalBroker, PlanApplier

### Community 32 - "Community 32"
Cohesion: 0.30
Nodes (12): fill_char(), fill_style(), label_style(), state_color(), states_map_to_fill_and_colour(), draw_bars(), draw_gutter(), render() (+4 more)

### Community 33 - "Community 33"
Cohesion: 0.25
Nodes (5): ConsensusNode, default_config(), propose(), propose_at(), SchedulerRaft

### Community 34 - "Community 34"
Cohesion: 0.21
Nodes (8): Record, FsmImage, StateIndex, EventSink, JournalEntry, VecSink, BrokerInner, Eval

### Community 35 - "Community 35"
Cohesion: 0.23
Nodes (4): DiskSnapBuilder, Meta, SNAPSHOT, StoredSnapshot

### Community 36 - "Community 36"
Cohesion: 0.15
Nodes (13): Event, AssignmentsFetched, Heartbeat, JobCancelled, JobExtended, JobFinished, JobRankFinished, JobReserved (+5 more)

### Community 37 - "Community 37"
Cohesion: 0.27
Nodes (8): header_num(), history_is_the_jobs_before_the_offset(), parse(), parses_header_and_filters_unusable_records(), Trace, TRACE, TraceJob, window_rebases_time_of_day_and_clips_runtime_to_walltime()

### Community 38 - "Community 38"
Cohesion: 0.36
Nodes (11): args(), demo(), main(), open_fsm(), requeue_pending(), restart_with_state_dir_recovers_jobs_nodes_and_reservations(), state_dir(), state_dir_flag_beats_env_and_empty_env_is_ignored() (+3 more)

### Community 39 - "Community 39"
Cohesion: 0.39
Nodes (12): gnomon-agent, gnomon-api, gnomon-cli, gnomon-consensus, gnomon-core, gnomon-ctl, gnomon-fsm, gnomon-graph (+4 more)

### Community 40 - "Community 40"
Cohesion: 0.25
Nodes (3): event_of(), Request, request_of()

### Community 41 - "Community 41"
Cohesion: 0.27
Nodes (4): count_gpus(), detect(), fallback(), main()

### Community 43 - "Community 43"
Cohesion: 0.44
Nodes (4): DiskLogReader, read_range(), SharedLog, C

### Community 44 - "Community 44"
Cohesion: 0.36
Nodes (3): load(), main(), med()

### Community 45 - "Community 45"
Cohesion: 0.29
Nodes (3): container_jobs_run_through_runtime_and_walltime_kills_container(), FAKE, spec()

### Community 46 - "Community 46"
Cohesion: 0.48
Nodes (5): bench_parallel(), bench_serial(), main(), make_fsm(), submit_jobs()

### Community 47 - "Community 47"
Cohesion: 0.53
Nodes (5): slurm-local.sh script, SLURM_CONF, start(), stop(), write_conf()

### Community 48 - "Community 48"
Cohesion: 0.60
Nodes (4): GNOMON_HEARTBEAT_SECS, local-cluster.sh script, start(), stop()

## Knowledge Gaps
- **66 isolated node(s):** `Job`, `Docker`, `Podman`, `Connected`, `Connecting` (+61 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 258 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **5 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Job` connect `Community 15` to `Community 1`, `Community 2`, `Community 3`, `Community 36`, `Community 13`, `Community 14`, `Community 19`, `Community 20`, `Community 24`, `Community 27`, `Community 28`, `Community 29`, `Community 30`, `Community 31`?**
  _High betweenness centrality (0.077) - this node is a cross-community bridge._
- **Are the 21 inferred relationships involving `JobId` (e.g. with `container_image_flows_from_submit_to_job_info_and_fetch_work()` and `fsm_with_node()`) actually correct?**
  _`JobId` has 21 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Job`, `Docker`, `Podman` to the rest of the system?**
  _66 weakly-connected nodes found - possible missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.05369415807560137 - nodes in this community are weakly interconnected._
- **Why does `Resources` connect `Community 2` to `Community 1`, `Community 36`, `Community 6`, `Community 38`, `Community 11`, `Community 15`, `Community 19`, `Community 25`, `Community 27`, `Community 30`?**
  _High betweenness centrality (0.073) - this node is a cross-community bridge._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.05025712949976625 - nodes in this community are weakly interconnected._
- **Why does `Fsm` connect `Community 23` to `Community 1`, `Community 34`, `Community 3`, `Community 5`, `Community 6`, `Community 38`, `Community 42`, `Community 11`, `Community 14`, `Community 46`, `Community 18`, `Community 19`, `Community 20`, `Community 25`, `Community 26`, `Community 29`?**
  _High betweenness centrality (0.066) - this node is a cross-community bridge._