# Gnomon test kit

Seven scheduler tests that run unchanged against Gnomon and Slurm, so the two can be compared.

```bash
# Gnomon, one machine
GNOMON_CORES=8 ./local-cluster.sh start
./run.sh --backend gnomon --quick          # ~6 min; drop --quick for full length (~20 min)
./local-cluster.sh stop

# Slurm (e.g. UTSA Arc), from a login node, kit on a shared filesystem
export SLURM_PARTITION=compute1 SLURM_NODE=<idle-node>
./run.sh --backend slurm

# Compare
./compare.py out/gnomon-<ts> out/slurm-<ts> > comparison.md
```

| Test | Checks | Pass rule |
|---|---|---|
| t01 | submit → run → complete | COMPLETED + end record |
| t02 | CPU request reaches the job | job sees 4 CPUs |
| t03 | backfill | short job starts before the wide job; long job after it |
| t04 | cancel a queued job | CANCELLED, never ran |
| t05 | burst of N small jobs (`--burst`) | all complete; submit rate, drain time |
| t06 | walltime kill | started, never finished, ends failed |
| t07 | non-zero exit | ends FAILED |

Results land in `out/<backend>-<timestamp>/` (`summary.tsv`, `metrics.tsv`, raw `.rec` records).
`run.sh` exits non-zero if any test fails. Full guide: "Gnomon — Install, Usage & Slurm Comparison
Test Plan".

## Side by side on one machine

No Slurm cluster at hand? `slurm-local.sh` starts a single-node Slurm (slurmctld + slurmd + munge, from
the Ubuntu/Debian `slurm-wlm` and `munge` packages) on the same host, advertising the same 8 CPUs as
`local-cluster.sh`, so both schedulers run the identical suite on identical hardware:

```bash
sudo apt-get install -y slurm-wlm munge
sudo ./slurm-local.sh start
SLURM_PARTITION=debug ./run.sh --backend slurm          # full length (Slurm limits are whole minutes)
sudo ./slurm-local.sh stop

GNOMON_CORES=8 ./local-cluster.sh start
./run.sh --backend gnomon                                # full length, same durations
./local-cluster.sh stop

./compare.py out/gnomon-<ts> out/slurm-<ts>
```

There is no `slurmdbd`, so `sacct` has no data; the kit reads finished jobs from `scontrol show job`
(kept for `MinJobAge`). CI runs exactly this weekly and on demand (`.github/workflows/e2e.yml`, job
*Gnomon vs Slurm*) and runs the quick Gnomon suite on every pull request. A one-node Slurm compares
scheduler behaviour and latency, not scale or multi-node placement.
