# 0003 — Admission plugins that can reject or rewrite jobs

**Status**: Accepted
**Date**: 2026-11-17

## Context

[ADR 0002](./0002-wasm-policy-plugins.md) introduced sandboxed WASM policy plugins and listed
`admission`, the equivalent of Slurm's `job_submit` plugin, as a further world. It asked for a separate
ADR if admission can rewrite jobs. Sites use `job_submit.lua` to do exactly that: reject
requests that break local rules, and rewrite others (route GPU jobs to a GPU partition, cap the
walltime of a debug partition) rather than bounce them.

Rewriting raises questions that a priority plugin does not:

- **What may change.** A rewrite that changes the job's id, owner or submit time would break
  accounting and the FSM's id allocation.
- **What a failure means.** A failed priority call has a natural fallback (the declarative priority).
  A failed admission call has two plausible ones, and which is right depends on the site.
- **Where the result lives.** If replicas or replay ran admission, a plugin change would change
  history.

## Options Considered

1. **Accept or reject only (a predicate).** Simple and safe, but sites would have to bounce jobs
   they now fix silently, and the declarative `PolicyConfig` already covers simple limits.
2. **Return a whole new job.** Maximal, but it lets a plugin change identity fields, so the host
   would need to validate and partly ignore its output anyway.
3. **A verdict: accept, reject with a reason, or rewrite the request.** The plugin sees the whole job
   but can only replace its *request* (partition, per-node resources, node count, walltime).

## Decision

Option 3, in the WIT package `gnomon:policy@0.1.0` (`crates/policy/wit/policy.wit`):

```wit
record request { partition: string, cores: u32, mem-mb: u64, gpus: u32, nodes: u32, walltime-secs: u64 }
variant verdict { accept, reject(string), rewrite(request) }
world admission {
  use types.{job, verdict};
  export admit: func(job: job, now: u64) -> verdict;
}
```

- **Before the event, like priority.** The submit handler runs admission first, then priority, so
  the priority sees the rewritten request. The `JobSubmitted` event records the prepared job.
  Replicas and replay never run admission, so ADR 0001's determinism holds.
- **Validated rewrites.** The host rejects a rewrite with an empty partition, zero nodes, zero
  walltime or an all-zero request, treating it as a plugin failure. Identity fields cannot be
  expressed in a `request` at all.
- **Rejections reach the user.** A rejected submission returns `PERMISSION_DENIED` with
  `rejected by site policy: <reason>`.
- **Configurable failure mode.** `--admission-fail open` (default) accepts the job unchanged when
  the plugin fails (fuel, memory, trap, invalid rewrite). `closed` rejects it with "admission
  policy unavailable", for sites where admission enforces a hard rule (an allocation check, say).
  Either way the failure is counted and logged.
- **API shape.** `gnomon-api`'s per-submission hook becomes `SubmitPolicy::prepare(&mut Job, now)
  -> Result<(), String>`, replacing ADR 0002's `SubmitPriority`. `gnomon-ctl` chains admission,
  priority and the shadow priority comparison behind it.

## Consequences

- **The default stays fail-open.** This keeps ADR 0002's rule that a plugin cannot stop scheduling. A
  site that picks `closed` accepts that a broken plugin blocks submissions, and should alert on the
  logged failures.
- **Admission is pure, like every world.** Rules that need state (a user's running jobs, their
  usage) cannot be expressed until the snapshot is exposed through parameters or host functions
  (ADR 0002, Consequences). The declarative `PolicyConfig::admits` limits remain the place for
  usage-based checks.
- **Offline evaluation.** `experiments --admission-plugin` (E4) applies admission at each job's submit
  time (`gnomon_sim::submit::admit`) and reports rejected and rewritten counts beside the schedule
  metrics. A rewrite that shortens the walltime also shortens the simulated runtime, since the
  walltime is the kill limit.
- **The example** (`examples/policy-plugins`, feature `admission`) rejects walltimes over 7 days,
  caps `debug` jobs at one hour and one node, and moves GPU jobs from `normal` to `gpu`.
