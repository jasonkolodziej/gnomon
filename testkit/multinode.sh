#!/usr/bin/env bash
# Multi-node Gnomon vs Slurm on one Docker host: every "node" is its own container.
#
#   ./multinode.sh up slurm|gnomon    # head container + MN_NODES node containers on one network
#   ./multinode.sh run slurm|gnomon [run.sh options]   # run.sh --multinode inside the head, as user tk
#   ./multinode.sh down               # save logs, remove the containers and the network
#   ./multinode.sh status
#   ./multinode.sh all [run.sh options]   # up/run/down Slurm, then Gnomon, then compare.py
#   ./multinode.sh --build-linux <command> ...   # first build Linux binaries in Docker (see below)
#
# Both clusters use the same image (Ubuntu 24.04 with slurm-wlm and munge) and the same node
# containers, named node1..nodeN, each advertising MN_CORES CPUs and MN_MEM_MB MiB:
#   Slurm:  slurmctld on `head`, slurmd on every node, shared munge key and slurm.conf.
#   Gnomon: gnomon-ctl on `head`, gnomon-agent on every node (GNOMON_NODE=nodeI).
# The test kit directory is bind-mounted at the same path in every container, so job records and
# output written on any node land in testkit/out/ on the host (the shared "filesystem").
# Jobs run as the unprivileged user `tk` (uid MN_TK_UID: the invoking user, or 1000 for root).
#
# Env: MN_NODES (3), MN_CORES (4), MN_MEM_MB (4096), GNOMON_BIN (../target/release),
#      MN_IMAGE (gnomon-testkit-node), MN_PREFIX (tkmn: container and network name prefix).
# --build-linux compiles gnomon-ctl/-agent/-cli for the Docker engine's own Linux architecture with
# the repository Dockerfile (target `dist`) into ../target/docker/linux-<arch> and uses those as
# GNOMON_BIN. It is the default on macOS when GNOMON_BIN is unset, since host binaries are Mach-O.
# No cgroup confinement: the host may lack cgroup v2 delegation, so Slurm uses proctrack/linuxproc
# and task/none and the agents launch unconfined, as slurm-local.sh and local-cluster.sh do.
set -euo pipefail
TK_DIR=$(cd "$(dirname "$0")" && pwd)
NODES=${MN_NODES:-3}
CORES=${MN_CORES:-4}
MEM=${MN_MEM_MB:-4096}
P=${MN_PREFIX:-tkmn}
NET="$P-net"
STATE="$TK_DIR/out/.multinode"
REPO_DIR=$(cd "$TK_DIR/.." && pwd)
BUILD_LINUX=0
if [ "${1:-}" = --build-linux ]; then BUILD_LINUX=1; shift
elif [ -z "${GNOMON_BIN:-}" ] && [ "$(uname -s)" = Darwin ]; then BUILD_LINUX=1
fi
BIN=${GNOMON_BIN:-$REPO_DIR/target/release}
TK_UID=${MN_TK_UID:-$(id -u)}
[ "$TK_UID" -eq 0 ] && TK_UID=1000
IMAGE="${MN_IMAGE:-gnomon-testkit-node}:u$TK_UID"
GUEST_PATH=/opt/gnomon/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

log() { printf '[%s] %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
die() { echo "error: $*" >&2; exit 2; }
node_names() { local i; for i in $(seq 1 "$NODES"); do echo "node$i"; done; }
on_head() { docker exec "$P-head" "$@"; }

# --- inside a container ------------------------------------------------------
entry() { # entry <role>: start this container's daemon in the foreground
  case $1 in
    slurm-head|slurm-node)
      install -o munge -g munge -m 400 "$STATE/munge.key" /etc/munge/munge.key
      mkdir -p /run/munge && chown munge:munge /run/munge
      runuser -u munge -- munged
      if [ "$1" = slurm-head ]; then
        mkdir -p /var/spool/slurmctld && chown slurm:slurm /var/spool/slurmctld
        exec slurmctld -D
      fi
      mkdir -p /var/spool/slurmd
      exec slurmd -D ;;
    gnomon-head) exec gnomon-ctl ;;
    gnomon-node) exec gnomon-agent ;;
    *) die "unknown role $1" ;;
  esac
}

# --- on the host ---------------------------------------------------------------
build_linux() { # compile the Linux binaries in Docker; sets BIN
  local arch
  arch=$(docker version --format '{{.Server.Arch}}') || die "docker engine not reachable"
  BIN="$REPO_DIR/target/docker/linux-$arch"
  log "building Linux binaries (linux/$arch) into $BIN"
  DOCKER_BUILDKIT=1 docker build --target dist --platform "linux/$arch" \
    --output "type=local,dest=$BIN" "$REPO_DIR" ||
    die "Linux build failed (docker build --target dist $REPO_DIR)"
}

