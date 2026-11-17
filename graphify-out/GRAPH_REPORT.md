# Graph Report - gnomon  (2026-11-17)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 1704 nodes · 4947 edges · 75 communities (67 shown, 8 thin omitted)
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 234 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `eaae23d2`
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
- Community 52
- Community 53
- Community 54
- Community 55
- Community 56
- Community 57
- Community 58
- Community 59
- Community 60
- Community 61
- Community 62
- Community 63
- Community 64
- Community 65
- Community 66
- Community 67
- Community 68
- Community 69
- Community 71
- Community 72
- Community 73

## God Nodes (most connected - your core abstractions)
1. `JobId` - 109 edges
2. `Fsm` - 97 edges
3. `Job` - 81 edges
4. `NodeId` - 65 edges
5. `Event` - 52 edges
6. `ResourceGraph` - 51 edges
7. `Resources` - 45 edges
8. `VertexId` - 42 edges
9. `DiskStore` - 38 edges
10. `State` - 31 edges

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

## Communities (75 total, 8 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.05
Nodes (44): CapacityProfile, Resources, ensure_cluster_root(), backfill_finds_future_window(), capacity_profile_shapes_placement(), CapView, CapView<'a>, commit_revalidation_rejects_conflicts() (+36 more)

### Community 1 - "Community 1"
Cohesion: 0.07
Nodes (32): cancel(), cancelling_a_running_job_kills_it_and_ends_cancelled(), container_image_flows_from_submit_to_job_info_and_fetch_work(), empty_container_means_none(), failed_rank_kills_its_peers_through_fetch_work(), fetch(), fsm_with_node(), gang_job_flows_through_submit_fetch_and_per_rank_completion() (+24 more)

### Community 2 - "Community 2"
Cohesion: 0.07
Nodes (29): Cgroups, CONTROLLERS, CPU_PERIOD_US, create_writes_limit_files_under_job_path(), DEFAULT_ROOT, detect_enables_controllers_and_creates_base(), detect_rejects_non_cgroup2_root(), detect_reports_unwritable_root() (+21 more)

### Community 3 - "Community 3"
Cohesion: 0.09
Nodes (9): apply_entries(), encode_image(), io_err(), restore_image(), SnapBuilder, snapshot_meta(), storage_pair(), Store (+1 more)

### Community 4 - "Community 4"
Cohesion: 0.09
Nodes (34): a_second_process_cannot_open_a_journal_in_use(), drive(), EventJournal, fingerprint(), JOURNAL, journal_recovers_identical_state_across_restarts(), records_already_in_the_image_are_skipped(), STATE (+26 more)

### Community 5 - "Community 5"
Cohesion: 0.08
Nodes (31): Peers, Replica, clock(), Cluster, fingerprint(), node(), NODES, replicas_converge_forward_writes_and_survive_leader_failover() (+23 more)

### Community 6 - "Community 6"
Cohesion: 0.12
Nodes (34): bounded_output(), bounded_output_reads_large_output_and_rejects_failures(), bounded_output_times_out_when_the_child_or_a_descendant_hangs(), bounded_run_reports_status_and_times_out(), container_name(), container_user(), ContainerRuntime, ContainerUser (+26 more)

### Community 7 - "Community 7"
Cohesion: 0.09
Nodes (24): Args, container_directive_both_spellings(), Directives, directives_combine_per_node_shape(), main(), nodes_flag_parses_count_and_range(), parse_mem_mb(), parse_nodes() (+16 more)

### Community 8 - "Community 8"
Cohesion: 0.12
Nodes (17): a_store_directory_can_be_opened_by_one_store_at_a_time(), append_truncate_purge_round_trip_across_reopen(), corrupted_middle_log_record_is_an_error(), DiskStore, entries(), fingerprint(), fresh_fsm(), indexes() (+9 more)

