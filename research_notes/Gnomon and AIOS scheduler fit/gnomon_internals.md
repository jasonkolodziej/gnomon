# Gnomon internals: technical profile for AIOS-scheduler fit analysis

Scope and method: local repository `/home/user/gnomon` only (HEAD `7b00379`, 10 commits, workspace version 0.5.0), read on 2026-11-20. The graphify knowledge graph (1,991 nodes, built at `1f400e6`, one graph-refresh commit behind HEAD) was used to locate symbols; all citations below are to repository files with line numbers, paths relative to the repo root. No web sources were used. "Inference" items are my reading, not statements in the repo.

## 1. What kind of scheduler is Gnomon, and what units does it schedule?

### Takeaway
Gnomon is a self-described "Rust-native HPC scheduler" and research codebase: a Slurm-style batch cluster workload manager (controller + node agents + Slurm-compatible CLI) that places batch **jobs** (shell commands or container images, with cores/memory/GPUs, a walltime and an optional multi-node gang size) onto **nodes** along an explicit time axis. It is not a task queue, an in-process thread scheduler or an LLM request scheduler.

### Cited Findings
- README headline: "A Rust-native HPC scheduler"; "A research codebase implementing the Rust-native HPC scheduler architecture proposed in docs/SLURM_Scheduler_Research_and_Rust_Design.md" — [README.md:16, 29-30](../../README.md)
- Core idea stated in the architecture doc: "scheduling is managing the time axis. Every schedulable vertex of the resource graph carries a planner of running spans and future reservations, the scheduler plans against immutable snapshots without holding locks, and a single applier commits plans with per-vertex optimistic concurrency." — [docs/ARCHITECTURE.md:8-11](../../docs/ARCHITECTURE.md)
- Workspace has 12 crates, version 0.5.0, MIT, edition 2021: core, graph, fsm, sched, policy, consensus, api, ctl, agent, cli, top, sim — [Cargo.toml:1-22](../../Cargo.toml); roles table at [README.md:39-52](../../README.md)
- Crate dependency layering: graph→core; fsm→core,graph; sched→core,fsm,graph; policy→core,graph,fsm,sched; consensus→core,fsm; api→core,fsm,sched; ctl→everything; agent→core,api; cli/top→api; sim→core,graph,fsm,sched,policy — [docs/ARCHITECTURE.md:55-80](../../docs/ARCHITECTURE.md)
- The scheduled unit is `Job`: id, name, user, partition, state, `request: Resources`, walltime, scalar `priority: u64`, submit_time, planned_start/planned_dur, `command: String` (a command line the agent executes), optional `container` image, `nodes` (gang size; `request` is per node) — [crates/core/src/lib.rs:135-177](../../crates/core/src/lib.rs)
- Vocabulary "deliberately mirrors Slurm (sbatch-style jobs, partitions, walltime)" — [crates/core/src/lib.rs:3-5](../../crates/core/src/lib.rs)
- Job states: Pending, Reserved (backfilled, start in the future), Running, Completed, Cancelled, Failed — [crates/core/src/lib.rs:21-32](../../crates/core/src/lib.rs); state diagram at [docs/ARCHITECTURE.md:198-216](../../docs/ARCHITECTURE.md)
- All state changes are `Event`s applied by a deterministic FSM (JobSubmitted, JobStarted, JobReserved, JobFinished, JobRankFinished, JobExtended, ReservationsCleared, NodeRegistered, AssignmentsFetched, NodeProfileSet, Heartbeat, NodeLost) — [crates/core/src/lib.rs:216-310](../../crates/core/src/lib.rs)
- Deployment: `gnomon-ctl` controller(s), `gnomon-agent` on each compute node, `gnbatch/gnqueue/gncancel/gninfo` CLI shims and `gntop` TUI; all traffic is pull-based ("agents poll FetchWork every heartbeat interval; the controller never connects to nodes") — [docs/ARCHITECTURE.md:13-51](../../docs/ARCHITECTURE.md)
- Jobs execute as `sh -c <command>` in the agent's working directory, in a cgroup v2 (`cpu.max`, `memory.max`) or under rootless Docker/Podman, with privilege drop to the submitting user — [README.md:179-262](../../README.md)
- Time unit is whole seconds: `now_secs()` returns Unix seconds; walltime on submit is clamped to at least 1 s — [crates/core/src/lib.rs:348-353](../../crates/core/src/lib.rs), [crates/api/src/lib.rs:133](../../crates/api/src/lib.rs)
- Stated lineage: Slurm (CLI/vocabulary), Fluxion (resource graph + planner), Omega/Nomad (optimistic evaluations + plan applier), Flux (nested instances), Spur (Rust + Raft baseline) — [docs/SLURM_Scheduler_Research_and_Rust_Design.md:126-142, 187-200](../../docs/SLURM_Scheduler_Research_and_Rust_Design.md)

