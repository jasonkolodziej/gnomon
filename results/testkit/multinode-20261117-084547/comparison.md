# Test-kit comparison

- A: `gnomon-mn-20261117-085617` (backend=gnomon started=2026-11-17T08:56:17+00:00 host=head scale=1 nodes=3 cores/node=4)
- B: `slurm-mn-20261117-084552` (backend=slurm started=2026-11-17T08:45:52+00:00 host=head scale=1 nodes=3 cores/node=4)

| test | A result | B result | A detail | B detail |
|---|---|---|---|---|
| t01 | PASS | PASS | job 0 completed; wait 2s | job 1 completed; wait 5s |
| t02 | PASS | PASS | job sees cpus=4 (nproc=4; equals 4 only with cgroup confinement) | job sees cpus=4 (nproc=4; equals 4 only with cgroup confinement) |
| t03 | PASS | PASS | C backfilled 236s before wide B; D waited for B (+32s) | C backfilled 235s before wide B; D waited for B (+35s) |
| t04 | PASS | PASS | queued job 7 cancelled, never ran | queued job 8 cancelled, never ran |
| t05 | PASS | PASS | 50 jobs; submit 141.1 jobs/s; drained in 19s | 50 jobs; submit 100.7 jobs/s; drained in 23s |
| t06 | PASS | PASS | killed at limit (state FAILED) | killed at limit (state TIMEOUT) |
| t07 | PASS | PASS | exit 3 -> FAILED | exit 3 -> FAILED |
| t08 | PASS | PASS | 3 ranks on 3 distinct nodes (node1:1 node2:1 node3:1); start spread 104 ms | 3 ranks on 3 distinct nodes (node1:1 node2:1 node3:1); start spread 4 ms |
| t09 | PASS | PASS | 3 whole-node jobs on 3 distinct nodes, all running together | 3 whole-node jobs on 3 distinct nodes, all running together |
| t10 | PASS | PASS | 12 jobs in one wave, all started 2s after the first submit (node1:4 node2:4 node3:4) | 12 jobs in one wave, all started 5s after the first submit (node1:4 node2:4 node3:4) |
| t11 | INFO | INFO | spreads: 3 1-core jobs on 3 nodes (node1:1 node2:1 node3:1) | packs: 3 1-core jobs on one node (node1:3) |

| metric | A | B |
|---|---|---|
| t01 queue wait (s) | 2 | 5 |
| t05 median wait (s) | 9 | 10 |
| t05 submit rate (jobs/s) | 141.1 | 100.7 |
| t05 drain time (s) | 19 | 23 |
| t08 gang queue wait (s) | 1 | 2 |
| t09 whole-node median wait (s) | 0 | 0 |
| t10 fill median wait (s) | 1 | 5 |
| t08 gang rank start spread (ms) | 104 | 4 |
| t10 time to fill every core (s) | 2 | 5 |
| t11 nodes used by N small jobs | 3 | 1 |

Waits include scheduler poll intervals (Gnomon agent heartbeat, Slurm sched/backfill intervals) and, on a shared Slurm cluster, other users' load.