### Community 9 - "Community 9"
Cohesion: 0.20
Nodes (37): a_fetch_racing_with_the_loss_claims_nothing(), a_loss_proposal_older_than_the_last_heartbeat_is_rejected(), a_returning_node_is_placeable_and_its_late_report_changes_nothing(), a_second_submission_with_a_taken_id_is_rejected(), cancelling_a_running_gang_kills_every_rank_and_ends_cancelled(), cancelling_a_running_single_node_job_kills_it(), double_start_of_same_job_is_rejected(), early_finish_frees_the_rest_of_the_window() (+29 more)

### Community 10 - "Community 10"
Cohesion: 0.11
Nodes (31): bar_label(), fill_char(), fill_style(), gantt_lanes(), gutter_shows_allocation_and_liveness(), gutter_text(), job(), label_style() (+23 more)

### Community 11 - "Community 11"
Cohesion: 0.09
Nodes (13): await_leader(), committed_events_survive_restart(), cores(), entry(), event(), LOG, META, NoNetwork (+5 more)

### Community 12 - "Community 12"
Cohesion: 0.16
Nodes (37): die(), log(), rank_fields(), rec_end(), rec_field(), rec_host(), rec_start(), result() (+29 more)

### Community 13 - "Community 13"
Cohesion: 0.11
Nodes (23): bars_land_on_their_time_columns_with_labels(), buffer_text(), disconnected_with_data_keeps_the_stale_plan(), disconnected_without_data_says_retrying(), draw(), filter_prompt_replaces_the_key_help(), fixture(), header_shows_counts_index_and_window() (+15 more)

### Community 14 - "Community 14"
Cohesion: 0.13
Nodes (18): Assignment, JobId, NodeInfo, apply_event(), cancel_running(), clear_lost(), finish(), kill_ranks() (+10 more)

### Community 15 - "Community 15"
Cohesion: 0.09
Nodes (19): BiasConfig, GuardMode, Block, Record, NarrowAhead, Cautious, Planned, Skip (+11 more)

### Community 16 - "Community 16"
Cohesion: 0.11
Nodes (15): Update, Error, Args, main(), run_interactive(), run_once(), ui_loop(), ClusterSnapshot (+7 more)

### Community 17 - "Community 17"
Cohesion: 0.09
Nodes (14): job_nodes_defaults_to_one_for_old_records(), NODE_LOST_EXIT, NodeList, now_secs(), SystemClock, Walltime, zero_nodes_counts_as_one(), declarative_priority_and_limits() (+6 more)

### Community 18 - "Community 18"
Cohesion: 0.15
Nodes (23): ManualClock, CORE_BUCKETS, finish(), machine(), MIN_RATIO, narrow_ahead_mode_decides_where_a_new_slot_goes(), pass(), random_run() (+15 more)

### Community 19 - "Community 19"
Cohesion: 0.14
Nodes (30): Args, Baseline, cluster(), DEFAULT_BASELINE, gang_workload_marks_every_seventh_job(), identical_measures_do_not_regress(), improvements_and_lower_utilization_are_judged_by_direction(), m() (+22 more)

### Community 20 - "Community 20"
Cohesion: 0.10
Nodes (14): SchedulerRaft, serve(), WriteResult, block_on(), is_leader(), parse_peers(), pause(), peers_parse_and_reject_malformed_lists() (+6 more)

### Community 21 - "Community 21"
Cohesion: 0.10
Nodes (17): Event, AssignmentsFetched, Heartbeat, JobCancelled, JobExtended, JobFinished, JobRankFinished, JobReserved (+9 more)

### Community 22 - "Community 22"
Cohesion: 0.09
Nodes (16): Bar, job_info(), JobState, .ALL, Cancelled, Completed, Failed, Pending (+8 more)

### Community 23 - "Community 23"
Cohesion: 0.18
Nodes (7): Job, book(), release(), SHAPE, Shield, Slot, MachineShape

### Community 24 - "Community 24"
Cohesion: 0.11
Nodes (21): app(), Conn, Connected, Connecting, Disconnected, crossterm_events_map_to_keys(), errors_mark_disconnected_but_keep_the_last_snapshot(), filter_limits_visible_jobs_by_user() (+13 more)