### Inferences
- The unit of work is an OS-level process tree on a compute node with a walltime measured in seconds; there is no notion of an in-process call, request, tool invocation or "syscall". An AIOS LLM-syscall scheduler operates at a much finer granularity (milliseconds, in-process) than Gnomon's job model.
- The name's second meaning ("nested instances: a delegated subgraph that is itself a full scheduler", README.md:21-24) suggests the intended scaling route for very many small tasks is hierarchical delegation, not shrinking the job abstraction.

### Gaps
- No explicit "non-goals" section exists in README or docs; the only "out of scope" statement found is in ADR 0002 (running user jobs as WASM on the agent) — [.github/plans/0002-wasm-policy-plugins.md:120-123](../../.github/plans/0002-wasm-policy-plugins.md).

## 2. What scheduling policies/algorithms exist, and how pluggable are they?

### Takeaway
One production policy family: priority-ordered conservative backfill with earliest-fit placement on per-node time planners, plus all-or-nothing multi-node gang placement, optional learned runtime prediction (quantile and bias-corrected estimators) and a "wide-job shield". Pluggability is through the Rust `Policy` and `RuntimeEstimator` traits (compile-time) and four WASM plugin hooks (admission, priority, runtime estimate, node score). There is no preemption, no time-slicing, and no implemented fair-share accounting.

### Cited Findings
- Scheduling core pipeline: `EvalBroker` (priority queue of per-job evaluations with coalescing) → `Policy` (pure function of snapshot and now, "No locks held while deciding") → `PlanApplier` (single-writer commit with per-vertex optimistic concurrency) plus periodic `backfill_pass` — [crates/sched/src/lib.rs:1-25](../../crates/sched/src/lib.rs)
- `EvalBroker` orders by (priority descending, sequence ascending), i.e. FIFO within a priority, and dedups by job — [crates/sched/src/lib.rs:146-213](../../crates/sched/src/lib.rs)
- `Policy` trait: `place(graph, job, now, allow_reserve) -> Decision`, with optional `planned_secs`, `observe`, `begin_pass`, `extend_secs`, `schedule(snapshot, job) -> Plan`; doc comment says "Replaceable per-partition or as a WASM plugin" — [crates/sched/src/lib.rs:215-255](../../crates/sched/src/lib.rs)
- `Decision` is one of Start{vertices,start,dur}, Reserve{vertices,start,dur}, Defer — [crates/sched/src/lib.rs:60-77](../../crates/sched/src/lib.rs)
- Default `BackfillPolicy`: earliest-fit over per-vertex planners with a reservation window (`window_secs`, default 48 h, "Slurm's bf_window"); gangs placed all-or-nothing via `ResourceGraph::match_gang` — [crates/sched/src/lib.rs:257-289, 358-417](../../crates/sched/src/lib.rs)
- `backfill_pass`: extend overrunning jobs, clear reservations, then place pending jobs in priority order (ties by job id); the first `max_reservations` (default 32) that cannot start get reservations, the rest may only start now — [crates/sched/src/lib.rs:676-783](../../crates/sched/src/lib.rs)
- Planner `earliest_fit` is a single sweep over a usage step function, O(S log S) in spans — [crates/graph/src/lib.rs:87-124](../../crates/graph/src/lib.rs); validated against a brute-force oracle on 400 random cases — [results/EVALUATION.md:336-338](../../results/EVALUATION.md)
- Gang matching algorithm (fixpoint over per-leaf earliest fits, common start on N distinct nodes, seed-rotated choice to spread load) — [crates/graph/src/lib.rs:531-560](../../crates/graph/src/lib.rs)
- Per-vertex OCC: each vertex has a version; a plan carries a read set; commit takes a fast path if versions are unchanged, otherwise revalidates capacity and rejects only if the plan no longer fits — [crates/sched/src/lib.rs:437-498](../../crates/sched/src/lib.rs), [crates/graph/src/lib.rs:660-668](../../crates/graph/src/lib.rs)
- `RuntimeEstimator` trait (planned_secs, spans_on, begin_pass, observe, extend_secs) with `QuantileEstimator` (per-user runtime/walltime ratio at quantile theta) — [crates/sched/src/lib.rs:550-670](../../crates/sched/src/lib.rs)
- `BiasCorrectedEstimator` (EWMA of ln(runtime/walltime) over a feature hierarchy, pinball-loss online bias correction, reservation protection) and `WideSafeBackfill` (protected walltime slots for wide jobs, no-slip guarantee) — [results/EVALUATION.md:123-143, 199-228](../../results/EVALUATION.md); code in `crates/sched/src/bias.rs` (1,409 lines)
- Walltime planning stays the default; predictors are opt-in — [results/EVALUATION.md:17-21, 193-197](../../results/EVALUATION.md)
- `ShadowPolicy` runs a candidate policy beside the live one and records disagreements without applying them — [crates/sched/src/lib.rs:785-800](../../crates/sched/src/lib.rs), [results/EVALUATION.md:321-332](../../results/EVALUATION.md)
- Declarative `PolicyConfig` (JSON): weights for age, job size and QoS, QoS tiers (boost, max walltime), per-partition limits (max cores/GPUs/running jobs per user); pure `priority()` and `admits()` functions — [crates/policy/src/lib.rs:23-116](../../crates/policy/src/lib.rs)
- WASM plugin worlds (WIT package `gnomon:policy@0.1.0`): `priority(job, now) -> u64`, `admit(job, now) -> accept | reject(reason) | rewrite(request)`, `estimate(job) -> option<u64>`, `score(job, candidates, now) -> list<s64>` — [crates/policy/wit/policy.wit:57-83](../../crates/policy/wit/policy.wit)
- Controller flags wiring the plugins: `--admission-plugin`, `--admission-fail open|closed`, `--priority-plugin`, `--policy-config`, `--shadow-priority-plugin`, `--estimator-plugin`, `--node-score-plugin`, `--node-score-candidates`, `--plugin-fuel`, `--plugin-memory-mb` — [crates/ctl/src/plugins.rs:4-17](../../crates/ctl/src/plugins.rs)
- The controller binary constructs its scheduling policy from `BackfillPolicy` plus optional plugin wrappers (`ScoredPolicy`, `WasmEstimator`) — [crates/ctl/src/plugins.rs:22-41, 104-110](../../crates/ctl/src/plugins.rs), [crates/ctl/src/main.rs:279, 334](../../crates/ctl/src/main.rs)
- Evaluation limitation stated by the authors: "FCFS priority only; Slurm multifactor/fair-share orders would change absolute numbers." — [results/EVALUATION.md:428](../../results/EVALUATION.md)
- Fair-share: README lists "fairshare accounting history next" as not done — [README.md:335-336](../../README.md); `PolicyConfig` has no fairshare field (only age/jobsize/qos weights) — [crates/policy/src/lib.rs:25-34](../../crates/policy/src/lib.rs); ADR 0002 notes plugins cannot see fairshare usage history — [.github/plans/0002-wasm-policy-plugins.md:133-136](../../.github/plans/0002-wasm-policy-plugins.md)
- Design-doc proposals not listed as implemented in the roadmap: a workload classifier routing interactive/short jobs to a latency-optimized path, Borg-style score caching, and optional Firmament-style min-cost-flow passes — [docs/SLURM_Scheduler_Research_and_Rust_Design.md:136-138](../../docs/SLURM_Scheduler_Research_and_Rust_Design.md); roadmap at [README.md:324-349](../../README.md)

