# 0002 — Sandboxed WASM plugins for scheduling policy

**Status**: Accepted (steps 1–4 implemented, see [Implementation](#implementation))
**Date**: 2026-11-17

## Context

Sites customize Slurm through in-process C plugins and `job_submit.lua`. Both run with full access to
`slurmctld`: a crash, leak or infinite loop in a plugin takes the controller down with it. Gnomon's
design report (section 4.5) proposes policy-as-data plus WASM-sandboxed plugins instead, and
`gnomon-policy` contains a first version, `WasmPriorityPlugin` (`crates/policy/src/lib.rs`). As it
stands, it cannot be used:

- **Not wired in.** `gnomon-ctl` does not depend on `gnomon-policy`, and every job is submitted with
  `priority: 1000` (`gnbatch`). The plugin runs only in its unit test.
- **No resource limits.** The `wasmtime::Engine` uses the default `Config`: no fuel, no epoch
  interruption, no `StoreLimits`. A plugin that loops forever blocks the calling thread forever,
  which on the controller is the scheduler thread.
- **Slow to call.** Every call creates a `Store` and an `Instance` from scratch, and modules load only
  from WAT text (`from_wat`).
- **Untyped interface.** The plugin must export `memory`, `alloc(len) -> ptr` and
  `priority(ptr, len) -> u64`, and parse the job from JSON in linear memory. Plugin authors write that
  glue by hand.

The controller has two properties that any plugin mechanism must preserve:

1. **Replicated, deterministic state** ([ADR 0001](./0001-replicated-controller-writes.md)). Every
   replica applies the same committed events and must reach the same state. Something computed
   differently on two replicas, if it reaches the FSM, makes them diverge.
2. **Exact replay.** `gnomon_sim::replay::verify_exact` rebuilds the FSM bit-identically from the
   journal, and the replay gate in CI uses the production `backfill_pass` as its regression oracle.

Priority is the first extension point: `JobSubmitted(Job)` carries `Job::priority`, which the submit
handler sets before the event is created (`crates/api/src/lib.rs`). Further candidates are
admission (Slurm's `job_submit`), runtime estimation (the estimators in `gnomon-sched`), node scoring
inside `Policy::place`, and completion/accounting.

## Options Considered

1. **Embedded scripting (Lua via `mlua`, or Rhai).** Familiar to Slurm admins (`job_submit.lua`) and
   small. But it runs in the controller's address space, has no hard memory limit, and limits CPU only
   through instruction-count hooks. Determinism depends on which standard library functions are
   exposed. One language only.
2. **Out-of-process plugins over gRPC or a Unix socket.** Strong isolation, any language. But every
   decision costs an IPC round trip on the scheduling path. Plugin processes need supervision,
   timeouts and their own deployment. Nothing structurally stops a plugin from reading a clock or the
   network, so determinism becomes a convention.
3. **WASM core modules with the current pointer/JSON interface, hardened.** Add fuel and memory
   limits and pre-link the module. Small change, but plugins still hand-write `alloc` and JSON
   parsing, the interface is unversioned, and each new hook means another hand-rolled interface.
4. **WASM components with a WIT interface (wasmtime component model), hardened and deterministic.**
   The host exposes typed WIT worlds (`priority`, later `admission`, `estimator`, ...). Plugins compile
   from Rust (`cargo component`), Go (TinyGo), JS (`jco`) or Python (`componentize-py`). The sandbox
   gives no imports by default, hard fuel and memory limits, and a cheap instantiation per call from
   an `InstancePre`. Costs: the component model's toolchain is younger than core WASM, and wasmtime
   is already a large dependency (36.x).

## Decision

Option 4, built in steps, with option 3's hardening done first because it closes the denial-of-service
gap independently of the interface.

**Rule 1: plugins run before an event, never during apply.** A plugin runs where a write is prepared:
the API handler, or the leader's scheduling loop. Its result goes into the event (`Job::priority` in
`JobSubmitted`) or into an explicit event of its own. Replicas and replay apply the recorded value and
never run the plugin. ADR 0001's guarantees therefore hold whatever the plugin does. Values that
change over time (for example an age factor) get an explicit `PriorityUpdated` event from the leader,
or are computed from the snapshot and the injected clock inside the plan, which the log already
carries.

**Rule 2: plugins are pure functions.** They link against no WASI and no host imports: no clock
(`now` is a parameter), no randomness, no I/O. The engine is configured for reproducible results:
`cranelift_nan_canonicalization(true)`, threads and relaxed SIMD off. So a plugin given the same input
gives the same output on x86_64 and aarch64, which `gnomon-sim` relies on to evaluate a plugin
offline.

**Rule 3: bounded cost, safe fallback.** Every call has a fuel budget (`Config::consume_fuel`) and a
`StoreLimits` memory cap, both configurable. Out of fuel, out of memory, a trap or a load error falls
back to the declarative `PolicyConfig` result and is logged and counted. A plugin can make decisions
worse; it cannot stop scheduling.

**Interface** (first world, versioned by package):

```wit
package gnomon:policy@0.1.0;

interface types {
  record job {
    id: u64, user: string, partition: string,
    cores: u32, mem-mb: u64, gpus: u32, nodes: u32,
    walltime-secs: u64, submit-time: u64,
  }
}

world priority {
  use types.{job};
  export priority: func(job: job, now: u64) -> u64;
}
```

**Steps:**

1. **Harden the host** (`gnomon-policy`): engine `Config` per Rule 2 and 3, `InstancePre` built once,
   load `.wasm` from bytes or a path, a fallback path, and tests for an infinite loop, a memory bomb
   and a trap.
2. **Wire it in**: `gnomon-ctl --priority-plugin PATH` (`GNOMON_PRIORITY_PLUGIN`). The submit handler
   computes `Job::priority` through the plugin, falling back to `PolicyConfig`, before building
   `JobSubmitted`. A gauge counts plugin failures.
3. **WIT world**: replace the pointer/JSON interface with the `priority` world via
   `wasmtime::component::bindgen!`. Ship an example plugin in Rust, built in CI.
4. **Offline evaluation**: `experiments --priority-plugin candidate.wasm` replays SWF traces under a
   candidate plugin, and `ShadowPolicy` runs it beside the live one, so a plugin can be judged on real
   workloads before deployment.
5. **Further worlds**, each its own follow-up ADR if the design differs: `admission` (accept, reject
   with a reason, or rewrite the request; the Slurm `job_submit` equivalent), `estimator` (runtime
   prediction), then `node-score`, which runs per candidate node on the hot path and must be
   benchmarked with `occ_bench`.

**Out of scope here**: running user *jobs* as WASM on the agent (a third launch mode beside cgroups
and containers, via `wasmtime-wasi` with preopens limited to `GNOMON_WORKDIR`). It is a different
trust boundary (untrusted user code, not admin policy) and needs WASI, which Rule 2 excludes for
controller plugins. It gets its own ADR.

## Consequences

- **Safety**: a faulty or malicious policy plugin cannot crash, hang or exhaust the controller. The
  worst case is fallback priorities plus a counted error.
- **Determinism kept**: since plugins run only before an event, replication and exact replay are
  unaffected, and the replay gate keeps working without plugins in its path. A plugin can still give
  different results across *versions*. The recorded value in the log, not the plugin, is the source
  of truth.
- **Expressiveness limits**: plugins see only the fields passed in. Fairshare usage history or
  cluster load must become explicit parameters (or host functions that read the snapshot), which
  this ADR does not add yet. Pure functions cannot call external services such as an accounting
  database or LDAP. That is deliberate and should be documented.
- **Performance**: instantiating from an `InstancePre` costs microseconds, which is fine at submit
  time. Hooks inside `Policy::place` are not: they run per job per candidate node on the hot path, and
  need a measured budget before being accepted.
- **Dependencies and toolchain**: the component model adds `wasmtime` features (`component-model`) and
  a plugin toolchain (`cargo component` or equivalents). WIT package versions become a compatibility
  surface: breaking changes need a new major version, and the host should support at least the
  previous one.
- **Operations**: plugin path, fuel budget and memory cap become controller configuration, and belong
  in `docs/INSTALL.md` once step 2 lands. Every replica must load the same plugin: the submit handler
  runs on whichever replica receives the request (followers forward only the finished event), so
  different plugins give different priorities depending on which replica a client reached.
- **Follow-ups**: separate ADRs for `admission` (if it can rewrite jobs) and for WASM job execution
  on the agent.

## Implementation

Steps 1–4 are implemented. Where the code differs from the text above:

- **Steps 1 and 3 together.** The pointer/JSON core-module interface was removed rather than hardened
  first: `PriorityPlugin` (`crates/policy/src/plugin.rs`) loads components for the WIT world in
  `crates/policy/wit/priority.wit` through `bindgen!`, pre-links them into an `InstancePre`, and runs
  each call in a fresh `Store` with fuel and a `StoreLimits` memory cap (`trap_on_grow_failure`, so a
  memory bomb traps instead of carrying on). The linker defines only the types-only `types`
  interface, so a component importing anything else (WASI included) fails to load.
  `crates/policy/tests/plugin.rs` covers an infinite loop, memory growth past the cap, oversized
  initial memory, a trap, an impure import and a core module, all with hand-written component WAT
  (`gnomon_policy::fixtures`) so the tests need no WASM toolchain.
- **Fallback and counters.** `PriorityHook` pairs an optional plugin with `PolicyConfig` and counts
  calls and failures (`AtomicU64`s; the controller has no metrics endpoint yet, so failures are
  logged with the running count rather than exported as a gauge).
- **Wiring keeps wasmtime out of the API client.** `gnomon-api` defines a `SubmitPriority` trait and
  `SchedulerSvc::priority: Option<Arc<dyn SubmitPriority>>`. `gnomon-ctl` implements it from a
  `PriorityHook`, so `gnomon-agent`, `gnomon-cli` and `gnomon-top` (which depend on `gnomon-api`) do
  not link wasmtime. The priority is computed after the job id is chosen and recorded in
  `JobSubmitted`. Without `--priority-plugin` or `--policy-config` the client's priority is kept, as
  before. Flags: `--priority-plugin`, `--policy-config`, `--plugin-fuel`, `--plugin-memory-mb`
  (with `GNOMON_*` equivalents, see `docs/INSTALL.md`). A plugin that does not load stops startup.
- **Example plugin.** `examples/priority-plugin` (outside the workspace) is built in the `Rust`
  workflow's `wasm-plugin` job for `wasm32-unknown-unknown` (which links no WASI) and wrapped with
  `wasm-tools component new`; `crates/policy/tests/example_plugin.rs` then loads it in the host.
- **Offline evaluation without `ShadowPolicy`.** `ShadowPolicy` compares *placements* for the same
  job, which a priority function does not change; it changes queue *order*. Step 4 is instead
  `experiments --priority-plugin` (experiment E4): each SWF window is scheduled under FCFS and under
  the plugin's submit-time priorities (`gnomon_sim::priority::assign`), and
  `gnomon_sim::priority::fcfs_agreement` reports the fraction of job pairs left in submit order.
  A live priority shadow (compute the candidate beside the deployed plugin and log the difference)
  is left for later.
- **Step 5** (further worlds) is not started.