### Community 25 - "Community 25"
Cohesion: 0.17
Nodes (9): an_uncommitted_write_is_an_error_not_a_state_change(), apply_at_ignores_the_local_clock(), Fsm, gang_is_released_only_once_every_rank_is_fetched(), LoopbackProposer, Proposal, Proposer, sink_sees_every_event_with_its_resulting_index() (+1 more)

### Community 26 - "Community 26"
Cohesion: 0.18
Nodes (19): is_privileged(), assignment(), cgroup_setup_failure_fails_the_job_instead_of_running_unconfined(), confined_job_joins_cgroup_and_cleans_up(), current_user(), env_of(), jobs_run_as_the_submitting_user_when_the_agent_is_root(), KILL_GRACE (+11 more)

### Community 27 - "Community 27"
Cohesion: 0.27
Nodes (21): alive(), cgroup_cpu_limit_caps_a_multithreaded_job_at_its_cores(), cgroup_job_starts_inside_its_cgroup_with_the_requested_limits(), cgroup_kill_reaches_processes_that_left_the_process_group(), cgroup_memory_limit_oom_kills_the_job_not_the_host(), cgroup_walltime_kill_reports_124(), confined(), container_cpu_and_memory_limits_are_applied() (+13 more)

### Community 28 - "Community 28"
Cohesion: 0.18
Nodes (10): default_window_is_ten_minutes_back_two_hours_ahead(), durations_parse_with_units(), MAX_SPAN, MIN_SPAN, pan_moves_a_quarter_window_and_zoom_scales_the_pan(), parse_duration_ms(), parse_duration_secs(), reset_restores_the_initial_window() (+2 more)

### Community 29 - "Community 29"
Cohesion: 0.16
Nodes (15): a_gang_ended_by_a_loss_runs_until_its_latest_rank_or_the_loss(), a_lost_job_ends_at_the_loss_and_its_late_report_is_ignored(), a_requeued_job_is_measured_from_its_rerun(), fingerprint(), fingerprint_includes_node_and_assignment_state(), from_jsonl(), jsonl_parse_errors_report_original_line_number(), JsonlParseError (+7 more)

### Community 30 - "Community 30"
Cohesion: 0.18
Nodes (10): frames_round_trip_and_oversized_lengths_are_refused(), handle(), read_frame(), Reply, Append, Snapshot, Vote, Write (+2 more)

### Community 31 - "Community 31"
Cohesion: 0.28
Nodes (15): bias_corrected_never_oversubscribes_even_when_badly_wrong(), capacity_profile_is_honoured_by_aware_planner(), cluster(), conservative_backfill_does_not_starve_a_full_machine_gang(), journal_replay_is_exact_and_counterfactual_reproduces_history(), Machine, mixed_gang_workload_never_oversubscribes_and_gangs_start_together(), mixed_workload() (+7 more)

### Community 32 - "Community 32"
Cohesion: 0.15
Nodes (15): a_finished_job_is_not_started(), await_release(), BARRIER_POLL, BARRIER_TIMEOUT_EXIT, controller_errors_are_retried(), missing_peers_time_out_at_the_limit(), Probe, Release (+7 more)

### Community 33 - "Community 33"
Cohesion: 0.26
Nodes (10): End, Exited, Killed, TimedOut, failed(), killed(), Launcher, LaunchSpec (+2 more)

### Community 34 - "Community 34"
Cohesion: 0.26
Nodes (16): all(), build(), build_linux(), die(), down(), entry(), gnomon_up(), log() (+8 more)

### Community 35 - "Community 35"
Cohesion: 0.16
Nodes (8): Walltime, run(), setup(), ConflictMode, GlobalIndex, PerVertex, optimistic_commit_detects_conflict(), PlanApplier

### Community 36 - "Community 36"
Cohesion: 0.19
Nodes (9): address_lists_are_split_and_trimmed(), an_unreachable_list_is_an_error_naming_every_controller(), Client, connect(), Controllers, Current, failures_rotate_once_per_generation(), parse_addrs() (+1 more)

### Community 37 - "Community 37"
Cohesion: 0.13
Nodes (5): broker_coalesces_duplicate_evals(), BrokerInner, Eval, EvalBroker, ShadowPolicy