### Inferences
- No preemption/suspend/time-slice mechanism exists: a case-insensitive search for "preempt" in `crates/`, `proto/`, README and ARCHITECTURE returned nothing, the job state enum has no Suspended/Preempted state (core lib.rs:24-32), and the event enum has no preemption event. A running job ends only by completion, walltime, cancel, failed gang peer or node loss.
- Policy pluggability at the Rust level is real (anything implementing `Policy` can be passed to `backfill_pass` or the worker loop), but the shipped `gnomon-ctl` binary offers no flag to select a different `Policy` implementation; choosing `WideSafeBackfill` or a custom policy means writing Rust that links the crates (README.md:87-109 shows this as library code). WASM plugins can adjust priority, admission, runtime estimate and node choice but cannot replace the backfill algorithm itself ("where, never when" for node scoring, ADR 0004 line 58).
- Round-robin, shortest-job-first or similar queue disciplines are expressible only indirectly through the scalar `priority` (e.g. a priority plugin), since ordering is strictly by priority then id.

### Gaps
- I did not read `crates/sched/src/bias.rs` or `crates/policy/src/plugin/*.rs` line by line; their behaviour is taken from module docs and `results/EVALUATION.md`.
- Whether the classifier / fast path for short jobs proposed in the design doc exists anywhere in code was not confirmed; nothing in the README roadmap or crate table mentions it.

## 3. What is the resource model, and can it express GPUs, memory, tokens/rate limits or other abstract consumables?

### Takeaway
Resources are a fixed three-field struct (`cores`, `mem_mb`, `gpus`) booked on node vertices of an arena resource graph, each with a time-axis planner and optional time-varying capacity profile. GPUs and memory are first-class counts; there is no generic/named consumable (no license, token, rate-limit or custom dimension), and although the graph type enumerates Rack/Switch/Socket/Core/Gpu vertex kinds, the running system only creates a flat cluster → node tree.

