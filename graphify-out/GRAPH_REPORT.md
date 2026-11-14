# Graph Report - gnomon  (2026-11-14)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 1284 nodes · 3575 edges · 56 communities (51 shown, 5 thin omitted)
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 187 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `25036c97`
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
- Community 50
- Community 51
- Community 54

## God Nodes (most connected - your core abstractions)
1. `Fsm` - 75 edges
2. `JobId` - 73 edges
3. `Job` - 71 edges
4. `NodeId` - 50 edges
5. `Resources` - 45 edges
6. `ResourceGraph` - 41 edges
7. `Event` - 39 edges
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

## Communities (56 total, 5 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.06
Nodes (41): Resources, ensure_cluster_root(), backfill_finds_future_window(), capacity_profile_shapes_placement(), CapView, CapView<'a>, commit_revalidation_rejects_conflicts(), earliest_fit_respects_reservations() (+33 more)

### Community 1 - "Community 1"
Cohesion: 0.08
Nodes (24): container_image_flows_from_submit_to_job_info_and_fetch_work(), empty_container_means_none(), fsm_with_node(), gang_job_flows_through_submit_fetch_and_per_rank_completion(), job(), job_info(), job_info_falls_back_to_walltime_and_no_nodes_while_pending(), job_info_reports_planned_span_and_nodes_once_placed() (+16 more)

### Community 2 - "Community 2"
Cohesion: 0.08
Nodes (10): apply_entries(), C, encode_image(), io_err(), restore_image(), SnapBuilder, snapshot_meta(), storage_pair() (+2 more)

### Community 3 - "Community 3"
Cohesion: 0.06
Nodes (30): Args, container_directive_both_spellings(), Directives, directives_combine_per_node_shape(), main(), nodes_flag_parses_count_and_range(), parse_mem_mb(), parse_nodes() (+22 more)

### Community 4 - "Community 4"
Cohesion: 0.10
Nodes (35): a_second_process_cannot_open_a_journal_in_use(), drive(), EventJournal, fingerprint(), JOURNAL, journal_recovers_identical_state_across_restarts(), records_already_in_the_image_are_skipped(), STATE (+27 more)

### Community 5 - "Community 5"
Cohesion: 0.09
Nodes (14): await_leader(), committed_events_survive_restart(), cores(), entry(), event(), fingerprint(), LOG, META (+6 more)

### Community 6 - "Community 6"
Cohesion: 0.14
Nodes (16): a_store_directory_can_be_opened_by_one_store_at_a_time(), append_truncate_purge_round_trip_across_reopen(), corrupted_middle_log_record_is_an_error(), DiskStore, entries(), fresh_fsm(), indexes(), installed_snapshot_is_persisted() (+8 more)

### Community 7 - "Community 7"
Cohesion: 0.12
Nodes (25): backfill_pass(), backfill_pass_shows_the_queue_before_clearing_reservations(), backfill_pass_starts_gang_and_counts_it_once(), BackfillPolicy, cluster(), ConflictMode, GlobalIndex, PerVertex (+17 more)

### Community 8 - "Community 8"
Cohesion: 0.10
Nodes (21): CONTROLLERS, CPU_PERIOD_US, create_writes_limit_files_under_job_path(), DEFAULT_ROOT, detect_enables_controllers_and_creates_base(), detect_rejects_non_cgroup2_root(), detect_reports_unwritable_root(), detect_requires_cpu_and_memory() (+13 more)

### Community 9 - "Community 9"
Cohesion: 0.13
Nodes (19): assignment(), cgroup_setup_failure_fails_the_job_instead_of_running_unconfined(), confined_job_joins_cgroup_and_cleans_up(), current_user(), env_of(), KILL_GRACE, Launcher, launcher() (+11 more)

### Community 10 - "Community 10"
Cohesion: 0.12
Nodes (23): bars_land_on_their_time_columns_with_labels(), buffer_text(), disconnected_with_data_keeps_the_stale_plan(), disconnected_without_data_says_retrying(), draw(), filter_prompt_replaces_the_key_help(), fixture(), header_shows_counts_index_and_window() (+15 more)

### Community 11 - "Community 11"
Cohesion: 0.12
Nodes (25): bar_label(), durations_format_compactly(), fmt_dur(), fmt_rel(), gantt_lanes(), gutter_shows_allocation_and_liveness(), job(), labels_truncate_with_ellipsis() (+17 more)

### Community 12 - "Community 12"
Cohesion: 0.10
Nodes (5): Job, estimator_learns_user_ratio_and_respects_walltime(), Policy, QuantileEstimator, RuntimeEstimator

### Community 13 - "Community 13"
Cohesion: 0.14
Nodes (30): Args, Baseline, cluster(), DEFAULT_BASELINE, gang_workload_marks_every_seventh_job(), identical_measures_do_not_regress(), improvements_and_lower_utilization_are_judged_by_direction(), m() (+22 more)

