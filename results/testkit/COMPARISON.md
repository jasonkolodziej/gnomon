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

# Multi-node comparison (containers, 3 nodes)

Raw results: [`multinode-20261117-084547/`](multinode-20261117-084547/) (per-run `summary.tsv`,
`metrics.tsv`, `metrics_extra.tsv`, every job and gang-rank `.rec`, Slurm and Gnomon daemon logs,
`comparison.md`). Produced by `testkit/multinode.sh all --burst 50`, full length (`TK_SCALE=1`):
Slurm first (08:45 UTC), then Gnomon (08:56 UTC), one after the other, never concurrently.

## What was compared

Two real multi-node clusters, built in turn on the same Docker host from the same image (Ubuntu
24.04, `slurm-wlm` 23.11.4, `munge`): a `head` container plus three node containers `node1..node3`
on one Docker network. Every node advertises 4 CPUs and 4096 MiB to its scheduler.

- **Slurm**: `slurmctld` on `head`, one `slurmd` per node container, shared munge key and
  `slurm.conf` (select/cons_tres `CR_Core_Memory`, sched/backfill `bf_interval=5`,
  proctrack/linuxproc, task/none, no accounting).
- **Gnomon**: `gnomon-ctl` on `head`, one `gnomon-agent` per node container (`GNOMON_NODE=nodeI`,
  `GNOMON_CORES=4`, `GNOMON_MEM_MB=4096`, 2 s agent poll), release build of this branch.
- The test-kit directory is bind-mounted at the same path in every container as the shared
  filesystem. `run.sh --multinode` runs inside `head` as the unprivileged user `tk`. Jobs run as
  `tk` on the nodes under both schedulers.

The suite: t01–t07 as before. In t03 (backfill) and t04 (cancel), the blocker and the wide job are
gangs over all three nodes. Four tests are new: t08 (one gang job over 3 nodes:
`gnbatch --nodes 3` vs `sbatch -N 3` + `srun`), t09 (one whole-node job per node), t10 (12
single-core jobs that fill every core) and t11 (pack or spread, informational).

## Results

| test | Gnomon | Slurm | Gnomon detail | Slurm detail |
|---|---|---|---|---|
| t01 | PASS | PASS | job completed; wait 2 s | job completed; wait 5 s |
| t02 | PASS | PASS | job sees cpus=4 (nproc=4, unconfined) | job sees cpus=4 (nproc=4, unconfined) |
| t03 | PASS | PASS | C backfilled 236 s before the 3-node gang B; D waited for B (+32 s) | C backfilled 235 s before B; D waited for B (+35 s) |
| t04 | PASS | PASS | queued 3-node gang cancelled, never ran | queued 3-node job cancelled, never ran |
| t05 | PASS | PASS | 50 jobs; submit 141.1 jobs/s; drained in 19 s | 50 jobs; submit 100.7 jobs/s; drained in 23 s |
| t06 | PASS | PASS | killed at limit (FAILED) | killed at limit (TIMEOUT) |
| t07 | PASS | PASS | exit 3 -> FAILED | exit 3 -> FAILED |
| t08 | PASS | PASS | 3 ranks on node1, node2, node3; start spread 104 ms | 3 ranks on node1, node2, node3; start spread 4 ms |
| t09 | PASS | PASS | 3 whole-node jobs on 3 distinct nodes, concurrent | same |
| t10 | PASS | PASS | 12 jobs in one wave, 4 per node, all started 2 s after the first submit | same, all started 5 s after the first submit |
| t11 | INFO | INFO | **spreads**: 3 small jobs on 3 nodes | **packs**: 3 small jobs on node1 |

| metric | Gnomon | Slurm |
|---|---|---|
| t01 queue wait (s) | 2 | 5 |
| t05 median wait (s) | 9 | 10 |
| t05 submit rate (jobs/s) | 141.1 | 100.7 |
| t05 drain time, 50 x 2 s jobs on 12 cores (s) | 19 | 23 |
| t08 gang queue wait (s) | 1 | 2 |
| t08 gang rank start spread (ms) | 104 | 4 |
| t10 time to fill all 12 cores (s) | 2 | 5 |
| t11 nodes used by 3 small jobs on an idle cluster | 3 | 1 |