### Cited Findings
- `Resources { cores: u32, mem_mb: u64, gpus: u32 }` with `fits_in`, `add`, `saturating_sub`; doc: "every dimension is first-class and matched inside the resource graph" — [crates/core/src/lib.rs:34-64](../../crates/core/src/lib.rs)
- The gRPC `Resources` message has the same three fields — [proto/gnomon.proto:18](../../proto/gnomon.proto)
- `VertexKind` enum: Cluster, Rack, Switch, Node, Socket, Core, Gpu — [crates/graph/src/lib.rs:19-28](../../crates/graph/src/lib.rs)
- Schedulable leaves are `Node` or `Gpu` vertices, with the comment "nodes (and later GPU vertices)" — [crates/graph/src/lib.rs:468-476](../../crates/graph/src/lib.rs)
- The FSM builds the graph as a `cluster` root plus one `Node` vertex per registered agent — [crates/fsm/src/lib.rs:712, 873-876](../../crates/fsm/src/lib.rs)
- Per-vertex `Planner` holds half-open spans `[start,end)` each consuming a `Resources` amount and tied to a job; answers "earliest start >= from where req fits continuously for dur" — [crates/graph/src/lib.rs:30-40, 87-124](../../crates/graph/src/lib.rs)
- `CapacityProfile`: piecewise-constant capacity over time, optionally periodic (daily), set through a replicated `NodeProfileSet` event; intended for power caps, demand response, carbon windows — [crates/core/src/lib.rs:66-128, 287-292](../../crates/core/src/lib.rs)
- Evaluated capacity-profile result: zero cap violations with 85% (power cap) / 67% (carbon window) lower median wait than static derating — [results/EVALUATION.md:293-319](../../results/EVALUATION.md)
- `SubgraphGrant`: a parent moves a set of node vertices to a child scheduler; granted vertices are excluded from parent matching; in-process only ("the distributed version sends a ReleaseSubgraph Raft event instead") — [crates/graph/src/lib.rs:1307-1400](../../crates/graph/src/lib.rs); README status "cross-process child instances next" — [README.md:337-338](../../README.md)
- GPU discovery on the agent is a count: number of lines from `nvidia-smi -L` or `rocm-smi -L`, overridable with `GNOMON_GPUS` — [crates/agent/src/main.rs:24-62](../../crates/agent/src/main.rs)
- GPU enforcement at launch: Docker gets `--gpus <n>`; Podman omits it (CDI); README roadmap lists "CDI injection next" — [README.md:252-265, 333-334](../../README.md)
- Declarative per-user limits exist only for cores, GPUs and running-job count per partition — [crates/policy/src/lib.rs:43-49, 99-115](../../crates/policy/src/lib.rs)
- Design doc's target model is broader than what is built (vertices for "memory pool, burst buffer, power domain", GPU topology as graph edges) — [docs/SLURM_Scheduler_Research_and_Rust_Design.md:132](../../docs/SLURM_Scheduler_Research_and_Rust_Design.md)

### Inferences
- Abstract consumables (LLM tokens-per-minute, API rate limits, context-window slots, model-server concurrency) cannot be expressed without changing `Resources` in `gnomon-core` and the proto; the struct is closed (three named integer fields, `Copy`), so adding a dimension is a source change touching core, graph, proto, policy WIT and agent.
- A workaround that needs no code change is to model a non-hardware pool as a synthetic "node" whose `cores`/`gpus` count stands for concurrency slots; a time-windowed quota (e.g. N units per period) loosely maps to a periodic `CapacityProfile`. This is my inference; the repo documents profiles only for power and carbon caps, and profile capacity is a concurrent-usage ceiling, not a cumulative token budget.
- No GPU identity, type, memory size, topology or fractional sharing is modelled; GPUs are an undifferentiated integer per node.

### Gaps
- Not verified whether any code path can register a vertex of kind `Gpu`, `Socket` or `Rack` outside tests; the only non-test `add_vertex` calls found in `crates/fsm` create `Cluster` and `Node`.

## 4. What external interfaces exist, and could the scheduling core be used as a library?

### Takeaway
One gRPC service (`gnomon.v1.Scheduler`, 10 unary RPCs, tonic, default port 7631) serves both users and node agents; there are Slurm-style CLIs and a TUI but no REST API, no streaming/watch RPC and no language SDK beyond the generated Rust client. The planning core (`gnomon-core`, `-graph`, `-fsm`, `-sched`) is plain synchronous Rust with only `serde` as an external dependency and is already used as a library by the simulator and benchmarks.

