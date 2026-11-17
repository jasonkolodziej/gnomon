# Installing and running Gnomon

This guide covers installing Gnomon from a release or from source, running it under systemd or in
containers, and setting up job confinement (cgroup v2) and container jobs on the nodes. The second
half is a reference for every binary: what it does, how to use it and how to configure it.

How the pieces fit together is described in [ARCHITECTURE.md](ARCHITECTURE.md).

- [What gets installed](#what-gets-installed)
- [Install](#install): [release tarball](#from-a-release-tarball), [container images](#container-images), [from source](#from-source)
- [Run under systemd](#run-under-systemd): [controller](#controller-host), [agents](#node-agents), [replicated controllers](#replicated-controllers)
- [Job confinement with cgroup v2](#job-confinement-with-cgroup-v2)
- [Container jobs and rootless Docker](#container-jobs-and-rootless-docker)
- [Binary reference](#binary-reference): [`gnomon-ctl`](#gnomon-ctl), [`gnomon-agent`](#gnomon-agent), [`gnbatch`](#gnbatch), [`gnqueue`](#gnqueue), [`gncancel`](#gncancel), [`gninfo`](#gninfo), [`gntop`](#gntop)
- [Environment variable reference](#environment-variable-reference)
- [Upgrading and uninstalling](#upgrading-and-uninstalling)

## What gets installed

| Binary | Runs on | Role | Slurm equivalent |
|---|---|---|---|
| `gnomon-ctl` | controller host(s) | Scheduler daemon: state machine, planner, gRPC API (`:7631`), optional Raft (`:7700`) | `slurmctld` |
| `gnomon-agent` | every compute node | Registers the node, polls for work, launches and confines jobs, reports exit codes | `slurmd` |
| `gnbatch` | login nodes, anywhere | Submit a job script | `sbatch` |
| `gnqueue` | login nodes, anywhere | List jobs | `squeue` |
| `gncancel` | login nodes, anywhere | Cancel a job | `scancel` |
| `gninfo` | login nodes, anywhere | List nodes | `sinfo` |
| `gntop` | any terminal | Live Gantt chart of the plan | (none) |

The client tools and `gntop` only need network access to the controller (`GNOMON_ADDR`).

**Platforms.** The controller, agent and client tools are built for Linux x86_64 and aarch64
(glibc 2.35 or newer: Debian 12, Ubuntu 22.04, RHEL 9 and later). macOS arm64 builds are published
for the client tools and for local experiments; job confinement (cgroups) is Linux-only.

## Install

### From a release tarball

Every tag `vX.Y.Z` publishes `gnomon-vX.Y.Z-<target>.tar.gz` on the
[Releases page](https://github.com/jasonkolodziej/gnomon/releases) for
`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` and `aarch64-apple-darwin`, plus `SHA256SUMS`.

```sh
VERSION=v0.1.0
TARGET=x86_64-unknown-linux-gnu        # or aarch64-unknown-linux-gnu, aarch64-apple-darwin
BASE=https://github.com/jasonkolodziej/gnomon/releases/download/$VERSION

curl -fLO "$BASE/gnomon-$VERSION-$TARGET.tar.gz"
curl -fLO "$BASE/SHA256SUMS"
sha256sum --ignore-missing -c SHA256SUMS      # macOS: shasum -a 256 --ignore-missing -c SHA256SUMS

tar -xzf "gnomon-$VERSION-$TARGET.tar.gz"
sudo install -m 0755 "gnomon-$VERSION-$TARGET"/bin/* /usr/local/bin/
```

The tarball contains:

```text
gnomon-vX.Y.Z-<target>/
├── bin/          gnomon-ctl gnomon-agent gnbatch gnqueue gncancel gninfo gntop
├── systemd/      gnomon-ctl.service gnomon-agent.service gnomon-ctl.env gnomon-agent.env
├── INSTALL.md    this file
├── README.md
└── LICENSE
```

With the GitHub CLI: `gh release download v0.1.0 -R jasonkolodziej/gnomon -p '*x86_64-unknown-linux-gnu*' -p SHA256SUMS`.

### Container images

Each release also pushes multi-arch (`linux/amd64`, `linux/arm64`) images to GHCR:

| Image | Contents | Entrypoint |
|---|---|---|
| `ghcr.io/jasonkolodziej/gnomon-ctl` | `gnomon-ctl`, `gninfo` (health check) | `gnomon-ctl`, state in the `/var/lib/gnomon` volume |
| `ghcr.io/jasonkolodziej/gnomon-agent` | `gnomon-agent` | `gnomon-agent`, jobs run in `/var/lib/gnomon/work` |
| `ghcr.io/jasonkolodziej/gnomon-cli` | `gnbatch`, `gnqueue`, `gncancel`, `gninfo`, `gntop` | `gninfo` by default |

Tags: `X.Y.Z`, `X.Y` and `latest` (pre-releases get only `X.Y.Z-rc.N`). Every image runs as the
unprivileged user `gnomon` (uid 7631).

A one-controller, one-agent cluster:

```sh
docker network create gnomon
docker run -d --name gnomon-ctl --network gnomon -p 7631:7631 \
  -e GNOMON_HEARTBEAT_SECS=2 -v gnomon-state:/var/lib/gnomon \
  ghcr.io/jasonkolodziej/gnomon-ctl:latest
docker run -d --name node1 --hostname node1 --network gnomon \
  -e GNOMON_ADDR=http://gnomon-ctl:7631 -e GNOMON_CORES=4 -e GNOMON_MEM_MB=4096 \
  ghcr.io/jasonkolodziej/gnomon-agent:latest

docker run --rm --network gnomon -e GNOMON_ADDR=http://gnomon-ctl:7631 \
  ghcr.io/jasonkolodziej/gnomon-cli:latest gninfo
docker run --rm -it --network gnomon -e GNOMON_ADDR=http://gnomon-ctl:7631 \
  ghcr.io/jasonkolodziej/gnomon-cli:latest gntop
```

The containerized agent runs jobs *inside its own container* as the `gnomon` user, without cgroup
confinement (the container has no writable cgroup2 root) and without container jobs (there is no
Docker CLI in the image). That is fine for trying Gnomon out and for test clusters. Production nodes
should run the agent binary under systemd, as described below.

To build the images yourself from a checkout:

```sh
docker build --target ctl   -t gnomon-ctl   .
docker build --target agent -t gnomon-agent .
docker build --target cli   -t gnomon-cli   .
docker build --target dist --output type=local,dest=out/ .   # just the Linux binaries, e.g. from macOS
```

### From source

Requirements: a recent stable Rust toolchain ([rustup](https://rustup.rs)) and `protoc`.

```sh
sudo apt-get install -y protobuf-compiler     # Debian/Ubuntu (or: dnf install protobuf-compiler; brew install protobuf)
git clone https://github.com/jasonkolodziej/gnomon && cd gnomon
cargo build --release --locked -p gnomon-ctl -p gnomon-agent -p gnomon-cli -p gnomon-top
sudo install -m 0755 target/release/{gnomon-ctl,gnomon-agent,gnbatch,gnqueue,gncancel,gninfo,gntop} /usr/local/bin/
```

`protoc` is found through `PATH` or the `PROTOC` environment variable.

## Run under systemd

The units and environment templates are in [`packaging/systemd/`](../packaging/systemd) (and in
`systemd/` inside every release tarball).

### Controller host

```sh
sudo useradd --system --home-dir /var/lib/gnomon --shell /usr/sbin/nologin gnomon
sudo install -d /etc/gnomon
sudo install -m 0644 systemd/gnomon-ctl.service /etc/systemd/system/
sudo install -m 0644 systemd/gnomon-ctl.env /etc/gnomon/          # edit as needed
sudo systemctl daemon-reload
sudo systemctl enable --now gnomon-ctl
journalctl -u gnomon-ctl -f        # "gnomon-ctl listening on 0.0.0.0:7631"
```

The unit runs `gnomon-ctl` as `gnomon`, with its state journalled to `/var/lib/gnomon`
(`StateDirectory=`), so a restart recovers every job, node and reservation. Open TCP 7631 to agents
and users (and 7700 between replicated controllers). The gRPC API is plaintext and unauthenticated
today: keep it on a trusted cluster network.

### Node agents

```sh
sudo install -d /etc/gnomon
sudo install -m 0644 systemd/gnomon-agent.service /etc/systemd/system/
sudo install -m 0644 systemd/gnomon-agent.env /etc/gnomon/
sudoedit /etc/gnomon/gnomon-agent.env    # at least GNOMON_ADDR and GNOMON_WORKDIR
sudo systemctl daemon-reload
sudo systemctl enable --now gnomon-agent
journalctl -u gnomon-agent -f            # "gnomon-agent 'node1': 32 cores, ...", then "registered; ..."
gninfo                                    # the node shows up as idle
```

The agent runs as **root**, like `slurmd`, so it can run each job as the user who submitted it. It
never runs a job as root, and refuses a job whose user has no local account. Job users therefore need
accounts on every node (local, LDAP or any NSS source) and read access to the job script and to
`GNOMON_WORKDIR`, ideally a shared filesystem mounted at the same path on all nodes and login hosts.

To run the agent unprivileged instead, add `User=`/`Group=` to the unit. All jobs then run as that
service user; confinement still works through the delegated cgroup.

Stopping or restarting the agent stops the jobs it is running. On SIGTERM it stops polling, stops
every running rank (`cgroup.kill`, or `docker kill`/`podman kill` for container jobs), reports each one
as exit 143 (the job ends `Failed`) and exits; the unit allows 90 s (`KillMode=mixed`,
`TimeoutStopSec=90`). To keep work, drain the node first: wait for, or `gncancel`, its jobs, then stop
the agent.

If the agent dies without that (SIGKILL, a crash, a power loss, a network partition), the leader
declares the node **lost** after `GNOMON_NODE_TIMEOUT_SECS` without a heartbeat (default: three
heartbeat intervals, at least 30 s):

- the node shows `down*` in `gninfo` and nothing new is placed on it;
- its running jobs end `Failed` (a gang's ranks on other nodes are stopped through the kill channel);
- work it had not fetched yet, and reservations on it, go back to `Pending` and are planned elsewhere.

The node comes back on its own when the agent heartbeats or registers again. An agent that was only
partitioned away is told to stop the ranks that were failed meanwhile, so nothing keeps running
unaccounted for, and the node takes new work only once it has reported them stopped. A newly
elected leader waits a full timeout before declaring any node lost, and a loss decision is dropped
if the node heartbeats before it commits.

### Replicated controllers

Three (or five) controllers form one Raft group. On each, set a distinct id and the same peer list in
`/etc/gnomon/gnomon-ctl.env`:

```sh
GNOMON_RAFT_ID=1                                  # 2 and 3 on the other hosts
GNOMON_RAFT_PEERS=1=ctl1:7700,2=ctl2:7700,3=ctl3:7700
```

and list every controller for the agents and the `gn*` tools:

```sh
GNOMON_ADDR=http://ctl1:7631,http://ctl2:7631,http://ctl3:7631
```

Agents fail over to the next controller on any error; the `gn*` tools only move on when a controller
cannot be reached. `gntop` takes a single URL: point it at one controller
(`gntop --addr http://ctl2:7631`). See [Connecting to the controller](#connecting-to-the-controller).

Any controller accepts requests (followers forward writes); only the elected leader schedules.
Membership is static. See [ARCHITECTURE.md](ARCHITECTURE.md#replication).

## Job confinement with cgroup v2

When the agent can write a cgroup v2 hierarchy offering the `cpu` and `memory` controllers, each job
rank runs in `<root>/gnomon/job_<id>_r<rank>` with `cpu.max` (requested cores), `memory.max`
(requested memory) and `memory.swap.max = 0`. Walltime and cancel kill the whole cgroup
(`cgroup.kill`, Linux 5.14+), so no process escapes. Without cgroup v2 the agent logs why once at
start-up and runs jobs unconfined (each in its own process group).

**Check the host:**

```sh
stat -fc %T /sys/fs/cgroup              # cgroup2fs
cat /sys/fs/cgroup/cgroup.controllers   # must include cpu and memory
```

**With the shipped unit (systemd 254+).** `Delegate=yes` hands the unit's cgroup to the agent, and
`DelegateSubgroup=supervisor` moves the agent process into a child cgroup, so the unit's own cgroup
holds no processes and can be the job root (cgroup v2's "no internal processes" rule). The unit
sets:

```ini
Delegate=yes
DelegateSubgroup=supervisor
Environment=GNOMON_CGROUP_ROOT=/sys/fs/cgroup/system.slice/gnomon-agent.service
```

Check it with `systemd-cgls -u gnomon-agent.service`: while jobs run you should see
`supervisor/` (the agent) and `gnomon/job_<id>_r<rank>/`. Ubuntu 24.04, Debian 13 and Fedora 39+
ship systemd 254 or newer (`systemctl --version`).

**Older systemd.** Remove `DelegateSubgroup=` and move the agent into a leaf cgroup before it starts:

```ini
Delegate=yes
ExecStartPre=/bin/sh -c 'mkdir -p /sys/fs/cgroup/system.slice/gnomon-agent.service/supervisor'
ExecStart=/bin/sh -c 'echo $$$$ > /sys/fs/cgroup/system.slice/gnomon-agent.service/supervisor/cgroup.procs && exec /usr/local/bin/gnomon-agent'
```

systemd turns `$$$$` into `$$`, which the shell then expands to its own PID (a single `$$` would
reach the shell as a literal `$` and the write would fail).

**Disable** confinement with `GNOMON_CGROUP=off`. **Validate** a node against the real kernel (and
Docker or Podman) with the test kit: `testkit/validate-host.sh` from a checkout, run as root.

## Container jobs and rootless Docker

A job submitted with an image (`gnbatch --container IMAGE`, or `#SBATCH --container=IMAGE`) runs
under Docker or Podman on the node, which enforces the CPU and memory limits:

```text
docker run --rm --name gnomon-job-<id>-r<rank> --cpus <cores> --memory <mem>m --network host \
  --user <identity> -v <workdir>:<workdir> -w <workdir> -e GNOMON_...=... --entrypoint sh <image> -c <command>
```

Only `GNOMON_WORKDIR` is mounted, so scripts given by absolute path must live under it.

| Runtime (`GNOMON_CONTAINER_RUNTIME`) | Job's `--user` | Notes |
|---|---|---|
| Rootful Docker, root agent (shipped unit) | the submitting user's `uid:gid` | container root would be host root, so it is never used |
| Rootless Docker, unprivileged agent | `0:0` | container root *is* the agent's user; detected from `docker info` (`GNOMON_CONTAINER_ROOTLESS=1/0` overrides) |
| Podman | agent's `uid:gid` + `--userns=keep-id` (non-root agent only) | GPUs through CDI, not `--gpus` |

**Rootless Docker for an unprivileged agent.** Do this on every compute node. Rootless Docker needs
an account with a home directory, subordinate uid/gid ranges and a running systemd user manager. The
account can keep a `nologin` shell, because nothing below starts a login shell.

```sh
# 1. Account and subordinate ids (pick a range unused by other users in /etc/subuid and /etc/subgid)
sudo useradd --system --create-home --home-dir /var/lib/gnomon --shell /usr/sbin/nologin gnomon
sudo usermod --add-subuids 231072-296607 --add-subgids 231072-296607 gnomon
sudo apt-get install -y docker-ce-rootless-extras uidmap dbus-user-session

# 2. A user manager that runs without a login session
sudo loginctl enable-linger gnomon
GUID=$(id -u gnomon)
USERENV="XDG_RUNTIME_DIR=/run/user/$GUID DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$GUID/bus"

# 3. Install and start the user's Docker daemon (as gnomon, without a login shell)
sudo -u gnomon -H env $USERENV dockerd-rootless-setuptool.sh install
sudo systemctl --user -M gnomon@ enable --now docker
sudo -u gnomon -H env $USERENV DOCKER_HOST=unix:///run/user/$GUID/docker.sock docker info --format '{{.SecurityOptions}}'   # lists rootless
```

Then run the agent as that user (`sudo systemctl edit gnomon-agent` and add `User=gnomon` and
`Group=gnomon` under `[Service]`) and point it at the user's daemon in
`/etc/gnomon/gnomon-agent.env`:

```sh
DOCKER_HOST=unix:///run/user/<gnomon uid>/docker.sock
```

`--cpus`/`--memory` under rootless Docker need the `cpu` and `memory` controllers delegated to the
user. Recent distributions do this by default; otherwise run
`sudo systemctl edit user@.service` and add:

```ini
[Service]
Delegate=cpu cpuset io memory pids
```

## Binary reference

### Connecting to the controller

Every program finds the controller through `GNOMON_ADDR` (default `http://127.0.0.1:7631`), but they
handle a list of controllers and timeouts differently:

| Program | `GNOMON_ADDR` | Failover | Timeouts |
|---|---|---|---|
| `gnomon-agent` | one URL or a comma-separated list | on **any** error (connect or RPC) it moves to the next controller and re-registers | 15 s connect and per request (fixed) |
| `gnbatch`, `gnqueue`, `gncancel`, `gninfo` | one URL or a comma-separated list | tries each in order **only until one connects**; an error after connecting is returned, not retried | `GNOMON_TIMEOUT_SECS` (default 10, fractions allowed) for the connect and the request |
| `gntop` | **one URL** (or `--addr`) | none; keeps retrying the same controller and shows `disconnected` | poll interval, clamped to 1–5 s |

### gnomon-ctl

The controller: holds the cluster state machine, plans jobs on the time axis (conservative backfill
with reservations), serves the gRPC API and, optionally, replicates through Raft.

```text
gnomon-ctl [--listen ADDR] [--state-dir DIR] [--raft-id N --raft-peers ID=HOST:PORT,...]
           [--admission-plugin FILE.wasm [--admission-fail open|closed]]
           [--priority-plugin FILE.wasm] [--policy-config FILE.json] [--shadow-priority-plugin FILE.wasm]
           [--estimator-plugin FILE.wasm] [--node-score-plugin FILE.wasm [--node-score-candidates N|all]]
           [--plugin-fuel N] [--plugin-memory-mb N]
gnomon-ctl --demo        # in-process demo: submits a few jobs and prints the plan, then exits
```

| Flag | Environment | Default | Meaning |
|---|---|---|---|
| `--listen ADDR` | `GNOMON_LISTEN` | `0.0.0.0:7631` | gRPC listen address |
| `--state-dir DIR` | `GNOMON_STATE_DIR` | none (in memory) | fsync'd event journal; a restart recovers jobs, nodes and reservations exactly |
| `--raft-id N` | `GNOMON_RAFT_ID` | none | this replica's id in the peer list (needs `--state-dir`) |
| `--raft-peers LIST` | `GNOMON_RAFT_PEERS` | none | `1=ctl1:7700,2=ctl2:7700,...`; this replica's entry is also its Raft listen address |
| | `GNOMON_HEARTBEAT_SECS` | `10` | interval agents heartbeat and poll for work (sent at registration; agents use at least 2) |
| | `GNOMON_NODE_TIMEOUT_SECS` | 3 × heartbeat, at least `30` | seconds without a heartbeat after which the leader declares a node lost and fails its jobs |
| `--admission-plugin FILE` | `GNOMON_ADMISSION_PLUGIN` | none | accepts, rejects or rewrites each submission; see [Policy plugins](#policy-plugins) |
| `--admission-fail MODE` | `GNOMON_ADMISSION_FAIL` | `open` | when the admission plugin fails: `open` accepts the job unchanged, `closed` rejects it |
| `--priority-plugin FILE` | `GNOMON_PRIORITY_PLUGIN` | none | computes each submission's priority |
| `--policy-config FILE` | `GNOMON_POLICY_CONFIG` | built-in defaults | declarative policy JSON (`PolicyConfig`): the priority when there is no plugin, and the fallback when the plugin fails |
| `--shadow-priority-plugin FILE` | `GNOMON_SHADOW_PRIORITY_PLUGIN` | none | candidate priority plugin run beside the live priority and logged, never applied |
| `--estimator-plugin FILE` | `GNOMON_ESTIMATOR_PLUGIN` | none | predicts runtimes for backfill planning (walltime when it declines or fails) |
| `--node-score-plugin FILE` | `GNOMON_NODE_SCORE_PLUGIN` | none | chooses among the nodes a job could use at its planned start |
| `--node-score-candidates N` | `GNOMON_NODE_SCORE_CANDIDATES` | `64` | most nodes offered per node-score call besides the default choice, or `all` |
| `--plugin-fuel N` | `GNOMON_PLUGIN_FUEL` | `10000000` | instruction budget per plugin call |
| `--plugin-memory-mb N` | `GNOMON_PLUGIN_MEMORY_MB` | `16` | linear-memory cap per plugin call, summed over all of the plugin's memories |

Flags win over environment variables. A flag given without a value is an error rather than a silent
fallback (a typo must not quietly disable durability). The controller logs to stdout/stderr.

```sh
gnomon-ctl                                   # in-memory, for experiments
gnomon-ctl --state-dir /var/lib/gnomon       # durable single controller
```

### Policy plugins

Sites can replace four policy decisions with WebAssembly plugins, the sandboxed equivalent of Slurm's
`job_submit.lua` and C plugins ([ADR 0002](../.github/plans/0002-wasm-policy-plugins.md),
[0003](../.github/plans/0003-admission-plugins.md), [0004](../.github/plans/0004-node-score-plugins.md)).
Each plugin is a component for one world in
[`crates/policy/wit/policy.wit`](../crates/policy/wit/policy.wit), and
[`examples/policy-plugins`](../examples/policy-plugins) has a Rust example of each, with build steps.

| World | Runs | Decides | When it fails |
|---|---|---|---|
| `admission` | at submit | accept, reject (the user sees the reason) or rewrite the request | accept unchanged (`--admission-fail open`) or reject (`closed`) |
| `priority` | at submit, after admission | the job's priority | the declarative priority from `--policy-config` |
| `estimator` | on the leader, once per queued job | the runtime backfill plans for | the walltime |
| `node-score` | on the leader, per placement with a choice | which feasible nodes the job gets (never when it starts) | the default placement |

- **Results are recorded, not recomputed.** Submit-time plugins run before the `JobSubmitted` event
  is built, and planning plugins before the plan event. Replicas and recovery apply the recorded
  result and never run a plugin.
- **Plugins are pure.** A plugin may import nothing (no WASI, no clock: the current time is a
  parameter), so it can only compute from its arguments. A component that imports anything, or
  implements a different world, is refused when it loads.
- **Every call is bounded** by `--plugin-fuel` and `--plugin-memory-mb`. Running out of either,
  trapping or returning an invalid value (a rewrite to zero nodes, say) gives the fallback above,
  and the controller logs it with a running failure count. A plugin or config file that does not
  load stops the controller at startup.
- By default a job keeps the priority its client submitted (`gnbatch` sends 1000). With
  `--priority-plugin` or `--policy-config` the controller computes it instead.
- **Give every replica the same plugins and config.** The submit-side plugins run on whichever
  replica handles a submission, so replicas with different plugins treat jobs differently depending
  on which one a client reached. The planning plugins run on the leader, so a failover to a replica
  with different plugins changes later plans.
- **Try a candidate before deploying it.** Offline, `experiments <swf-dir> <out> 5000 --only e4
  --admission-plugin a.wasm --priority-plugin p.wasm --estimator-plugin e.wasm` schedules SWF trace
  windows with and without the plugins and writes `e4_plugins.csv`. Live,
  `--shadow-priority-plugin` runs a candidate priority beside the deployed one and logs how often it
  disagrees. `plugin_bench` (in `gnomon-policy`) measures what plugins cost a scheduling pass.

### gnomon-agent

The node agent: detects the node's cores, memory and GPUs (`nvidia-smi`/`rocm-smi`), registers with
the controller, heartbeats, fetches assignments, waits at the gang barrier, launches each job rank
(confined in a cgroup, or in a container), stops ranks on walltime or through the kill channel and
reports exit codes. It takes no flags; everything is environment.

| Environment | Default | Meaning |
|---|---|---|
| `GNOMON_ADDR` | `http://127.0.0.1:7631` | controller(s); fails over to the next on any error and re-registers (15 s connect and request timeout) |
| `GNOMON_NODE` | hostname | node name shown in `gninfo` and placements |
| `GNOMON_CORES` / `GNOMON_MEM_MB` / `GNOMON_GPUS` | detected | capacity to advertise (e.g. to leave room for the OS) |
| `GNOMON_WORKDIR` | agent's working directory | where jobs run; the only directory mounted into container jobs |
| `GNOMON_CGROUP_ROOT` | `/sys/fs/cgroup` | cgroup2 root for job cgroups (the unit sets its own delegated cgroup) |
| `GNOMON_CGROUP` | on | `off` disables cgroup confinement |
| `GNOMON_CONTAINER_RUNTIME` | `docker` | `docker` or `podman` (path or name) for container jobs |
| `GNOMON_CONTAINER_ROOTLESS` | detected | `1`/`0` overrides rootless Docker detection |
| `DOCKER_HOST` | runtime default | which Docker daemon to use (rootless: the user's socket) |

Exit codes the agent reports: the job's own status; `124` walltime reached; `125` gang peers never
arrived; `143` stopped through the kill channel (gang peer failed, or cancelled); `-1` the job could
not be started (unknown user, root user, cgroup creation failed); `137` OOM-killed in a container.

On SIGTERM or SIGINT the agent stops fetching work, stops its running ranks and reports them (exit
`143`) before exiting, waiting at most 60 s for the reports.

Every job sees `GNOMON_JOB_ID`, `GNOMON_JOB_NAME`, `GNOMON_CPUS`, `GNOMON_MEM_MB`, `GNOMON_GPUS`,
`GNOMON_TIME_LIMIT`, `GNOMON_NODELIST` (comma-separated), `GNOMON_NODE_RANK`, `GNOMON_NNODES` and
`GNOMON_JOB_START`.

### gnbatch

Submit a batch script, like `sbatch`. Prints `Submitted batch job <id>`.

```text
gnbatch [OPTIONS] [SCRIPT] [ARGS...]
```

| Option | Default | Meaning |
|---|---|---|
| `-J, --name NAME` | `job` | job name |
| `-c, --cpus N` | 1 | CPUs per node |
| `--mem-mb MB` | 1024 | memory per node, MiB |
| `--gpus N` | 0 | GPUs per node |
| `-t, --time SECS` | 600 | walltime limit, seconds |
| `-p, --partition NAME` | none | partition label |
| `-N, --nodes N` | 1 | nodes allocated together with a common start (gang); `min-max` uses `min` |
| `--container IMAGE` (`--container-image`) | none | run the job in this image on the node |

`#SBATCH` (or `#GNOMON`) directives at the top of the script are read first, then command-line
flags override them, as with `sbatch`. Recognized directives (long forms, plus `-N`): `--job-name`,
`--cpus-per-task`, `--mem` (units K/M/G/T, default MB), `--gpus`, `--gres=gpu:N`, `--time` (Slurm
formats `M`, `M:S`, `H:M:S`, `D-H:M:S`), `--partition`, `--nodes`/`-N`,
`--container`/`--container-image`. Other directives (`--output`, `--ntasks`, short forms such as
`-c 4`, ...) are accepted and ignored. The script path is made
absolute, so it must be readable at the same path on the nodes. The job runs as `$USER`.

```sh
gnbatch -c 8 --mem-mb 16384 -t 3600 train.sh --epochs 10
gnbatch -N 4 -c 16 -t 7200 mpi.sh              # 16 CPUs on each of 4 nodes, started together
gnbatch --container python:3.12 -c 2 job.sh    # under Docker/Podman on the node
```

### gnqueue

List jobs, like `squeue`.

```text
gnqueue [-u, --user NAME]
```

Columns: `JOBID USER STATE NAME CPUS NODES TIME START NODELIST`. `STATE` is one of `Pending`,
`Reserved` (backfilled with a future start), `Running`, `Completed`, `Cancelled`, `Failed`. `CPUS` is
per node, `TIME` the walltime limit in seconds, `START` the planned or actual start (epoch seconds,
0 while not yet placed).

### gncancel

Cancel a job, like `scancel`. Prints nothing on success.

```text
gncancel JOB_ID
```

A pending or reserved job is cancelled at once. A running job's ranks are stopped by their agents
(kill channel, exit 143) and the job ends `Cancelled`.

### gninfo

List nodes, like `sinfo`.

```text
gninfo
```

Columns: `NODELIST CPUS GPUS MEM(MB) STATE`, with `STATE` `idle`, `mix`, `alloc` or `down*` (no
heartbeat). The test kit and the container health check use its exit status to tell whether the
controller is reachable.

### gntop

A live terminal Gantt chart of the plan. One row per node (capacity, allocation and liveness in the
gutter); running spans are solid (`█`), future backfill reservations hatched (`░`), both labelled
`<id> <name>`; a red `│` marks now. Jobs not yet placed go to a `pending` lane, and the pending queue
(priority, wait time) is listed below the chart. If the controller goes away the last plan stays on
screen, marked stale, and polling reconnects by itself.

```text
gntop [--addr URL] [--interval D] [--history D] [--horizon D] [-u USER] [--once [--width N --height N]]
```

| Flag | Default | Meaning |
|---|---|---|
| `--addr URL` | `$GNOMON_ADDR` or `http://127.0.0.1:7631` | one controller URL (not a list) |
| `--interval D` | `1s` | poll interval (`500ms`, `5s`, ...) |
| `--history D` / `--horizon D` | `10m` / `2h` | window before / after now |
| `-u, --user NAME` | all | initial user filter |
| `--once` | off | print one plain-text frame and exit; non-zero exit when the controller is unreachable |
| `--width N` / `--height N` | terminal size, else 120×40 | frame size for `--once` |

| Key | Action |
|---|---|
| `q`, `Esc`, `Ctrl-C` | quit |
| `p` | pause / resume polling |
| `+` / `-` | zoom in / out |
| `←` / `→` (`h` / `l`) | pan a quarter window |
| `0` | reset zoom and pan |
| `/` | filter by user (Enter applies, empty shows all, Esc cancels) |

```sh
gntop --horizon 8h                       # a working day ahead
gntop --once --width 160 > plan.txt      # snapshot for a report or CI log
```

## Environment variable reference

| Variable | Read by | Default |
|---|---|---|
| `GNOMON_ADDR` | agent and `gn*` tools (list allowed), `gntop` (one URL) | `http://127.0.0.1:7631` |
| `GNOMON_TIMEOUT_SECS` | `gn*` tools | `10` |
| `GNOMON_LISTEN` | controller | `0.0.0.0:7631` |
| `GNOMON_STATE_DIR` | controller | in memory |
| `GNOMON_RAFT_ID`, `GNOMON_RAFT_PEERS` | controller | standalone |
| `GNOMON_HEARTBEAT_SECS` | controller | `10` |
| `GNOMON_NODE_TIMEOUT_SECS` | controller | 3 × heartbeat, at least `30` |
| `GNOMON_NODE` | agent | hostname |
| `GNOMON_CORES`, `GNOMON_MEM_MB`, `GNOMON_GPUS` | agent | detected |
| `GNOMON_WORKDIR` | agent | working directory |
| `GNOMON_CGROUP`, `GNOMON_CGROUP_ROOT` | agent | on, `/sys/fs/cgroup` |
| `GNOMON_CONTAINER_RUNTIME`, `GNOMON_CONTAINER_ROOTLESS` | agent | `docker`, detected |

## Upgrading and uninstalling

**Upgrade:** install the new binaries, then `sudo systemctl restart gnomon-ctl` (state is recovered
from `/var/lib/gnomon`) and restart agents when they are idle (restarting an agent stops its running
jobs and reports them failed). With replicated controllers, restart one controller at a time and wait for it to rejoin
before the next.

**Uninstall:**

```sh
sudo systemctl disable --now gnomon-agent gnomon-ctl
sudo rm -f /etc/systemd/system/gnomon-{ctl,agent}.service /usr/local/bin/{gnomon-ctl,gnomon-agent,gnbatch,gnqueue,gncancel,gninfo,gntop}
sudo rm -rf /etc/gnomon /var/lib/gnomon     # configuration and controller state
sudo systemctl daemon-reload
```
