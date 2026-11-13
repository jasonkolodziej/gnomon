# Test-kit comparison

- A: `gnomon-20261113-224712` (backend=gnomon started=2026-11-13T22:47:12+00:00 host=vm scale=1)
- B: `slurm-20261113-223034` (backend=slurm started=2026-11-13T22:30:34+00:00 host=vm scale=1)

| test | A result | B result | A detail | B detail |
|---|---|---|---|---|
| t01 | PASS | PASS | job 0 completed; wait 1s | job 3 completed; wait 1s |
| t02 | PASS | PASS | job sees cpus=4 (nproc=4; equals 4 only with cgroup confinement) | job sees cpus=4 (nproc=4; equals 4 only with cgroup confinement) |
| t03 | PASS | PASS | C backfilled 237s before wide B; D waited for B (+32s) | C backfilled 234s before wide B; D waited for B (+31s) |
| t04 | PASS | PASS | queued job 7 cancelled, never ran | queued job 10 cancelled, never ran |
| t05 | PASS | PASS | 50 jobs; submit 119.7 jobs/s; drained in 27s | 50 jobs; submit 42.7 jobs/s; drained in 37s |
| t06 | PASS | PASS | killed at limit (state FAILED) | killed at limit (state TIMEOUT) |
| t07 | PASS | PASS | exit 3 -> FAILED | exit 3 -> FAILED |

| metric | A | B |
|---|---|---|
| t01 queue wait (s) | 1 | 1 |
| t05 median wait (s) | 13 | 19 |
| t05 submit rate (jobs/s) | 119.7 | 42.7 |
| t05 drain time (s) | 27 | 37 |

Waits include scheduler poll intervals (Gnomon agent heartbeat, Slurm sched/backfill intervals) and, on a shared Slurm cluster, other users' load.

## Setup and caveats

- Same host (4 vCPU cloud VM, cgroup v1), run one after the other at full length (`TK_SCALE=1`).
  Both advertise one node with 8 CPUs and 16 GiB (`GNOMON_CORES=8`, `SLURM_CORES=8`).
- Gnomon: this branch's release build, `testkit/local-cluster.sh` (controller + 1 agent, 2 s agent poll).
- Slurm 23.11.4 (Ubuntu `slurm-wlm`), `testkit/slurm-local.sh`: sched/backfill with `bf_interval=5`,
  select/cons_tres, no cgroup or accounting plugins.
- Neither confines CPUs here (`nproc=4` in t02): the host has no cgroup v2, so Gnomon's agent falls back
  to unconfined launches, and the Slurm config uses `task/none`.
- A one-node Slurm on a VM is not a production cluster: this compares scheduler behaviour and latency,
  not scale, multi-node placement or a site's tuned configuration. Other jobs were compiling on the
  host during both runs, so treat timings as indicative.
- Regenerate: see `testkit/README.md` (*Side by side on one machine*); CI runs it weekly (`e2e.yml`).