### Cited Findings
- Service definition: SubmitJob, CancelJob, GetJob, ListJobs, ListNodes (user side); RegisterNode, Heartbeat, FetchWork, ReportCompletion, GangReady (agent side) — [proto/gnomon.proto:4-16](../../proto/gnomon.proto)
- `SubmitJobRequest`: name, user, partition, request (cores/mem_mb/gpus), walltime_secs, priority, command, nodes, container — [proto/gnomon.proto:20-26](../../proto/gnomon.proto)
- `FetchWorkReply` carries assignments (job_id, command, request, start, walltime, name, container, nodelist, rank, user) plus a kill list — [proto/gnomon.proto:62-76](../../proto/gnomon.proto)
- `GetJob(wait_index)` is a blocking query: the handler polls every 20 ms until state passes the index, with a 30 s deadline — [crates/api/src/lib.rs:175-195](../../crates/api/src/lib.rs); `ListJobs` returns `state_index` — [proto/gnomon.proto:35](../../proto/gnomon.proto)
- Reads are served from immutable FSM snapshots; uncommittable writes return `UNAVAILABLE` — [crates/api/src/lib.rs:1-13, 62-70](../../crates/api/src/lib.rs)
- No authentication fields: `user` is a plain string in SubmitJob and CancelJob — [proto/gnomon.proto:21, 28](../../proto/gnomon.proto)
- `SubmitPolicy` trait (`prepare(&mut Job, now) -> Result<(), String>`) is the server-side hook for admission and priority, also implemented for closures — [crates/api/src/lib.rs:38-55](../../crates/api/src/lib.rs)
- Agent, CLI and gntop link only the generated client from `gnomon-api`, "so node and user tools never link the scheduler" — [docs/ARCHITECTURE.md:86-94](../../docs/ARCHITECTURE.md)
- CLI: `gnbatch` (parses `#SBATCH`, `-c`, `--gpus`, `-t`, `-N`, `--container`), `gnqueue`, `gninfo`, `gncancel`, `gntop`; address from `GNOMON_ADDR` (may list several controllers), timeout `GNOMON_TIMEOUT_SECS` default 10 — [README.md:145-157](../../README.md)
- Dependencies of the core crates: `gnomon-sched` depends only on core, fsm, graph; fsm and graph only on core and serde; core only on serde — `crates/sched/Cargo.toml`, `crates/fsm/Cargo.toml`, `crates/graph/Cargo.toml`, `crates/core/Cargo.toml` (dependency sections)
- Library-style usage shown in README (constructing `BackfillPolicy::with_estimator`, `WideSafeBackfill::new`, `ShadowPolicy::new`, `Fsm::recording(clock)`, `Simulator::new(machine, candidate).run(&history)`) — [README.md:85-109](../../README.md)
- `gnomon-sim` "runs the production backfill_pass against recorded or synthetic workloads" — [docs/ARCHITECTURE.md:88-90](../../docs/ARCHITECTURE.md)
- Embedding API surface: `Fsm::new/with_clock/recording`, `apply`, `apply_checked`, `try_apply`, `snapshot`, `read`, `next_job_id`, `set_sink`, `set_proposer` — [crates/fsm/src/lib.rs:227-404](../../crates/fsm/src/lib.rs); injected `Clock` trait with `SystemClock` and `ManualClock` — [crates/core/src/lib.rs:312-341](../../crates/core/src/lib.rs)
- A minimal in-process loop (register nodes, submit jobs, broker, worker threads, applier) is in the controller's `demo()` — [crates/ctl/src/main.rs:144-255](../../crates/ctl/src/main.rs)
- Packaging: release tarballs for x86_64/aarch64 Linux and aarch64 macOS, container images `ghcr.io/<owner>/gnomon-{ctl,agent,cli}`, systemd units — [README.md:139](../../README.md), `packaging/systemd/`, [docs/INSTALL.md](../../docs/INSTALL.md) (sections at lines 36-127)
- REST API, Kubernetes operator, Prometheus/OpenTelemetry are proposed in the design doc — [docs/SLURM_Scheduler_Research_and_Rust_Design.md:146](../../docs/SLURM_Scheduler_Research_and_Rust_Design.md); none appears in the README crate table or roadmap — [README.md:39-52, 324-349](../../README.md)

