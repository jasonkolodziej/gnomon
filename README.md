<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo/svg/gnomon-lockup-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="assets/logo/svg/gnomon-lockup-light.svg">
    <img src="assets/logo/svg/gnomon-lockup-light.svg" alt="Gnomon" height="72">
  </picture>
</p>

# Gnomon

[![Rust](https://github.com/jasonkolodziej/gnomon/actions/workflows/rust.yml/badge.svg)](https://github.com/jasonkolodziej/gnomon/actions/workflows/rust.yml)
[![End-to-end](https://github.com/jasonkolodziej/gnomon/actions/workflows/e2e.yml/badge.svg)](https://github.com/jasonkolodziej/gnomon/actions/workflows/e2e.yml)
[![Benchmarks](https://github.com/jasonkolodziej/gnomon/actions/workflows/bench.yml/badge.svg)](https://github.com/jasonkolodziej/gnomon/actions/workflows/bench.yml)
[![Security Audit](https://github.com/jasonkolodziej/gnomon/actions/workflows/audit.yml/badge.svg)](https://github.com/jasonkolodziej/gnomon/actions/workflows/audit.yml)

A Rust-native HPC scheduler. The name is the thesis, twice over:

- A **gnomon** is the upright of a sundial — the piece whose shadow *marks time*.
  Scheduling here is managing the time axis (per-vertex Planners, parallel
  optimistic backfill), not just packing nodes.
- In geometry, a **gnomon** is the shape you add to a figure to grow it into a
  larger figure of the same form. That is backfill (filling the gaps in the
  plan) and nested instances (a delegated subgraph that is itself a full
  scheduler).

Pronounced *NO-mon*. CLI shims use the `gn` prefix: `gnbatch`, `gnqueue`,
`gncancel`, `gninfo`, plus `gntop`, a live Gantt view of the plan.

A research codebase implementing the Rust-native HPC scheduler architecture
proposed in [`docs/SLURM_Scheduler_Research_and_Rust_Design.md`](docs/SLURM_Scheduler_Research_and_Rust_Design.md).

- **Install:** [`docs/INSTALL.md`](docs/INSTALL.md): release tarballs, container images, systemd units,
  cgroup and rootless Docker setup, and a reference for every binary.
- **Architecture:** [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): diagrams of the deployment, crates,
  controller pipeline, job lifecycle, replication and node-side launch.

## Crates

| Crate | Role | Report section |
|---|---|---|
| `gnomon-core` | Job/resource/event types, serde-serializable for the Raft log | 4.1, 4.5 |
| `gnomon-graph` | Resource graph, per-vertex Planners (O(S log S) sweep), **per-vertex versions**, **time-varying capacity profiles**, **ownership-typed subgraph grants** | 4.2, 4.4 |
| `gnomon-fsm` | Replicated state machine, snapshots, injected clock, **event journal**, node registry, work assignments, **kill list** (gang peers, running-job cancel), **`Proposer` hook** routing writes through Raft | 4.1 |
| `gnomon-sched` | Coalescing eval broker, parallel workers, **per-vertex optimistic plan applier**, `backfill_pass`, **runtime-quantile estimator**, **bias-corrected estimator** with the **wide-job shield** (`WideSafeBackfill`), **shadow mode** | 4.3 |
| `gnomon-policy` | **Policy-as-data** (fairshare/QoS/limits JSON) + **wasmtime-sandboxed plugins** for admission, priority, runtime estimation and node scoring | 4.5 |
| `gnomon-consensus` | **openraft** storage bridging Raft to the FSM: in-memory `Store` and **durable `DiskStore`** (fsync'd checksummed log, atomic vote/snapshot, crash recovery); **TCP Raft transport** and **replicated controller** (`replica`): writes committed through the log, forwarded from followers; durable event journal for the single controller | 4.1 |
| `gnomon-api` | **tonic gRPC** service: lock-free snapshot reads, index-aware blocking queries; writes that cannot be committed are `UNAVAILABLE`; kill list delivered with `FetchWork` | 4.3 |
| `gnomon-ctl` | Controller daemon (gRPC server + background scheduler loop); `--state-dir` / `GNOMON_STATE_DIR` makes it restart-safe; `--raft-id`/`--raft-peers` makes it one replica of a fault-tolerant group | 4.6 |
| `gnomon-agent` | Node agent: real /proc + GPU probing, registration, heartbeats, job launch in cgroup v2 or rootless containers, **kill channel** (gang peers, running-job cancel), controller failover | 4.6 |
| `gnomon-cli` | **Slurm-compatible shims**: `gnbatch` (parses #SBATCH), `gnqueue`, `gncancel` (also kills running jobs), `gninfo`; `GNOMON_ADDR` may list every controller | 4.5 |
| `gnomon-top` | **`gntop`**: `top`-like live Gantt chart of the cluster plan (running spans + backfill reservations per node) | 4.2, 5.2 |
| `gnomon-sim` | **Exact replay**, counterfactual policy evaluation on recorded history, SWF trace simulator, metrics | 6.3 |

## What's new in this iteration

Evaluated on real Parallel Workloads Archive traces — full write-up in
[`results/EVALUATION.md`](results/EVALUATION.md).

| Change | Default | Result |
|---|---|---|
| **Replicated controllers (Raft).** `gnomon-ctl --raft-id N --raft-peers 1=h:p,2=h:p,3=h:p --state-dir DIR`: openraft over a TCP transport; every write (API, plan applier, backfill pass) is committed through the log via a `Proposer` hook on the FSM, followers forward writes to the leader and read their own writes, plan read sets travel in the log so every replica revalidates identically, and only the leader schedules. Agents and CLIs take a list of controllers in `GNOMON_ADDR` and fail over. | Opt-in (`--raft-id`) | Three replicas over TCP: converge, survive SIGKILL of the leader, a restarted replica catches up byte-identically, a minority refuses writes. Live: 3 controllers + 3 agents, leader and the agents' controller each SIGKILLed mid-job, every job completed. Replay gate unchanged |
| **Kill channel: a failed gang rank stops its peers.** The first non-zero rank exit puts every unreported rank on a replicated per-node kill list delivered with `FetchWork`; agents stop the rank's cgroup/container (exit 143), release ranks still at the barrier and report ranks that never started. The same path makes `gncancel` work on **running** jobs (they end `CANCELLED`). Unconfined jobs now run in their own process group, so walltime and kill reach grandchildren. | On | Live 3-node gang with rank 0 failing after 2 s: FAILED within one agent poll instead of holding three nodes for 120 s, no processes left; replay of kill-channel state exact |
| **Real-host validation of cgroup v2 and containers.** `testkit/validate-host.sh` runs the launcher against the kernel's cgroup v2 (limits read back from inside the job, OOM within `memory.max`, CPU capped at the requested cores, `cgroup.kill` reaching `setsid` escapees) and a real Docker/Podman (env, user, `--cpus`/`--memory`, OOM 137, walltime and kill stop the container itself). | CI (`host-validation.yml`, Ubuntu 24.04, Docker and Podman) | 11/11 on the CI hosts with both Docker and Podman. Caught a real bug: rootful Podman rejects `--userns=keep-id`, now passed only for a non-root agent |
| **Per-vertex optimistic concurrency.** A plan conflicts only if a vertex it uses changed *and* it no longer fits (was: any event, even a heartbeat). Also fixes rejected plans counted as committed and double placement of one job. | On | 92–100% fewer conflicts, 1.15–1.99× controller throughput |
| **Exact replay + counterfactuals + shadow mode.** Journal → bit-identical state; recovered history re-scheduled under any policy with the production code; candidate policies run alongside the live one. | Tooling | 5,000-job trace: replay exact, same policy reproduces every start time |
| **Distribution-aware backfill.** Spans planned at a learned runtime quantile (risk budget); overruns extended, walltime still the kill limit. | Opt-in | Oracle bound: −16% wait, −29% slowdown. Per-user quantile: inconsistent (median −3% wait at θ=0.7, range −15…+21%). New `BiasCorrectedEstimator` (pinball-loss bias correction + reservation protection): lower mean wait in 9/9 windows (median −10%, −20% slowdown), but wide jobs wait longer in 4/9. The new **wide-job shield** (`WideSafeBackfill`) fixes that: wide jobs wait less than under walltime planning in 9/9 windows (median −18%), for a smaller mean-wait gain (median −5%, better in 8/9). Walltime stays default |
| **Multi-node gang jobs.** `gnbatch -N 4` / `#SBATCH --nodes=4`: the per-node request is placed on N distinct nodes reserved from one common planned start (all-or-nothing, oracle-tested `match_gang`), in both the start-now and reservation paths; agents get ranked assignments with the nodelist (each rank launches on its agent's next poll), and the FSM aggregates per-rank completions so replay stays exact. | On (`-N` defaults to 1) | Mixed 16-node simulation: no node oversubscribed, gang ranks start together, full-machine gang not starved under conservative backfill |
| **Durable controller state + on-disk Raft storage.** `gnomon-ctl --state-dir DIR` journals every event to a checksummed, fsync'd write-ahead log and restores jobs, nodes and reservations on restart; `DiskStore` persists openraft's log, vote, commit point and snapshots with torn-tail recovery. | Opt-in (`--state-dir`) | Restart tests: identical state fingerprint after recovery; a real single-node openraft instance commits, restarts and resumes |
| **cgroup v2 confinement + rootless containers.** Each job gets `cpu.max`/`memory.max` in its own cgroup (joined before `exec`, killed with `cgroup.kill`), or runs under rootless Docker/Podman with `--container IMAGE`; `GNOMON_NODELIST`/`GNOMON_NODE_RANK` for gangs. | On when cgroup2 is writable; containers per job | Tested against a fake cgroupfs and a fake `docker`; falls back to unconfined launches on cgroup v1 hosts |
| **`gntop`.** Live Gantt TUI of every node's planner: running spans, backfill reservations, the pending queue. | Tool | Render-tested on ratatui's `TestBackend`; `--once` for CI and screenshots |
| **Multi-node Gnomon vs Slurm.** `testkit/multinode.sh` builds a 3-node Slurm 23.11 and a 3-node Gnomon on identical containers and runs the kit plus multi-node tests (gangs, whole-node spread, fill, placement, failed-gang kill). | CI weekly | Both pass t01–t10 and t12; Gnomon's gang ranks start within ~100 ms (barrier poll) vs ~5 ms for `srun`; Gnomon spreads, Slurm packs. Nodes are containers on one VM |
| **Verification.** The seven-test kit runs against Gnomon on every PR and side by side with a single-node Slurm weekly; the *replay gate* fails any PR that worsens schedule quality or breaks exact replay; line coverage ≥ 80% enforced. | CI | Both schedulers pass 7/7 ([`results/testkit/COMPARISON.md`](results/testkit/COMPARISON.md)); coverage 58% → 86% |
| **Time-varying capacity.** Per-node `CapacityProfile` (power caps, carbon windows) honoured natively by the planner. | Opt-in | Zero cap violations with 85% (power) / 67% (carbon) lower median wait than static derating; no job made unrunnable |

Also: the planner's `earliest_fit` is now a single sweep (validated against a brute-force oracle),
completed jobs release the rest of their reserved window, and the controller runs a 1 s
`backfill_pass` so reservations are re-planned and reserved jobs actually start. Two live-path bugs
found by the end-to-end test are fixed: agents re-ran every queued job on each poll (assignments are
now drained by a replicated `AssignmentsFetched` event — exactly-once dispatch), and `gnbatch` sent a
relative script path the agent could not resolve.

Trace files are not bundled: download the SWF traces from the
[Parallel Workloads Archive](https://www.cs.huji.ac.il/labs/parallel/workload/).

### Using the new features

```rust
// Runtime-quantile planning with a 10% risk budget
let est = Arc::new(QuantileEstimator::new(0.90));
let policy = BackfillPolicy::with_estimator(48 * 3600, est);

// Demand-response cap: 60% of cores 16:00-20:00 daily
fsm.apply(Event::NodeProfileSet { name: "node0".into(), profile: Some(CapacityProfile::daily(vec![
    (0, full), (16 * 3600, sixty_pct), (20 * 3600, full)])) });

// Bias-corrected runtime prediction with the wide-job shield: wide jobs keep protected
// walltime slots that prediction errors cannot push back (results/EVALUATION.md §3b)
let est = Arc::new(BiasCorrectedEstimator::new(BiasConfig::default()));
let policy = WideSafeBackfill::new(48 * 3600, est);   // .with_ahead(NarrowAhead::Planned) is the default

// Shadow a candidate policy on live traffic
let shadow = ShadowPolicy::new(live_policy, candidate_policy); // shadow.agreement(), shadow.diffs

// Replay / counterfactual
let fsm = Fsm::recording(clock);                     // journal every event
let same = gnomon_sim::replay::verify_exact(&fsm);   // bit-identical rebuild
let history = gnomon_sim::replay::workload_from_journal(&fsm.journal());
let what_if = Simulator::new(machine, candidate).run(&history);
```

## Build & test

```sh
cargo build --workspace          # needs protoc (PROTOC env var supported)
cargo test --workspace           # ~240 tests: planner (incl. brute-force oracle), OCC, replay, estimators and the wide-job shield,
                                 # profiles, policy, WASM, delegation, kill channel, 3-replica Raft over TCP, agent launch
testkit/validate-host.sh         # opt-in real-host suite (root): cgroup v2 limits and Docker/Podman containers
cargo run --release -p gnomon-sched --bin bench       # planning-core A/B benchmark
cargo run --release -p gnomon-sched --bin occ_bench   # controller-path concurrency benchmark
cargo run --release -p gnomon-sim --bin experiments -- <swf-dir> results 5000   # trace evaluation
```

## Test kit (Gnomon vs Slurm)

`testkit/` runs seven scheduler tests (smoke, resources, backfill, cancel, throughput, walltime,
exit codes) unchanged against Gnomon or a Slurm cluster and compares the two — see
[`testkit/README.md`](testkit/README.md). `testkit/multinode.sh all` does the same on a 3-node Slurm
and a 3-node Gnomon built from identical containers, adding multi-node tests t08–t12.

## Continuous integration

| Workflow | Runs | Checks |
|---|---|---|
| [`rust.yml`](.github/workflows/rust.yml) | push to `main`, pull requests | `cargo fmt --check`, `cargo clippy -D warnings`, Linux build + `cargo test`, macOS workspace build and identity tests, `cargo doc` with `-D warnings`, **line coverage ≥ 80%** (`cargo llvm-cov`; benchmark drivers excluded) |
| [`e2e.yml`](.github/workflows/e2e.yml) | push to `main`, pull requests; weekly + on demand | Test kit against a live controller and agents (quick); weekly, full length against Gnomon **and** a single-node Slurm with a side-by-side comparison; ShellCheck |
| [`bench.yml`](.github/workflows/bench.yml) | push to `main`, pull requests | **Replay gate**: production scheduling path on synthetic workloads and real SWF traces, fails if schedule quality regresses vs [`results/replay_baseline.json`](results/replay_baseline.json) or replay is not exact; on PRs, **controller throughput A/B** against the base branch (`occ_bench`, interleaved runs, fails below 85%); planner and OCC benchmarks published to the job summary |
| [`host-validation.yml`](.github/workflows/host-validation.yml) | agent/API/proto changes, push to `main`, weekly | `testkit/validate-host.sh` on Ubuntu 24.04 (cgroup v2): job confinement against the real kernel and real Docker and Podman (matrix) |
| [`audit.yml`](.github/workflows/audit.yml) | `Cargo.toml`/`Cargo.lock` changes, weekly | `cargo audit` against the RustSec advisory database |
| [`release.yml`](.github/workflows/release.yml) | tag `v*` (manual: dry run for a tag) | Checks the tag matches the workspace version, builds and smoke-tests tarballs for x86_64/aarch64 Linux and aarch64 macOS, pushes `ghcr.io/<owner>/gnomon-{ctl,agent,cli}` (amd64 + arm64), publishes a GitHub release with `SHA256SUMS` |
| [`ci.yml`](.github/workflows/ci.yml) | push to `main`, pull requests | YAML validation and secret-pattern scan |
| [`graphify.yml`](.github/workflows/graphify.yml) | push to `main`, pull requests | Rebuilds the knowledge graph of the Rust crates and `testkit/` (see [GRAPHIFY.md](GRAPHIFY.md)) |

CI installs `protoc` from `protobuf-compiler`; do the same locally (or set `PROTOC`) before building.

## Run it end-to-end

```sh
gnomon-ctl &                     # gRPC on :7631; add --state-dir DIR (or GNOMON_STATE_DIR) to survive restarts
GNOMON_NODE=node0 gnomon-agent & # registers real hardware, heartbeats, executes jobs
gnbatch --name train -c 8 --gpus 2 -t 3600 train.sh arg1     # parses #SBATCH too; flags win
gnbatch -N 4 -c 16 -t 3600 mpi.sh # gang: 16 CPUs on each of 4 nodes, reserved together
gnqueue; gninfo; gncancel 3      # cancelling a running job kills its ranks; it ends CANCELLED
gntop                            # live Gantt chart of the plan (q quits)
```

The `gn*` shims connect to `GNOMON_ADDR` (default `http://127.0.0.1:7631`). Connecting and each request
give up after `GNOMON_TIMEOUT_SECS` seconds (default 10).

### Replicated controllers

Run three (or five) controllers as one Raft group. Each needs its own state directory and the same
peer list; `--listen` sets the gRPC address (default `0.0.0.0:7631`):

```sh
PEERS=1=ctl1:7700,2=ctl2:7700,3=ctl3:7700
gnomon-ctl --raft-id 1 --raft-peers $PEERS --state-dir /var/lib/gnomon   # on ctl1; 2 and 3 likewise
export GNOMON_ADDR=http://ctl1:7631,http://ctl2:7631,http://ctl3:7631    # agents and gn* shims
```

Any controller accepts requests: followers forward writes to the leader and answer reads from their
own (caught-up) state. Only the leader schedules; a newly elected leader re-plans everything pending
on its next 1 s backfill pass. With no quorum, writes fail with `UNAVAILABLE` instead of being
accepted and lost. Agents move to the next controller in `GNOMON_ADDR` on any error, re-register,
and retry completion reports until one lands. Membership is static (the `--raft-peers` list). A
forwarded write is retried only when it provably was not sent; a leader that steps down mid-write can
still make one event apply twice, which every controller event tolerates (duplicates are no-ops or
rejected; fetched work is drained by job and rank). See `crates/consensus/src/replica.rs`.

## Node agent runtime: cgroups and containers

Each job runs as `sh -c <command>` in the agent's working directory (`GNOMON_WORKDIR` overrides it)
with `GNOMON_JOB_ID`, `GNOMON_JOB_NAME`, `GNOMON_CPUS`, `GNOMON_MEM_MB`, `GNOMON_GPUS`,
`GNOMON_TIME_LIMIT`, `GNOMON_NODELIST` (comma-separated), `GNOMON_NODE_RANK`, `GNOMON_NNODES` and
`GNOMON_JOB_START` (the gang's common planned start).

**Gang release barrier.** Agents fetch a gang's ranks on independent polls, so a rank could start up to
one poll interval (`GNOMON_HEARTBEAT_SECS`) before its peers. Instead each rank waits at a barrier:
fetching a rank is recorded by the replicated `AssignmentsFetched` event, the controller's `GangReady`
RPC reports when every rank has been fetched, and waiting ranks poll it every 200 ms and then start.
Measured with two agents polling 1.5 s out of phase: ranks started 0.14 s apart. A rank whose job is
no longer running is not started; one whose peers never arrive gives up at its walltime (exit 125).
At the walltime it is killed and reported as exit 124, like `timeout(1)`.

**cgroup v2 confinement.** When the agent can write a cgroup2 hierarchy offering the `cpu` and `memory`
controllers, every job rank gets `<root>/gnomon/job_<id>_r<rank>` (so gang ranks stay separate when
several agents share a host) with `cpu.max` = cores × 100 ms per 100 ms,
`memory.max` = the requested memory (if any) and `memory.swap.max` = 0. The child joins the cgroup
itself before `exec`, so no job process ever runs unconfined. The walltime kill uses `cgroup.kill`
(Linux 5.14+) to kill the whole process tree. On exit the agent prints the job's CPU time and peak
memory (`cpu.stat`, `memory.peak`) and removes the cgroup. If confinement is available but a job's
cgroup cannot be created, that job fails (exit -1) rather than running without its limits.

| Variable | Default | Meaning |
|---|---|---|
| `GNOMON_CGROUP_ROOT` | `/sys/fs/cgroup` | cgroup2 root, or a cgroup delegated to the agent's user that holds no processes itself |
| `GNOMON_CGROUP` | on | `off` disables confinement |

Without cgroup2 (a v1 host, no delegation, missing controllers) the agent logs the reason once at
start-up and runs jobs unconfined, each in its own process group so the walltime and kill paths still
reach the job's children (a process that `setsid`s away escapes; only a cgroup catches it).

**Kill channel.** Every `FetchWork` reply also lists the jobs whose rank on this node must stop: a
gang peer exited non-zero, or the job was cancelled while running. The agent stops the rank exactly
as at the walltime (`cgroup.kill`, or `kill`/`rm -f` of the container) and reports exit 143; a rank
still waiting at the gang barrier is released and reported without starting, and a listed job the
agent never received is reported as killed, so the gang always completes. The controller repeats
the list until each rank has reported, and keeps the nodes booked until then.

**Lost nodes.** The leader declares a node lost after `GNOMON_NODE_TIMEOUT_SECS` without a
heartbeat (default three heartbeat intervals, at least 30 s; counted from when it became leader): a
replicated `NodeLost` event takes the node out of placement, ends its running ranks (a gang's peers
are stopped through the kill channel), and returns unfetched work and reservations on it to
`Pending`. A heartbeat or registration brings the node back; an agent that was only partitioned
away gets the failed ranks on its kill list and stops them before it takes new work, and its late
reports change nothing. A loss decision carries the heartbeat it saw and is dropped if the node
heartbeated meanwhile.

**Validating a host.** `testkit/validate-host.sh [cgroup|container]` (root; re-runs itself under
`sudo`) checks all of the above against the real kernel and runtime: the job sits in its cgroup from
its first instruction with the requested `cpu.max`/`memory.max`/`memory.swap.max`, a memory hog is
OOM-killed within its limit, a busy loop on every CPU is held to the requested cores, `cgroup.kill`
reaches `setsid` escapees, and containers get the environment, user, CPU/memory limits and walltime
or kill stop of the container itself. CI runs it on Ubuntu 24.04 with Docker and with Podman.

**Privilege separation.** Like slurmd, an agent running as **root** runs every job as the user who
submitted it: the job's `user` is resolved to a local account (`getpwnam`/`getgrouplist`, so NSS/LDAP
accounts work) in the agent, and the child joins its cgroup and then drops to that user's groups, gid
and uid before `exec`. The job therefore cannot change its own cgroup limits, which stay root-owned. A
job whose user has no local account, or is `root`, is refused (exit -1): jobs never run as root.
Container jobs under a root agent get `--user <job uid>:<job gid>`. The job user needs read access to
the job script and the working directory (`GNOMON_WORKDIR`, ideally a shared filesystem).

An **unprivileged** agent cannot switch users, so its jobs run as the agent's service user. That also
works with cgroups through a delegated subtree: a systemd unit with `Delegate=yes` whose delegated
subtree contains both the agent's own cgroup and `GNOMON_CGROUP_ROOT` (moving a process requires write
access to `cgroup.procs` of the common ancestor; the root itself must hold no processes).

**Rootless containers.** A job submitted with an image (`gnbatch --container IMAGE`, or
`#SBATCH --container=IMAGE` / pyxis's `--container-image=IMAGE`) runs as

```text
docker run --rm --name gnomon-job-<id>-r<rank> --cpus <cores> --memory <mem>m [--gpus <n>] --network host \
  --user <identity> -v <workdir>:<workdir> -w <workdir> -e GNOMON_...=... \
  --entrypoint sh <image> -c <command>
```

The runtime enforces the limits, so the agent does not also put the job in a gnomon cgroup. The
walltime kill runs `docker kill gnomon-job-<id>-r<rank>`, which stops the container, not just the
client; each call is time-bounded and its exit status checked, with a retry and `docker rm -f` as the
fallback (a failure to confirm the stop is logged loudly).
Scripts given by absolute path must live under the working directory (ideally a shared
filesystem), since only that directory is mounted.

`GNOMON_CONTAINER_RUNTIME` selects the binary (default `docker`). With `podman` the agent adds
`--userns=keep-id` and leaves out `--gpus`, which Podman handles through CDI devices. Rootless
Docker setup for the agent's user:

```sh
dockerd-rootless-setuptool.sh install          # from docker-ce-rootless-extras; needs uidmap
systemctl --user enable --now docker
export DOCKER_HOST=unix://$XDG_RUNTIME_DIR/docker.sock   # in the agent's environment
loginctl enable-linger $USER                    # keep the daemon running without a login session
```

`--cpus`/`--memory` need cgroup v2 with the `cpu` and `memory` controllers delegated to the user
(`systemctl edit user@.service` with `Delegate=cpu cpuset io memory pids` on older distributions).
The `--user` identity is chosen so the job can write the agent's working directory and owns its
output, without ever being host root:

| Runtime | `--user` | Why |
|---|---|---|
| Rootless Docker | `0:0` | Container root *is* the agent's user on the host; any other uid maps to a subordinate uid that cannot write an agent-owned directory |
| Rootful Docker | agent's `uid:gid`; the **job user's** under a root agent | Container root would be host root |
| Podman | agent's `uid:gid` + `--userns=keep-id` | `keep-id` maps the agent's uid to itself |

Rootless Docker is detected once at start-up from `docker info`; `GNOMON_CONTAINER_ROOTLESS=1`/`0`
overrides the detection.

### `gntop`: the time axis, live

Gnomon schedules on the time axis (each vertex's planner holds running spans and future backfill
reservations), and `gntop` draws it. It polls `ListJobs` and `ListNodes`, then shows one row per
node with its capacity, allocation and liveness in a left gutter. Running spans are solid (`█`) and
reservations are hatched (`░`); both are labelled `<id> <name>`, and overlapping spans on one node
stack onto extra lanes. A red `│` cursor marks *now*. Planned jobs whose nodes are unknown go to a
`pending` lane. Below the chart is the pending queue (priority, wait time). The header shows the
counts per state and the API state index. If the controller goes away, the header says
`disconnected, retrying`, the last plan stays on screen marked stale, and polling reconnects by
itself.

| Key | Action |
|---|---|
| `q`, `Esc`, `Ctrl-C` | quit (the terminal is restored, also on panic) |
| `p` | pause / resume polling |
| `+` / `-` | zoom in / out (halve / double the window) |
| `←` / `→` (`h` / `l`) | pan a quarter window earlier / later |
| `0` | reset zoom and pan |
| `/` | filter by user (Enter applies, empty shows all, Esc cancels) |

| Flag | Default | Meaning |
|---|---|---|
| `--addr URL` | `$GNOMON_ADDR` or `http://127.0.0.1:7631` | controller |
| `--interval D` | `1s` | poll interval (`500ms`, `5s`, ...) |
| `--history D` / `--horizon D` | `10m` / `2h` | window before / after now |
| `-u, --user NAME` | all | initial user filter |
| `--once` | off | print one plain-text frame to stdout (no alternate screen, no colours) and exit; the exit status is non-zero when the controller is unreachable, for CI smoke tests and screenshots |
| `--width N` / `--height N` | terminal size, else 120×40 | frame size for `--once` |

`gntop` lives in its own crate (`crates/top`), not in `gnomon-cli`. That keeps the ratatui/crossterm
dependency tree out of the Slurm shims, and its library target lets the pure layout code (time→column
mapping, lane stacking, zoom/pan state machine) and the full-screen render be unit-tested against
ratatui's `TestBackend` with no network.

## Roadmap status (report section 4.6)

1. [x] FSM + graph + optimistic scheduler skeleton
2. [x] openraft store (log/state machine/snapshot), durable on disk (`DiskStore`: fsync'd, checksummed
       log, atomic vote/commit/snapshot, torn-tail recovery, exact FSM rebuild); the single controller
       journals every event under `--state-dir` and recovers on restart; TCP Raft transport and
       replicated controllers (`--raft-id`), every write through `client_write`, agent/CLI failover —
       dynamic membership changes next
3. [x] gRPC API tier + Slurm-compatible CLI shims
4. [x] Agent registration/heartbeat/exec over the wire, cgroup v2 confinement, rootless containers;
       CDI injection next
5. [x] Policy-as-data + WASM-sandboxed admission, priority, estimator and node-score plugins;
       fairshare accounting history next
6. [x] Nested instances: ownership-typed `SubgraphGrant` (compile-time delegation safety);
       cross-process child instances next
7. [x] Per-vertex OCC, exact replay/shadow mode, distribution-aware planning (opt-in),
       time-varying capacity (opt-in); better runtime predictors and deferrable-job carbon
       policies next
8. [x] Multi-node gang jobs (`-N`), all-or-nothing with a common start; per-rank completion; the
       agent kill channel stops a gang's peers on the first failed rank and cancels running jobs
9. [x] Test kit side by side with Slurm (single-node, CI weekly), replay regression gate on every PR,
       coverage floor; multi-node comparison on a 3-node container cluster (CI weekly) — separate hardware next
10. [x] `gntop` live Gantt view of the plan (polling `ListJobs`/`ListNodes`); next: push updates
       through index-aware blocking queries instead of polling, a per-job detail pane, partition
       and state filters, and drawing capacity profiles (power/carbon windows) as shaded
       background
