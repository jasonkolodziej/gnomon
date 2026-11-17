# Test-kit comparison

- A: `gnomon-mn-gnomon-20261117-091206` (backend=gnomon started=2026-11-17T09:12:06+00:00 host=head scale=1 nodes=3 cores/node=4)
- B: `slurm-mn-slurm-20261117-092152` (backend=slurm started=2026-11-17T09:21:52+00:00 host=head scale=1 nodes=3 cores/node=4)

| test | A result | B result | A detail | B detail |
|---|---|---|---|---|
| t01 | PASS | — | job 0 completed; wait 2s |  |
| t02 | PASS | — | job sees cpus=4 (nproc=4; equals 4 only with cgroup confinement) |  |
| t03 | PASS | — | C backfilled 237s before wide B; D waited for B (+32s) |  |
| t04 | PASS | — | queued job 7 cancelled, never ran |  |
| t05 | PASS | — | 50 jobs; submit 134.5 jobs/s; drained in 19s |  |
| t06 | PASS | — | killed at limit (state FAILED) |  |
| t07 | PASS | — | exit 3 -> FAILED |  |
| t08 | PASS | PASS | 3 ranks on 3 distinct nodes (node1:1 node2:1 node3:1); start spread 134 ms | 3 ranks on 3 distinct nodes (node1:1 node2:1 node3:1); start spread 5 ms |
| t09 | PASS | — | 3 whole-node jobs on 3 distinct nodes, all running together |  |
| t10 | PASS | — | 12 jobs in one wave, all started 2s after the first submit (node1:4 node2:4 node3:4) |  |
| t11 | INFO | — | spreads: 3 1-core jobs on 3 nodes (node1:1 node2:1 node3:1) |  |
| t12 | PASS | PASS | rank 0 exited 3 after 2 s; 2 peers stopped, job FAILED 8s after submit | rank 0 exited 3 after 2 s; 2 peers stopped, job FAILED 8s after submit |

| metric | A | B |
|---|---|---|
| t01 queue wait (s) | 2 | — |
| t05 median wait (s) | 9 | — |
| t05 submit rate (jobs/s) | 134.5 | — |
| t05 drain time (s) | 19 | — |
| t08 gang queue wait (s) | 1 | 5 |
| t09 whole-node median wait (s) | 1 | — |
| t10 fill median wait (s) | 2 | — |
| t08 gang rank start spread (ms) | 134 | 5 |
| t10 time to fill every core (s) | 2 | — |
| t11 nodes used by N small jobs | 3 | — |
| t12 failed gang: submit to job end (s) | 8 | 8 |

Waits include scheduler poll intervals (Gnomon agent heartbeat, Slurm sched/backfill intervals) and, on a shared Slurm cluster, other users' load.