### Inferences
- Three integration levels are possible for an outside system: (a) call the gRPC API as a client to submit/cancel/poll batch jobs; (b) implement the agent side of the protocol (RegisterNode/Heartbeat/FetchWork/ReportCompletion) so a non-Gnomon executor receives assignments, since the controller never dials out; (c) link `gnomon-core/graph/fsm/sched` as Rust crates and drive `Fsm` + `Policy` + `PlanApplier` in-process with a custom clock. Level (c) is what `gnomon-sim` and the benches already do.
- A non-Rust host (AIOS is Python, per the caller's framing; not verified here) would need gRPC or an FFI/pyo3 wrapper; no bindings exist in the repo.
- Job payload is an opaque `command` string executed by `sh -c`; an alternative executor implementing the agent protocol could interpret that string differently, but that is a convention the repo does not define.
- Crates are path dependencies in a workspace; nothing in the files read indicates publication to crates.io.

### Gaps
- No API versioning or stability policy was found beyond the proto package name `gnomon.v1` and the WIT package version `0.1.0`.
- Completion notification to submitters is poll-only (GetJob/ListJobs); README lists "push updates through index-aware blocking queries" as future work for gntop (README.md:346-349).

## 5. What does it say about latency/throughput, fault tolerance and extensibility?

### Takeaway
Measured decision throughput is in the low thousands of jobs per second through the full controller path (and tens to hundreds of thousands of decisions per second for the bare planning core on small problems), but end-to-end dispatch latency is bounded by second-scale polling (1 s backfill pass, agent poll of 2 s or more, whole-second clock). Fault tolerance is opt-in openraft replication with a durable log; extensibility is fuel- and memory-bounded pure WASM components costing roughly 10-35 microseconds per call.

### Cited Findings
- Controller-path benchmark (`occ_bench`: broker → snapshot → plan → applier; 256 nodes, 2,048 jobs, background heartbeats, 2-core sandbox): 1,825-3,151 jobs/s with per-vertex OCC vs 1,404-2,231 before; conflicts down 92-100% — [results/EVALUATION.md:37-51](../../results/EVALUATION.md), raw data [results/occ_conflicts.csv:1-19](../../results/occ_conflicts.csv)
- Same benchmark on a 4-vCPU VM: 1,758-4,708 jobs/s (PerVertex) — [results/EVALUATION.md:368-380](../../results/EVALUATION.md)
- Planning-core benchmark (decisions only, no RPC/Raft): serial 733,495 decisions/s at 64 nodes/256 jobs, falling to 14,061/s at 1,024 nodes/8,192 jobs; parallel core 21,059-22,109/s at the largest size — [bench_results.csv:1-21](../../bench_results.csv); interpretation (serial wins at ≤1,024 jobs, parallel wins 1.5-1.6x at 4,096-8,192) — [results/EVALUATION.md:334-342](../../results/EVALUATION.md)
- Authors' caveat: "these are decision-core numbers only — no RPC, Raft replication or real-cluster cost"; "no numbers exist against real SLURM, Flux, Spur, or Kubernetes installations; ... no failure-injection or Raft-failover timing" (written before the later Slurm side-by-side tests) — [docs/SLURM_Scheduler_Research_and_Rust_Design.md:217-219](../../docs/SLURM_Scheduler_Research_and_Rust_Design.md)
- Live test kit vs Slurm 23.11 on one VM: 50-job burst submitted at 119.7 jobs/s (Slurm 42.7) and drained in 27 s (Slurm 37 s); queue wait for a single job 1 s — [results/testkit/COMPARISON.md:8-23](../../results/testkit/COMPARISON.md)
- 3-node container cluster: submit 141.1 jobs/s vs Slurm 100.7; 12 jobs started 2 s after first submit (Slurm 5 s); gang rank start spread 104 ms vs Slurm 4 ms — [results/testkit/COMPARISON.md:66-80](../../results/testkit/COMPARISON.md)
- Timing constants: scheduler worker thread sleeps 10 ms when the broker is empty; backfill pass every 1 s; agent poll starts at 2 s then uses the controller's heartbeat interval (min 2 s); gang barrier polled every 200 ms — [crates/ctl/src/main.rs:375, 421](../../crates/ctl/src/main.rs), [crates/agent/src/main.rs:87, 123-129](../../crates/agent/src/main.rs), [README.md:186-190](../../README.md)
- Every `Fsm::snapshot()` clones the whole state (`Arc::new(self.state.read().unwrap().clone())`) — [crates/fsm/src/lib.rs:313-320](../../crates/fsm/src/lib.rs); this cost caused a measured 0.55x throughput regression when per-job data grew, later fixed to 0.92-0.96x — [results/EVALUATION.md:357-366](../../results/EVALUATION.md)
- Schedule-quality results on Parallel Workloads Archive traces (9 windows × 5,000 jobs): bias-corrected predictor median −10% mean wait, −20% bounded slowdown; wide-job shield −18% wide-job wait with −5.1% mean wait; oracle −15.7% / −29.4% — [results/EVALUATION.md:149-159, 248-254](../../results/EVALUATION.md)
- Replay: 132,421-event journal replays to bit-identical state; 5,000 start times reproduced; simulate 5,000 jobs in 979 ms, rebuild from journal in 403 ms — [results/EVALUATION.md:66-74](../../results/EVALUATION.md)
- Replication: openraft 0.9 over a custom TCP transport; every write (API, plan applier, backfill pass) goes through a `Proposer` hook into the Raft log; followers forward writes and serve reads from their own state; only the leader schedules; no quorum → `UNAVAILABLE`; static membership — [README.md:159-177](../../README.md), [docs/ARCHITECTURE.md:218-252](../../docs/ARCHITECTURE.md), [crates/fsm/src/lib.rs:195-201](../../crates/fsm/src/lib.rs), `crates/consensus/Cargo.toml` (openraft = 0.9)
- Durability: `--state-dir` journals every event to an fsync'd checksummed WAL; `DiskStore` persists Raft log, vote, snapshots with torn-tail recovery; without it the controller is in-memory — [README.md:68](../../README.md), [crates/ctl/src/main.rs:10-14](../../crates/ctl/src/main.rs)
- Replication test evidence: 3 replicas over TCP converge, survive SIGKILL of the leader, restarted replica catches up byte-identically, minority refuses writes; live run with 3 controllers + 3 agents — [results/EVALUATION.md:418](../../results/EVALUATION.md)
- Known at-least-once edge: "a leader that steps down mid-write can still make one event apply twice, which every controller event tolerates" — [README.md:175-177](../../README.md)
- Node failure handling: leader declares a node lost after `GNOMON_NODE_TIMEOUT_SECS` (default 3 heartbeat intervals, min 30 s); running ranks fail, unfetched work returns to Pending — [README.md:219-226](../../README.md), [crates/ctl/src/main.rs:68-82](../../crates/ctl/src/main.rs)
- WASM sandbox rules: plugins run only where a write is prepared (never during FSM apply or replay); no imports (no WASI, no clock, no I/O); per-call fuel (default 10 M instructions) and memory cap (default 16 MiB); failure falls back to declarative behaviour — [crates/policy/src/plugin/mod.rs:1-17, 61-80](../../crates/policy/src/plugin/mod.rs), [.github/plans/0002-wasm-policy-plugins.md:64-81](../../.github/plans/0002-wasm-policy-plugins.md)
- wasmtime version 36.0.20, component model with WIT bindgen — `crates/policy/Cargo.toml`, [crates/policy/src/plugin/mod.rs:25-54](../../crates/policy/src/plugin/mod.rs)
- Plugin overhead (Apple-silicon laptop): estimator about 10-20 µs per job (cached), capped node scoring about 25-35 µs per placement; uncapped node scoring about 175 µs per placement at 1,024 nodes; full pass at 1,024 nodes 103 ms (walltime) vs 275 ms (both plugins, cap 64) — [.github/plans/0004-node-score-plugins.md:62-88](../../.github/plans/0004-node-score-plugins.md)
- Example plugins in Rust (admission, priority, estimator, node-score) built separately for wasm32 — `examples/policy-plugins/` ([README](../../examples/policy-plugins/README.md)), excluded from the workspace at [Cargo.toml:17-18](../../Cargo.toml)
- Plugin expressiveness limit: "plugins see only the fields passed in"; "Pure functions cannot call external services" — [.github/plans/0002-wasm-policy-plugins.md:133-136](../../.github/plans/0002-wasm-policy-plugins.md)

### Inferences
- For workloads lasting seconds to hours, measured overheads are small relative to job length. For sub-second units (an LLM call queue), the built-in daemon path has structural floors: whole-second timestamps, a 1 s re-plan pass, and agent polling of at least 2 s. The event-driven broker path (10 ms idle sleep) can make a start decision quickly, but delivery to an executor still waits for the next `FetchWork` poll.
- Used in-process (level (c) in section 4) these polling floors do not apply, but the clock/planner granularity remains integer seconds, and each worker evaluation clones full state, so cost grows with the number of tracked jobs and nodes.
- The pure-function, no-import rule means a WASM policy plugin could not itself call an LLM or any external service to make a decision.

### Gaps
- No latency percentiles (submit-to-start, decision time distributions) are reported anywhere; only throughput and coarse wait seconds.
- No Raft failover time or replicated-mode throughput number was found; the bench figures are for the non-replicated path.
- All benchmarks ran on a 2-core sandbox or 4-vCPU VM; no multi-host hardware results ("separate hardware is still to be compared", EVALUATION.md:422).

## 6. What is missing or immature? Any mention of AI, LLM, agents, inference?

### Takeaway
Gnomon is a young (10 commits, v0.5.0) research implementation with unusually heavy verification for its age, but several production features are explicitly "next", and the repository contains no AI/LLM/agent-workload awareness beyond counting GPUs and one design-doc sentence about Slurm's gaps for "modern workloads".

### Cited Findings
- History: 10 commits; version bump "from 0.1.0 to 0.5.0"; WASM policy plugins landed as PR #1 — `git log --oneline` (HEAD `7b00379`), [Cargo.toml:21](../../Cargo.toml)
- Self-description as "A research codebase" — [README.md:29](../../README.md); controller module doc still says "the controller daemon skeleton" — [crates/ctl/src/main.rs:1](../../crates/ctl/src/main.rs)
- Tests: README says "~240 tests" covering planner (brute-force oracle), OCC, replay, estimators, profiles, policy, WASM, delegation, kill channel, 3-replica Raft, agent launch — [README.md:115-116](../../README.md); a grep for `#[test]`/`#[tokio::test]` under `crates/` counts 307 attributes (my count, 2026-11-20)
- CI: fmt, clippy -D warnings, tests, line coverage ≥ 80% enforced, replay gate against `results/replay_baseline.json`, controller-throughput A/B failing below 85%, host validation on Ubuntu 24.04 with Docker and Podman, weekly Slurm side-by-side, cargo audit — [README.md:130-141](../../README.md)
- Replay baseline holds six scenarios (three SWF windows, three synthetic), all `replay_exact: true` — [results/replay_baseline.json:1-58](../../results/replay_baseline.json)
- Explicit "next"/open items in the roadmap: dynamic Raft membership; CDI GPU injection; fairshare accounting history; cross-process child instances; better runtime predictors and deferrable-job carbon policies; comparison on separate hardware; push updates for gntop — [README.md:324-349](../../README.md)
- Evaluation limitations stated: "Simulation, not a cluster: SWF traces record processor counts, so the machine is one vertex with N processors; there is no network, I/O or Raft replication cost"; FCFS priority only; 2-core sandbox — [results/EVALUATION.md:424-430](../../results/EVALUATION.md)
- Slurm comparison caveat: one-node or 3-container clusters on one VM, "treat timings as indicative" — [results/testkit/COMPARISON.md:25-37](../../results/testkit/COMPARISON.md)
- Design-doc items with no implementation listed in README: REST API, Kubernetes operator CRD, Prometheus/OpenTelemetry, async accounting sink (SQLite/Parquet), MCMF global passes, interactive/short-job fast path — [docs/SLURM_Scheduler_Research_and_Rust_Design.md:128, 136-138, 146](../../docs/SLURM_Scheduler_Research_and_Rust_Design.md)
- AI/ML mention in product docs: a single sentence that practitioners flag Slurm gaps "for modern workloads: no native elastic scaling or scale-to-zero, batch-only semantics that fit serving/interactive workloads poorly, static resource reservation causing fragmentation", citing a "slurm-for-ai-ml-workloads" blog — [docs/SLURM_Scheduler_Research_and_Rust_Design.md:35](../../docs/SLURM_Scheduler_Research_and_Rust_Design.md)
- "Agent" in this repo means the per-node daemon `gnomon-agent` (registration, heartbeats, job launch) — [README.md:49](../../README.md)
- GPU awareness is limited to: a `gpus` count in `Resources`, agent probing via nvidia-smi/rocm-smi, `--gpus` passed to Docker, per-user GPU limits, GPU weight 4x cores in the declarative size factor, and example plugins that route GPU jobs to a `gpu` partition and keep GPU nodes for GPU jobs — [crates/core/src/lib.rs:40](../../crates/core/src/lib.rs), [crates/agent/src/main.rs:38-62](../../crates/agent/src/main.rs), [crates/policy/src/lib.rs:47, 89](../../crates/policy/src/lib.rs), [examples/policy-plugins/README.md:10-12](../../examples/policy-plugins/README.md)
- The only other "LLM" strings are in the graphify tooling script, unrelated to the scheduler — [.github/graphify/update.sh:25, 218-223](../../.github/graphify/update.sh)
- The README usage example names a job "train" with `--gpus 2` — [README.md:150](../../README.md)

### Inferences
- A whole-word, case-insensitive search across `.md`, `.rs`, `.proto`, `.toml`, `.sh`, `.py` files for AI, LLM, agentic, inference, "machine learning", ML, token and "rate limit" found no scheduler feature aimed at LLM or agent workloads; matches were the design-doc sentence above, "token" in the optimistic-concurrency sense, "conformal inference" (a statistics citation), loop variables named `ai`, and graphify tooling. (The search filtered out lines containing "kill", so a line mentioning both would have been missed.)
- Gnomon's stated direction is HPC batch (Slurm succession, power/carbon caps, gang jobs). Serving/interactive workloads are named as a Slurm weakness but Gnomon itself keeps batch semantics: every job needs a walltime, runs to completion, and is never preempted.
- Missing pieces most relevant to an LLM/agent-syscall scheduler, as inferred from the above: no preemption or time-slicing; no sub-second time; no generic consumable resources; no fair-share history; no streaming/push interface; no authentication on the API; no non-Rust bindings.
- Strengths that could transfer, also inferred: deterministic event-sourced core with exact replay and shadow evaluation of candidate policies; pure-function `Policy` trait; optimistic concurrent planning with per-resource versioning; time-axis reservations with capacity profiles; sandboxed policy hooks.

### Gaps
- No issue tracker, release notes or CHANGELOG was examined (local repo only); whether tagged releases exist was not checked.
- The README's "~240 tests" and my 307 attribute count differ; I did not run `cargo test` to get an executed-test count, nor verify that the build or tests pass in this checkout.
- No statement of intended production users or deployments was found.