### Community 14 - "Community 14"
Cohesion: 0.09
Nodes (19): Bar, name_width(), ClusterSnapshot, job_info(), JobState, .ALL, Cancelled, Completed (+11 more)

### Community 15 - "Community 15"
Cohesion: 0.15
Nodes (24): all_spans(), bias_correction_converges_to_target_coverage(), bias_correction_is_per_user_with_global_fallback(), BiasConfig, Cell, core_bucket(), CORE_BUCKETS, guard_modes_shape_the_spans() (+16 more)

### Community 16 - "Community 16"
Cohesion: 0.17
Nodes (24): bounded_run_reports_status_and_times_out(), container_name(), container_user(), ContainerRuntime, detect_docker_rootless(), docker_argv_is_complete_and_ordered(), gang_ranks_get_distinct_container_names(), has_pair() (+16 more)

### Community 17 - "Community 17"
Cohesion: 0.11
Nodes (21): app(), Conn, Connected, Connecting, Disconnected, crossterm_events_map_to_keys(), errors_mark_disconnected_but_keep_the_last_snapshot(), filter_limits_visible_jobs_by_user() (+13 more)

### Community 18 - "Community 18"
Cohesion: 0.19
Nodes (27): die(), log(), rec_end(), rec_field(), rec_start(), result(), sc(), lib.sh script (+19 more)

### Community 19 - "Community 19"
Cohesion: 0.31
Nodes (20): JobId, double_start_of_same_job_is_rejected(), early_finish_frees_the_rest_of_the_window(), gang_completes_only_when_every_rank_reports(), gang_fails_if_any_rank_fails(), gang_journal_replays_to_identical_state(), gang_placement_with_wrong_vertex_count_or_duplicates_is_rejected(), gang_setup() (+12 more)

### Community 20 - "Community 20"
Cohesion: 0.23
Nodes (14): bias_corrected_never_oversubscribes_even_when_badly_wrong(), capacity_profile_is_honoured_by_aware_planner(), cluster(), conservative_backfill_does_not_starve_a_full_machine_gang(), journal_replay_is_exact_and_counterfactual_reproduces_history(), Machine, mixed_gang_workload_never_oversubscribes_and_gangs_start_together(), mixed_workload() (+6 more)

### Community 21 - "Community 21"
Cohesion: 0.19
Nodes (10): default_window_is_ten_minutes_back_two_hours_ahead(), durations_parse_with_units(), MAX_SPAN, MIN_SPAN, pan_moves_a_quarter_window_and_zoom_scales_the_pan(), parse_duration_ms(), parse_duration_secs(), reset_restores_the_initial_window() (+2 more)

### Community 22 - "Community 22"
Cohesion: 0.13
Nodes (9): bias_policy(), bias_variants(), CARBON, cores(), main(), Oracle, quantile_policy(), WINDOW_SECS (+1 more)

### Community 23 - "Community 23"
Cohesion: 0.17
Nodes (10): connect(), Update, Error, connect(), DEFAULT_ADDR, error_text(), fetch(), now_secs() (+2 more)

### Community 24 - "Community 24"
Cohesion: 0.18
Nodes (4): Clock, Applied, apply_at_ignores_the_local_clock(), Fsm

### Community 25 - "Community 25"
Cohesion: 0.12
Nodes (8): Record, FsmImage, StateIndex, EventSink, VecSink, broker_coalesces_duplicate_evals(), BrokerInner, Eval

### Community 26 - "Community 26"
Cohesion: 0.20
Nodes (3): BiasCorrectedEstimator, SHAPE, MachineShape

### Community 27 - "Community 27"
Cohesion: 0.27
Nodes (15): App, fill_char(), fill_style(), gutter_text(), label_style(), state_color(), states_map_to_fill_and_colour(), draw_bars() (+7 more)

### Community 28 - "Community 28"
Cohesion: 0.18
Nodes (12): JournalEntry, fingerprint(), fingerprint_includes_node_and_assignment_state(), from_jsonl(), jsonl_parse_errors_report_original_line_number(), JsonlParseError, live_gang_finished_by_rank_reports_is_recovered(), node_fsm() (+4 more)

### Community 29 - "Community 29"
Cohesion: 0.19
Nodes (8): Cgroups, enable_controllers(), Unavailable, Disabled, MissingController, NotCgroup2, NotWritable, Usage

### Community 30 - "Community 30"
Cohesion: 0.21
Nodes (11): CapacityProfile, Outcome, SimResult, energy(), EnergySummary, gang_jobs_count_every_node_in_wide_and_core_hour_metrics(), outcome(), pct() (+3 more)

### Community 31 - "Community 31"
Cohesion: 0.21
Nodes (7): NodeInfo, apply_event(), finish(), place(), rank_finished(), Snapshot, State

### Community 32 - "Community 32"
Cohesion: 0.20
Nodes (8): header_num(), history_is_the_jobs_before_the_offset(), parse(), parses_header_and_filters_unusable_records(), Trace, TRACE, TraceJob, window_rebases_time_of_day_and_clips_runtime_to_walltime()