### Community 38 - "Community 38"
Cohesion: 0.21
Nodes (6): kill_all_signals_every_tracked_rank_once(), KillPlan, LocalRanks, tracked_ranks_are_signalled_once_and_untracked_jobs_are_reported(), tracking_twice_shares_one_flag(), untracked_ranks_are_reported_after_they_finish()

### Community 39 - "Community 39"
Cohesion: 0.15
Nodes (4): an_unreachable_peer_is_reported_as_never_sent(), MAX_FRAME, Peer, TcpNetwork

### Community 40 - "Community 40"
Cohesion: 0.21
Nodes (12): an_ordinary_account_resolves_with_its_primary_group(), drop_privileges(), group_count(), group_count_rejects_values_that_do_not_fit_in_c_int(), group_list(), JobIdentity, lookup(), passwd_entry() (+4 more)

### Community 41 - "Community 41"
Cohesion: 0.17
Nodes (9): count_gpus(), detect(), drain(), fallback(), main(), report(), REPORT_ATTEMPTS, SHUTDOWN_GRACE (+1 more)

### Community 42 - "Community 42"
Cohesion: 0.30
Nodes (11): backfill_pass(), backfill_pass_shows_the_queue_before_clearing_reservations(), backfill_pass_starts_gang_and_counts_it_once(), cluster(), conservative_backfill_protects_gang_reservation(), gang_starts_now_or_reserves_in_both_policy_paths(), heartbeat_does_not_invalidate_plans_in_per_vertex_mode(), PassOptions (+3 more)

### Community 43 - "Community 43"
Cohesion: 0.24
Nodes (4): ConsensusNode, default_config(), propose(), propose_at()

### Community 44 - "Community 44"
Cohesion: 0.36
Nodes (10): all_spans(), bias_correction_converges_to_target_coverage(), bias_correction_is_per_user_with_global_fallback(), guard_modes_shape_the_spans(), job(), plans_walltime_without_history(), rng(), slipped_wide_reservation_turns_on_protection_until_it_starts() (+2 more)

### Community 45 - "Community 45"
Cohesion: 0.23
Nodes (7): Cell, core_bucket(), log2_bucket(), log_ratio(), State, UserCells, wall_bucket()

### Community 46 - "Community 46"
Cohesion: 0.37
Nodes (11): App, durations_format_compactly(), fmt_dur(), fmt_rel(), draw_gutter(), render(), render_footer(), render_gantt() (+3 more)

### Community 47 - "Community 47"
Cohesion: 0.29
Nodes (7): FixedSpans, guard_span_blocks_backfill_in_front_of_a_reservation(), machine_shape_reads_the_graph(), place_with(), record_span_is_used_only_when_it_fits(), reserved_node(), Spans

### Community 48 - "Community 48"
Cohesion: 0.31
Nodes (10): Outcome, SimResult, energy(), EnergySummary, gang_jobs_count_every_node_in_wide_and_core_hour_metrics(), outcome(), pct(), result() (+2 more)

### Community 49 - "Community 49"
Cohesion: 0.27
Nodes (8): header_num(), history_is_the_jobs_before_the_offset(), parse(), parses_header_and_filters_unusable_records(), Trace, TRACE, TraceJob, window_rebases_time_of_day_and_clips_runtime_to_walltime()

### Community 50 - "Community 50"
Cohesion: 0.26
Nodes (4): DiskSnapBuilder, Meta, SNAPSHOT, StoredSnapshot

### Community 51 - "Community 51"
Cohesion: 0.26
Nodes (7): Record, FsmImage, StateIndex, EventSink, JournalEntry, VecSink, to_jsonl()

### Community 52 - "Community 52"
Cohesion: 0.29
Nodes (4): CallError, Network, Unreachable, mismatched()

### Community 53 - "Community 53"
Cohesion: 0.21
Nodes (6): Decision, Defer, Reserve, Start, Plan, ShadowDiff

### Community 54 - "Community 54"
Cohesion: 0.38
Nodes (12): gnomon-agent, gnomon-api, gnomon-cli, gnomon-consensus, gnomon-core, gnomon-ctl, gnomon-fsm, gnomon-graph (+4 more)

