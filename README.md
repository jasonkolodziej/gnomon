<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo/svg/gnomon-lockup-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="assets/logo/svg/gnomon-lockup-light.svg">
    <img src="assets/logo/svg/gnomon-lockup-light.svg" alt="Gnomon" height="72">
  </picture>
</p>

# Gnomon

[![Rust](https://github.com/jasonkolodziej/gnomon/actions/workflows/rust.yml/badge.svg)](https://github.com/jasonkolodziej/gnomon/actions/workflows/rust.yml)
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
`gncancel`, `gninfo`.

A research codebase implementing the Rust-native HPC scheduler architecture
proposed in [`docs/SLURM_Scheduler_Research_and_Rust_Design.md`](docs/SLURM_Scheduler_Research_and_Rust_Design.md).

## Crates

| Crate | Role | Report section |
|---|---|---|
| `gnomon-core` | Job/resource/event types, serde-serializable for the Raft log | 4.1, 4.5 |
| `gnomon-graph` | Resource graph, per-vertex Planners (O(S log S) sweep), **per-vertex versions**, **time-varying capacity profiles**, **ownership-typed subgraph grants** | 4.2, 4.4 |
| `gnomon-fsm` | Replicated state machine, snapshots, injected clock, **event journal**, node registry, work assignments | 4.1 |
| `gnomon-sched` | Coalescing eval broker, parallel workers, **per-vertex optimistic plan applier**, `backfill_pass`, **runtime-quantile estimator**, **shadow mode** | 4.3 |
| `gnomon-policy` | **Policy-as-data** (fairshare/QoS/limits JSON) + **wasmtime-sandboxed priority plugins** | 4.5 |
| `gnomon-consensus` | **openraft** log/state-machine store bridging Raft to the FSM | 4.1 |
| `gnomon-api` | **tonic gRPC** service: lock-free snapshot reads, index-aware blocking queries | 4.3 |
| `gnomon-ctl` | Controller daemon (gRPC server + background scheduler loop) | 4.6 |
| `gnomon-agent` | Node agent: real /proc + GPU probing, registration, heartbeats, job launch in cgroup v2 or rootless containers | 4.6 |
| `gnomon-cli` | **Slurm-compatible shims**: `gnbatch` (parses #SBATCH), `gnqueue`, `gncancel`, `gninfo` | 4.5 |
| `gnomon-sim` | **Exact replay**, counterfactual policy evaluation on recorded history, SWF trace simulator, metrics | 6.3 |

## What's new in this iteration

Evaluated on real Parallel Workloads Archive traces — full write-up in
[`results/EVALUATION.md`](results/EVALUATION.md).

| Change | Default | Result |
|---|---|---|
| **Per-vertex optimistic concurrency.** A plan conflicts only if a vertex it uses changed *and* it no longer fits (was: any event, even a heartbeat). Also fixes rejected plans counted as committed and double placement of one job. | On | 92–100% fewer conflicts, 1.15–1.99× controller throughput |
| **Exact replay + counterfactuals + shadow mode.** Journal → bit-identical state; recovered history re-scheduled under any policy with the production code; candidate policies run alongside the live one. | Tooling | 5,000-job trace: replay exact, same policy reproduces every start time |
| **Distribution-aware backfill.** Spans planned at a learned runtime quantile (risk budget); overruns extended, walltime still the kill limit. | Opt-in | Oracle bound: −16% wait, −29% slowdown. Built-in predictor: inconsistent (median −3% wait at θ=0.7, range −15…+21%), so walltime stays default |
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
cargo test --workspace           # 22 tests: planner (incl. brute-force oracle), OCC, replay, estimator, profiles, policy, WASM, delegation
cargo run --release -p gnomon-sched --bin bench       # planning-core A/B benchmark
cargo run --release -p gnomon-sched --bin occ_bench   # controller-path concurrency benchmark
cargo run --release -p gnomon-sim --bin experiments -- <swf-dir> results 5000   # trace evaluation
```

## Test kit (Gnomon vs Slurm)

`testkit/` runs seven scheduler tests (smoke, resources, backfill, cancel, throughput, walltime,
exit codes) unchanged against Gnomon or a Slurm cluster and compares the two — see
[`testkit/README.md`](testkit/README.md).

## Continuous integration

| Workflow | Runs | Checks |
|---|---|---|
| [`rust.yml`](.github/workflows/rust.yml) | push to `main`, pull requests | `cargo fmt --check`, `cargo clippy -D warnings`, build + `cargo test`, `cargo doc` with `-D warnings` |
| [`audit.yml`](.github/workflows/audit.yml) | `Cargo.toml`/`Cargo.lock` changes, weekly | `cargo audit` against the RustSec advisory database |
| [`ci.yml`](.github/workflows/ci.yml) | push to `main`, pull requests | YAML validation and secret-pattern scan |
| [`graphify.yml`](.github/workflows/graphify.yml) | push to `main`, pull requests | Rebuilds the knowledge graph of the Rust crates and `testkit/` (see [GRAPHIFY.md](GRAPHIFY.md)) |

CI installs `protoc` from `protobuf-compiler`; do the same locally (or set `PROTOC`) before building.

## Run it end-to-end

```sh
gnomon-ctl &                     # gRPC on :7631
GNOMON_NODE=node0 gnomon-agent & # registers real hardware, heartbeats, executes jobs
gnbatch --name train -c 8 --gpus 2 -t 3600 train.sh arg1     # parses #SBATCH too; flags win
gnqueue; gninfo; gncancel 3
```

## Node agent runtime: cgroups and containers

Each job runs as `sh -c <command>` in the agent's working directory (`GNOMON_WORKDIR` overrides it)
with `GNOMON_JOB_ID`, `GNOMON_JOB_NAME`, `GNOMON_CPUS`, `GNOMON_MEM_MB`, `GNOMON_GPUS`,
`GNOMON_TIME_LIMIT`, `GNOMON_NODELIST` (comma-separated), `GNOMON_NODE_RANK` and `GNOMON_NNODES`.
At the walltime it is killed and reported as exit 124, like `timeout(1)`.

**cgroup v2 confinement.** When the agent can write a cgroup2 hierarchy offering the `cpu` and `memory`
controllers, every job gets `<root>/gnomon/job_<id>` with `cpu.max` = cores × 100 ms per 100 ms,
`memory.max` = the requested memory (if any) and `memory.swap.max` = 0. The child joins the cgroup
itself before `exec`, so no job process ever runs unconfined. The walltime kill uses `cgroup.kill`
(Linux 5.14+) to kill the whole process tree. On exit the agent prints the job's CPU time and peak
memory (`cpu.stat`, `memory.peak`) and removes the cgroup.

| Variable | Default | Meaning |
|---|---|---|
| `GNOMON_CGROUP_ROOT` | `/sys/fs/cgroup` | cgroup2 root, or a cgroup delegated to the agent's user that holds no processes itself |
| `GNOMON_CGROUP` | on | `off` disables confinement |

Without cgroup2 (a v1 host, no delegation, missing controllers) the agent logs the reason once at
start-up and runs jobs unconfined. Running the agent as root (as slurmd runs) needs no setup. An
unprivileged agent needs a delegated subtree (a systemd unit with `Delegate=yes`) that contains both
the agent's own cgroup and `GNOMON_CGROUP_ROOT`, because moving a process requires write access to
`cgroup.procs` of the common ancestor; the root itself must hold no processes.

**Rootless containers.** A job submitted with an image (`gnbatch --container IMAGE`, or
`#SBATCH --container=IMAGE` / pyxis's `--container-image=IMAGE`) runs as

```text
docker run --rm --name gnomon-job-<id> --cpus <cores> --memory <mem>m [--gpus <n>] --network host \
  --user <uid>:<gid> -v <workdir>:<workdir> -w <workdir> -e GNOMON_...=... \
  --entrypoint sh <image> -c <command>
```

The runtime enforces the limits, so the agent does not also put the job in a gnomon cgroup. The
walltime kill runs `docker kill gnomon-job-<id>`, which stops the container, not just the client.
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
Under rootless Docker, container root maps to the agent's user, while `--user <uid>:<gid>` maps to a
subordinate uid, so files the job writes to the bind mount belong to that subuid. Use Podman with
`--userns=keep-id` (automatic here) when output ownership matters.

## Roadmap status (report section 4.6)

1. [x] FSM + graph + optimistic scheduler skeleton
2. [x] openraft store (log/state machine/snapshot); in-memory log — persistence + network layer next
3. [x] gRPC API tier + Slurm-compatible CLI shims
4. [x] Agent registration/heartbeat/exec over the wire, cgroup v2 confinement, rootless containers;
       CDI injection next
5. [x] Policy-as-data + WASM-sandboxed priority plugins; fairshare accounting history next
6. [x] Nested instances: ownership-typed `SubgraphGrant` (compile-time delegation safety);
       cross-process child instances next
7. [x] Per-vertex OCC, exact replay/shadow mode, distribution-aware planning (opt-in),
       time-varying capacity (opt-in); better runtime predictors and deferrable-job carbon
       policies next