build() {
  docker image inspect "$IMAGE" >/dev/null 2>&1 && return 0
  log "building $IMAGE"
  docker build -q -t "$IMAGE" --build-arg TK_UID="$TK_UID" - <<'EOF' >/dev/null
FROM ubuntu:24.04
ARG TK_UID=1000
RUN apt-get update -q \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends \
      slurm-wlm munge bsdextrautils procps hostname ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN (userdel -r ubuntu 2>/dev/null || true) && useradd -m -u "$TK_UID" -s /bin/bash tk
EOF
}

write_slurm_conf() {
  cat > "$STATE/slurm.conf" <<EOF
ClusterName=testkit
SlurmctldHost=head
SlurmUser=slurm
AuthType=auth/munge
StateSaveLocation=/var/spool/slurmctld
SlurmdSpoolDir=/var/spool/slurmd
SlurmctldLogFile=$STATE/logs/slurmctld.log
SlurmdLogFile=$STATE/logs/slurmd-%n.log
SlurmctldPidFile=/run/slurmctld.pid
SlurmdPidFile=/run/slurmd.pid
# No cgroup plugins: containers on a cgroup v1 host get no delegation (as slurm-local.sh).
ProctrackType=proctrack/linuxproc
TaskPlugin=task/none
JobAcctGatherType=jobacct_gather/none
AccountingStorageType=accounting_storage/none
MpiDefault=none
SchedulerType=sched/backfill
SchedulerParameters=bf_interval=5,bf_continue
SelectType=select/cons_tres
SelectTypeParameters=CR_Core_Memory
MinJobAge=3600
ReturnToService=2
SlurmdParameters=config_overrides
NodeName=node[1-$NODES] CPUs=$CORES RealMemory=$MEM State=UNKNOWN
PartitionName=debug Nodes=node[1-$NODES] Default=YES MaxTime=INFINITE State=UP
EOF
}

start_container() { # start_container <host> <role> [docker run options...]
  local host=$1 role=$2; shift 2
  # slurmd starts each slurmstepd with clone(2) namespace flags, which Docker's default seccomp
  # profile refuses without CAP_SYS_ADMIN. Both clusters get the same options: identical nodes.
  docker run -d --init --name "$P-$host" --hostname "$host" --network "$NET" --network-alias "$host" \
    --cap-add SYS_ADMIN --security-opt seccomp=unconfined \
    --label "$P=1" -v "$TK_DIR:$TK_DIR" -v "$BIN:/opt/gnomon/bin:ro" \
    -e PATH="$GUEST_PATH" -e SLURM_CONF="$STATE/slurm.conf" "$@" \
    "$IMAGE" bash "$TK_DIR/multinode.sh" _entry "$role" >/dev/null
}

wait_for() { # wait_for <seconds> <what> <command...>: retry until the command succeeds
  local deadline=$(( SECONDS + $1 )) what=$2; shift 2
  until "$@" >/dev/null 2>&1; do
    [ "$SECONDS" -lt "$deadline" ] || { log "$what not ready after timeout"; return 1; }
    sleep 1
  done
}

slurm_ready() {
  local idle
  idle=$(on_head sinfo -h -N -o '%N %t' | awk '$2 == "idle" {print $1}' | sort -u | wc -l)
  [ "$idle" -eq "$NODES" ] && return 0
  on_head scontrol update NodeName="node[1-$NODES]" State=RESUME >/dev/null 2>&1 || true
  return 1
}
gnomon_up() { on_head gninfo >/dev/null; }
gnomon_ready() { [ "$(on_head gninfo | awk 'NR > 1 && $NF == "idle"' | wc -l)" -eq "$NODES" ]; }

