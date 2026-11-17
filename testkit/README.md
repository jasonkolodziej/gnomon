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

Run the kit as an ordinary user. If `local-cluster.sh` runs as root, the agents run each job as its
submitting user (`$USER`) and refuse `root`, as slurmd does; submit as a non-root user whose account can
read the kit (`USER=<name>`, kit on a readable path).

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
(kept for `MinJobAge`). CI runs exactly this, both schedulers sequentially in one job on the same runner, weekly and on demand (`.github/workflows/e2e.yml`, job
*Gnomon vs Slurm*) and runs the quick Gnomon suite on every pull request. A one-node Slurm compares
scheduler behaviour and latency, not scale or multi-node placement.

## Multi-node, side by side (containers)

`multinode.sh` builds a real multi-node cluster of each kind on one Docker host: a `head` container
plus `MN_NODES` (3) node containers `node1..nodeN` on one Docker network, all from the same Ubuntu 24.04
image with `slurm-wlm` and `munge`, each node advertising `MN_CORES` (4) CPUs and `MN_MEM_MB` (4096) MiB.

- Slurm: `slurmctld` on `head`, `slurmd` on every node, one shared munge key and `slurm.conf`
  (cons_tres, backfill every 5 s, `proctrack/linuxproc`, `task/none`, no accounting).
- Gnomon: `gnomon-ctl` on `head`, `gnomon-agent` on every node (`GNOMON_NODE=nodeI`, same cores/memory),
  release binaries bind-mounted from `target/release` (`GNOMON_BIN`).

The kit directory is bind-mounted at the same path in every container, so it is the shared filesystem
for job working directories, records and output. The kit runs inside `head` as the unprivileged user
`tk`, and jobs run as `tk` on the nodes under both schedulers.

```bash
cargo build --release -p gnomon-ctl -p gnomon-agent -p gnomon-cli   # on Linux; on macOS see below
testkit/multinode.sh all            # Slurm, then Gnomon, then compare.py (~25-40 min)
# results: testkit/out/multinode-<ts>/{slurm-mn-*,gnomon-mn-*,comparison.md,logs-*}

# or step by step
testkit/multinode.sh up slurm       # or: up gnomon
testkit/multinode.sh run slurm --tests "t08 t10"
testkit/multinode.sh down
```

`run.sh --multinode` (which `multinode.sh run` passes) adds five tests and widens two:

| Test | Checks | Pass rule |
|---|---|---|
| t03, t04 | backfill and cancel, with the blocker and wide job as gangs over every node | as above |
| t08 | gang job over 3 nodes (`gnbatch --nodes 3` / `sbatch -N 3` + `srun`, `jobs/gang.sh`) | every rank ran, on distinct nodes, rank starts within 2 s |
| t09 | one whole-node job per node | distinct nodes, all running at once |
| t10 | nodes x cores single-core jobs | one wave (last start before first end), cores-per-node jobs on every node |
| t11 | N single-core jobs on an idle cluster | INFO only: packs or spreads |
| t12 | gang over 3 nodes whose rank 0 exits 3 after 2 s while the others sleep 300 s | the peers are stopped (Gnomon kill channel / `srun --kill-on-bad-exit`) and the job ends FAILED within 90 s |

Gnomon starts a gang's script on every node (`GNOMON_NODELIST`, `GNOMON_NODE_RANK`); Slurm runs a
batch script on the first node only, so `jobs/gang.sh` re-runs itself there through `srun`, one task
per node. Every rank appends its own `<jobid>.rank<r>.rec`. The node containers get `CAP_SYS_ADMIN`
and no seccomp profile (slurmd's `clone(2)` of slurmstepd needs it), the same for both schedulers.
All "nodes" share the host's CPUs and clock: this compares scheduling and placement, not hardware.
Container daemons are visible to host-side `pkill` (as in `slurm-local.sh stop`), so do not run the
single-machine kits on the same host while a multi-node cluster is up.

**macOS (Docker Desktop).** Host binaries are Mach-O and cannot run in the Linux node containers, so
on macOS `multinode.sh` builds Linux binaries for the Docker engine's architecture first, using the
repository `Dockerfile` (target `dist`), into `target/docker/linux-<arch>`. On any host, pass
`--build-linux` before the command to do the same, or set `GNOMON_BIN` to skip it:

```bash
testkit/multinode.sh all                   # macOS: builds Linux binaries in Docker, then runs
testkit/multinode.sh --build-linux up gnomon
```

Give Docker Desktop at least 4 CPUs and 8 GB, or lower `MN_CORES`/`MN_MEM_MB`.

CI runs `multinode.sh all` weekly and on demand (`e2e.yml`, job *Gnomon vs Slurm (multi-node,
containers)*). Results from a run: `results/testkit/COMPARISON.md`.

## Validating a host (cgroup v2 and containers)

`validate-host.sh` is not a scheduler comparison: it checks that a machine can confine Gnomon jobs.
It runs the agent's launcher against the real kernel and container runtime
(`crates/agent/tests/host_validation.rs`):

```bash
testkit/validate-host.sh            # everything (re-runs itself under sudo)
testkit/validate-host.sh cgroup     # cgroup v2 only: limits from inside the job, OOM, CPU cap, cgroup.kill
testkit/validate-host.sh container  # Docker/Podman only (GNOMON_CONTAINER_RUNTIME): env, user, limits, OOM, stop
```

The cgroup checks need a cgroup v2 hierarchy with the `cpu` and `memory` controllers (Linux). The
container checks need Docker or Podman (on macOS, Docker Desktop or Podman machine). A missing
prerequisite fails the run with the reason. CI runs it on Ubuntu 24.04 with Docker and with Podman
(`.github/workflows/host-validation.yml`).

## Testing end to end with Docker on macOS

The full end-to-end test on a Mac is [`multinode.sh`](multinode.sh) under Docker Desktop: a head
container and 3 node containers (Ubuntu 24.04) running the kit against Gnomon and against Slurm, then
`compare.py`.

A `cargo build --release` on macOS produces Mach-O binaries, which fail in the Linux node containers
with `exec format error`. So when `GNOMON_BIN` is unset on macOS, `multinode.sh` first builds Linux
binaries for the Docker engine's architecture from the repository `Dockerfile` (target `dist`) into
`target/docker/linux-<arch>`; no Rust toolchain is needed on the host for this path.

```sh
testkit/multinode.sh all                 # Slurm, then Gnomon, then compare.py (~25-40 min)

# or just Gnomon, step by step
testkit/multinode.sh up gnomon
testkit/multinode.sh run gnomon --quick
testkit/multinode.sh down
```

Results go to `testkit/out/multinode-<ts>/` (`comparison.md` plus logs).

> [!NOTE]
> - Docker Desktop shares `/Users` by default, so the kit's bind mount works from a checkout there.
> - Give Docker Desktop at least 4 CPUs and 8 GB, or lower `MN_CORES` / `MN_MEM_MB`. The 4 containers
>   share the VM's CPUs.
> - Jobs run unconfined in this setup (no cgroup delegation), so t02 ("job sees 4 CPUs") does not
>   reflect real limits.
> - [`validate-host.sh`](validate-host.sh) (real cgroup v2 and Docker/Podman checks) needs a Linux
>   host with root; CI runs it in [`host-validation.yml`](../.github/workflows/host-validation.yml).
> - A quicker check without Docker: `cargo build --release`, then `./local-cluster.sh start` and
>   `./run.sh --backend gnomon --quick`. CI never runs the kit on macOS, so this path is unverified there.