Both schedulers pass all ten pass/fail tests on the multi-node shape. Differences:

- **Gang start alignment**: under Slurm, `srun` launches the three tasks from one step, and they
  start within ~4 ms of each other. Gnomon's agents fetch their ranks on independent polls and wait at
  the controller's release barrier, which they poll every 200 ms. Their ranks start within ~100 ms
  (104 ms in this run, 94 ms in a shakedown run). Both are well inside the test's 2 s bound.
- **Placement policy**: Slurm's cons_tres best-fit packs small jobs onto the lowest-numbered node.
  Gnomon spreads them across idle nodes. Neither is wrong: packing keeps whole nodes free for wide jobs,
  and spreading reduces contention between jobs. When capacity forces placement (t09, t10), both place
  identically.
- **Latency**: Gnomon's waits are bounded by its 2 s agent poll. Slurm's are bounded by its
  scheduling loop and backfill interval (5 s). This matches the single-node results above.

## Caveats

- **The "nodes" are containers on one 4-vCPU VM, not separate hardware.** All four containers share
  the same CPUs, memory, kernel, disk and clock. The 3 x 4 = 12 advertised CPUs are an
  oversubscription of 4 real vCPUs. This compares scheduling decisions, placement and control-plane
  latency over a real network hop between controller and node daemons. It does not compare compute
  performance, interconnect behaviour, or a site's tuned production configuration.
- The rank start-spread figures are trustworthy only because every container reads the same kernel
  clock. On separate machines they would include NTP skew.
- No confinement under either scheduler (`nproc=4` in t02): the host is cgroup v1 with no delegation
  into containers, so Slurm uses `task/none` and the Gnomon agents launch unconfined. The node
  containers run with `CAP_SYS_ADMIN` and no seccomp profile. slurmd needs this to clone slurmstepd,
  and both clusters get the same options.
- One run per scheduler. Other work (another engineer's builds) ran on the host at the same time, so
  treat the timings as indicative. An earlier attempt was discarded after all four Gnomon
  containers stopped at the same moment mid-t03 (Docker logged the tasks exiting; the cause was
  outside the cluster, as the controller had logged nothing). `multinode.sh all` now detects
  and reports dead containers. A host-side `pkill slurmd`/`pkill gnomon-agent`, such as
  `slurm-local.sh stop`, also kills container daemons, so do not run both kits on one host at once.
- Regenerate: `testkit/multinode.sh all` (see `testkit/README.md`, *Multi-node, side by side*). CI
  runs it weekly and on demand (`e2e.yml`, job *Gnomon vs Slurm (multi-node, containers)*).

## Multi-node rerun with the agent kill channel (2026-11-17 09:12)

Same three container nodes and kit as above, rerun after this iteration's Gnomon changes (agent kill
channel, replicated-controller support, idempotent work fetch), plus the new **t12**: a 3-node gang
whose rank 0 exits 3 after 2 s while the other ranks would sleep 300 s. Gnomon ran the full multi-node
suite; Slurm ran only t08 and t12, since its other results above are unchanged by Gnomon changes. Raw
data: [`multinode-20261117-091206-kill-channel/`](multinode-20261117-091206-kill-channel/)
(`comparison.md` there is the generated side-by-side).

| test | Gnomon | Slurm |
|---|---|---|
| t01–t10 | PASS (all ten) | (t08 PASS; others as above) |
| t08 gang rank start spread (ms) | 134 | 5 |
| t12 failed gang: peers stopped, job FAILED, submit to end (s) | PASS, 8 | PASS, 8 |

Before the kill channel, the two surviving ranks would have held their nodes for the full 300 s and the
job would have ended FAILED only then. Gnomon now matches Slurm's `srun --kill-on-bad-exit`: the
failed rank's report puts the peers on the kill list, and their agents stop them on the next poll
(2 s here). The start spread varies between runs (94–134 ms over three Gnomon runs) because it is
bounded by the 200 ms release-barrier poll.