up() {
  local backend=${1:-} n
  case $backend in slurm|gnomon) ;; *) die "usage: $0 up slurm|gnomon" ;; esac
  command -v docker >/dev/null || die "docker not found"
  docker ps -q --filter "label=$P=1" | grep -q . && die "a cluster is already up ($0 down)"
  if [ "$backend" = gnomon ]; then
    [ "$BUILD_LINUX" = 1 ] && build_linux
    for n in gnomon-ctl gnomon-agent gnbatch gnqueue gncancel gninfo; do
      [ -x "$BIN/$n" ] || die "$BIN/$n missing (cargo build --release -p gnomon-ctl -p gnomon-agent -p gnomon-cli on Linux, or $0 --build-linux)"
    done
  fi
  mkdir -p "$STATE/logs"
  # tk (jobs, run.sh) and the slurm user write here. No sticky bit, so the host can clean up.
  chmod 777 "$TK_DIR/out" "$STATE/logs"
  build
  docker network inspect "$NET" >/dev/null 2>&1 || docker network create "$NET" >/dev/null
  log "starting $backend: head + $NODES nodes ($CORES CPUs, $MEM MiB each)"
  if [ "$backend" = slurm ]; then
    write_slurm_conf
    [ -s "$STATE/munge.key" ] || { head -c 1024 /dev/urandom > "$STATE/munge.key"; chmod 600 "$STATE/munge.key"; }
    start_container head slurm-head
    for n in $(node_names); do start_container "$n" slurm-node; done
    wait_for 90 "slurm nodes" slurm_ready || { on_head sinfo -N || true; die "slurm did not come up (logs: $STATE/logs)"; }
    on_head sinfo -N
  else
    start_container head gnomon-head -e GNOMON_HEARTBEAT_SECS=2
    wait_for 30 "gnomon-ctl" gnomon_up || die "gnomon-ctl did not start (docker logs $P-head)"
    for n in $(node_names); do
      start_container "$n" gnomon-node -e GNOMON_HEARTBEAT_SECS=2 -e GNOMON_ADDR=http://head:7631 \
        -e GNOMON_NODE="$n" -e GNOMON_CORES="$CORES" -e GNOMON_MEM_MB="$MEM" -e GNOMON_WORKDIR="$TK_DIR/out"
    done
    wait_for 60 "gnomon agents" gnomon_ready || { on_head gninfo || true; die "agents did not register (docker logs $P-node1)"; }
    on_head gninfo
  fi
}

down() {
  local c names
  names=$(docker ps -a --filter "label=$P=1" --format '{{.Names}}')
  if [ -n "$names" ]; then
    mkdir -p "$STATE/logs"
    for c in $names; do docker logs "$c" > "$STATE/logs/$c.docker.log" 2>&1 || true; done
    # Slurm writes its logs as root/slurm with private modes; let the invoking user read them.
    docker exec "$P-head" chmod -R a+rX "$STATE/logs" 2>/dev/null || true
    # shellcheck disable=SC2086  # one container name per word
    docker rm -f $names >/dev/null
  fi
  docker network rm "$NET" >/dev/null 2>&1 || true
  echo "stopped"
}

run() {
  local backend=${1:-}; shift || true
  case $backend in slurm|gnomon) ;; *) die "usage: $0 run slurm|gnomon [run.sh options]" ;; esac
  docker exec -u tk -e USER=tk -e HOME=/home/tk -e SLURM_PARTITION=debug -w "$TK_DIR" "$P-head" \
    ./run.sh --backend "$backend" --multinode --label mn "$@"
}

all() {
  local res status=0 b run_dir
  res="$TK_DIR/out/multinode-$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$res"
  down >/dev/null
  for b in slurm gnomon; do
    rm -rf "$STATE/logs"
    if (up "$b"); then
      run "$b" "$@" || status=1
      # A host-side `pkill slurmd` or `pkill gnomon-agent` (e.g. slurm-local.sh stop) also kills
      # these containers' daemons: say so instead of leaving a truncated run unexplained.
      [ "$(docker ps -q --filter "label=$P=1" --filter status=running | wc -l)" -eq $(( NODES + 1 )) ] ||
        { log "error: $b cluster containers died during the run (see $res/logs-$b)"; status=1; }
      run_dir=$(readlink -f "$TK_DIR/out/current")
      case $(basename "$run_dir") in "$b"-mn-*) mv "$run_dir" "$res/" ;; *) status=1 ;; esac
    else status=1; fi
    down >/dev/null
    [ -d "$STATE/logs" ] && cp -r "$STATE/logs" "$res/logs-$b"
  done
  local g s
  g=$(find "$res" -maxdepth 1 -name 'gnomon-mn-*' | head -1)
  s=$(find "$res" -maxdepth 1 -name 'slurm-mn-*' | head -1)
  if [ -n "$g" ] && [ -n "$s" ]; then
    python3 "$TK_DIR/compare.py" "$g" "$s" > "$res/comparison.md"
    cat "$res/comparison.md"
  else
    echo "a run is missing; no comparison" >&2; status=1
  fi
  echo "results: $res"
  return "$status"
}

case ${1:-} in
  _entry) entry "${2:-}" ;;
  up) up "${2:-}" ;;
  down) down ;;
  run) shift; run "$@" ;;
  all) shift; all "$@" ;;
  status) docker ps --filter "label=$P=1" --format 'table {{.Names}}\t{{.Status}}'
          on_head sinfo -N 2>/dev/null || on_head gninfo 2>/dev/null || true ;;
  *) sed -n 2,25p "$0"; exit 2 ;;
esac
