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