### Community 33 - "Community 33"
Cohesion: 0.17
Nodes (5): Args, main(), run_interactive(), run_once(), ui_loop()

### Community 34 - "Community 34"
Cohesion: 0.25
Nodes (5): ConsensusNode, default_config(), propose(), propose_at(), SchedulerRaft

### Community 35 - "Community 35"
Cohesion: 0.19
Nodes (7): Assignment, job_nodes_defaults_to_one_for_old_records(), NodeList, now_secs(), SystemClock, Walltime, zero_nodes_counts_as_one()

### Community 36 - "Community 36"
Cohesion: 0.17
Nodes (7): Decision, Defer, Reserve, Start, Plan, ShadowDiff, ShadowPolicy

### Community 37 - "Community 37"
Cohesion: 0.16
Nodes (3): run(), setup(), EvalBroker

### Community 38 - "Community 38"
Cohesion: 0.22
Nodes (7): container_jobs_run_through_runtime_and_walltime_kills_container(), FAKE, spec(), exit_code_is_reported(), launcher(), spec(), walltime_kills_long_job_with_exit_124()

### Community 39 - "Community 39"
Cohesion: 0.29
Nodes (11): args(), demo(), main(), open_fsm(), requeue_pending(), restart_with_state_dir_recovers_jobs_nodes_and_reservations(), state_dir(), state_dir_flag_beats_env_and_empty_env_is_ignored() (+3 more)

### Community 40 - "Community 40"
Cohesion: 0.15
Nodes (13): Event, AssignmentsFetched, Heartbeat, JobCancelled, JobExtended, JobFinished, JobRankFinished, JobReserved (+5 more)

### Community 41 - "Community 41"
Cohesion: 0.26
Nodes (4): DiskSnapBuilder, Meta, SNAPSHOT, StoredSnapshot

### Community 42 - "Community 42"
Cohesion: 0.39
Nodes (12): gnomon-agent, gnomon-api, gnomon-cli, gnomon-consensus, gnomon-core, gnomon-ctl, gnomon-fsm, gnomon-graph (+4 more)

### Community 43 - "Community 43"
Cohesion: 0.25
Nodes (3): event_of(), Request, request_of()

### Community 44 - "Community 44"
Cohesion: 0.36
Nodes (3): load(), main(), med()

### Community 45 - "Community 45"
Cohesion: 0.36
Nodes (4): count_gpus(), detect(), fallback(), main()

### Community 46 - "Community 46"
Cohesion: 0.43
Nodes (3): DiskLogReader, read_range(), SharedLog

### Community 47 - "Community 47"
Cohesion: 0.39
Nodes (5): bench_parallel(), bench_serial(), main(), make_fsm(), submit_jobs()

### Community 48 - "Community 48"
Cohesion: 0.53
Nodes (4): bad_durations_are_rejected_by_the_parser(), gntop(), once_honours_window_and_user_flags(), once_prints_a_plain_disconnected_frame_and_fails()

### Community 49 - "Community 49"
Cohesion: 0.53
Nodes (5): slurm-local.sh script, SLURM_CONF, start(), stop(), write_conf()

### Community 51 - "Community 51"
Cohesion: 0.60
Nodes (4): GNOMON_HEARTBEAT_SECS, local-cluster.sh script, start(), stop()

## Knowledge Gaps
- **59 isolated node(s):** `Cluster`, `Core`, `Gpu`, `Node`, `Rack` (+54 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 236 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **5 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Job` connect `Community 12` to `Community 0`, `Community 1`, `Community 35`, `Community 3`, `Community 36`, `Community 7`, `Community 40`, `Community 15`, `Community 19`, `Community 20`, `Community 22`, `Community 25`, `Community 26`, `Community 31`?**
  _High betweenness centrality (0.089) - this node is a cross-community bridge._
- **Are the 20 inferred relationships involving `JobId` (e.g. with `container_image_flows_from_submit_to_job_info_and_fetch_work()` and `fsm_with_node()`) actually correct?**
  _`JobId` has 20 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Cluster`, `Core`, `Gpu` to the rest of the system?**
  _59 weakly-connected nodes found - possible missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.05820476858345021 - nodes in this community are weakly interconnected._
- **Why does `Fsm` connect `Community 24` to `Community 1`, `Community 2`, `Community 4`, `Community 5`, `Community 6`, `Community 39`, `Community 37`, `Community 7`, `Community 47`, `Community 19`, `Community 20`, `Community 25`, `Community 28`, `Community 31`?**
  _High betweenness centrality (0.084) - this node is a cross-community bridge._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.08208020050125313 - nodes in this community are weakly interconnected._
- **Why does `Resources` connect `Community 0` to `Community 35`, `Community 4`, `Community 5`, `Community 3`, `Community 39`, `Community 40`, `Community 9`, `Community 12`, `Community 20`, `Community 22`, `Community 28`, `Community 30`, `Community 31`?**
  _High betweenness centrality (0.084) - this node is a cross-community bridge._