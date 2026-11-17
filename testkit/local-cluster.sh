#!/usr/bin/env bash
# Start/stop a single-machine Gnomon cluster for the test kit.
#
#   ./local-cluster.sh start   # controller + GNOMON_NODES agents (default 1)
#   ./local-cluster.sh status
#   ./local-cluster.sh stop
#
# Env: GNOMON_NODES (1), GNOMON_CORES (8 per emulated node), GNOMON_MEM_MB (16384),
#      GNOMON_HEARTBEAT_SECS (2: agents poll for work every 2 s), GNOMON_BIN (dir
#      holding the binaries; default: PATH).
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p out
PIDS=out/.cluster.pids
BIN=${GNOMON_BIN:+$GNOMON_BIN/}
export GNOMON_HEARTBEAT_SECS=${GNOMON_HEARTBEAT_SECS:-2}

gninfo_within() { # gninfo_within <deadline>: gninfo bounded by the time left until <deadline> ($SECONDS)
  local left=$(( $1 - SECONDS ))
  [ "$left" -gt 0 ] || left=1
  GNOMON_TIMEOUT_SECS=$left "${BIN}gninfo"
}

registered() { # registered <listing> <node>: <node> registered during this start and is not down
  grep -q '^registered;' "out/gnomon-agent-$2.log" 2>/dev/null &&
    printf '%s\n' "$1" | awk -v n="$2" '$1 == n && $NF != "down*" { up = 1 } END { exit !up }'
}

start() {
  [ -f "$PIDS" ] && { echo "already running (./local-cluster.sh stop)"; exit 1; }
  "${BIN}gnomon-ctl" > out/gnomon-ctl.log 2>&1 & echo $! >> "$PIDS"
  for _ in $(seq 1 50); do gninfo_within $(( SECONDS + 2 )) >/dev/null 2>&1 && break; sleep 0.2; done
  local want=${GNOMON_NODES:-1} n listing missing
  for n in $(seq 0 $(( want - 1 ))); do
    GNOMON_NODE="node$n" GNOMON_CORES=${GNOMON_CORES:-8} GNOMON_MEM_MB=${GNOMON_MEM_MB:-16384} \
      "${BIN}gnomon-agent" > "out/gnomon-agent-node$n.log" 2>&1 & echo $! >> "$PIDS"
  done
  # Agents probe cgroups and the container runtime before registering, which can take seconds.
  # Rows restored from GNOMON_STATE_DIR exist before then, so require this start's log line too.
  local deadline=$(( SECONDS + 30 ))
  while :; do
    listing=$(gninfo_within "$deadline" 2>/dev/null) || listing=""
    missing=""
    for n in $(seq 0 $(( want - 1 ))); do
      registered "$listing" "node$n" || missing="$missing node$n"
    done
    [ -z "$missing" ] && break
    if [ "$SECONDS" -ge "$deadline" ]; then
      gninfo_within $(( SECONDS + 5 )) || true
      echo "agents not registered within 30s:$missing" >&2
      for n in $missing; do
        echo "--- out/gnomon-agent-$n.log" >&2
        tail -n 20 "out/gnomon-agent-$n.log" >&2 || true
      done
      exit 1
    fi
    sleep 0.2
  done
  printf '%s\n' "$listing"
}

stop() {
  [ -f "$PIDS" ] || { echo "not running"; return 0; }
  xargs kill 2>/dev/null < "$PIDS" || true
  rm -f "$PIDS"; echo "stopped"
}

case ${1:-} in
  start) start ;;
  stop) stop ;;
  status) [ -f "$PIDS" ] && "${BIN}gninfo" || echo "not running" ;;
  *) sed -n 2,11p "$0"; exit 2 ;;
esac