### Community 56 - "Community 56"
Cohesion: 0.24
Nodes (3): BackfillPolicy, estimator_learns_user_ratio_and_respects_walltime(), shadow_records_disagreements_without_acting()

### Community 57 - "Community 57"
Cohesion: 0.44
Nodes (8): a_rank_killed_before_launch_never_runs(), alive(), exit_code_is_reported(), kill_flag_stops_a_running_job_with_exit_143(), launcher(), spec(), unconfined_kill_reaches_grandchildren(), walltime_kills_long_job_with_exit_124()

### Community 58 - "Community 58"
Cohesion: 0.38
Nodes (4): DiskLogReader, read_range(), SharedLog, C

### Community 59 - "Community 59"
Cohesion: 0.27
Nodes (3): event_of(), request_of(), Response

### Community 60 - "Community 60"
Cohesion: 0.36
Nodes (3): load(), main(), med()

### Community 61 - "Community 61"
Cohesion: 0.29
Nodes (3): container_jobs_run_through_runtime_and_walltime_kills_container(), FAKE, spec()

### Community 63 - "Community 63"
Cohesion: 0.29
Nodes (5): Rpc, Append, Snapshot, Vote, Write

### Community 64 - "Community 64"
Cohesion: 0.48
Nodes (5): bench_parallel(), bench_serial(), main(), make_fsm(), submit_jobs()

### Community 65 - "Community 65"
Cohesion: 0.48
Nodes (6): gninfo_within(), GNOMON_HEARTBEAT_SECS, registered(), local-cluster.sh script, start(), stop()

### Community 66 - "Community 66"
Cohesion: 0.53
Nodes (4): bad_durations_are_rejected_by_the_parser(), gntop(), once_honours_window_and_user_flags(), once_prints_a_plain_disconnected_frame_and_fails()

### Community 67 - "Community 67"
Cohesion: 0.53
Nodes (5): slurm-local.sh script, SLURM_CONF, start(), stop(), write_conf()

### Community 69 - "Community 69"
Cohesion: 0.50
Nodes (4): GangRelease, NotRunning, Released, Waiting

## Knowledge Gaps
- **78 isolated node(s):** `Cluster`, `Core`, `Gpu`, `Node`, `Rack` (+73 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 328 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **8 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Fsm` connect `Community 25` to `Community 64`, `Community 1`, `Community 3`, `Community 4`, `Community 5`, `Community 35`, `Community 8`, `Community 9`, `Community 42`, `Community 11`, `Community 14`, `Community 47`, `Community 18`, `Community 51`, `Community 20`, `Community 21`, `Community 29`, `Community 31`?**
  _High betweenness centrality (0.089) - this node is a cross-community bridge._
- **Are the 29 inferred relationships involving `JobId` (e.g. with `container_image_flows_from_submit_to_job_info_and_fetch_work()` and `fsm_with_node()`) actually correct?**
  _`JobId` has 29 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Cluster`, `Core`, `Gpu` to the rest of the system?**
  _78 weakly-connected nodes found - possible missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.05454545454545454 - nodes in this community are weakly interconnected._
- **Why does `JobId` connect `Community 14` to `Community 1`, `Community 4`, `Community 5`, `Community 9`, `Community 11`, `Community 15`, `Community 17`, `Community 18`, `Community 19`, `Community 21`, `Community 23`, `Community 25`, `Community 29`, `Community 31`, `Community 35`, `Community 37`, `Community 42`, `Community 44`, `Community 47`, `Community 48`, `Community 49`, `Community 53`, `Community 55`, `Community 56`?**
  _High betweenness centrality (0.057) - this node is a cross-community bridge._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.06886338393187708 - nodes in this community are weakly interconnected._
- **Why does `Resources` connect `Community 0` to `Community 11`, `Community 14`, `Community 15`, `Community 17`, `Community 18`, `Community 21`, `Community 23`, `Community 31`?**
  _High betweenness centrality (0.051) - this node is a cross-community bridge._