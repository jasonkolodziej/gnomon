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

start() {
  [ -f "$PIDS" ] && { echo "already running (./local-cluster.sh stop)"; exit 1; }
  "${BIN}gnomon-ctl" > out/gnomon-ctl.log 2>&1 & echo $! >> "$PIDS"
  for _ in $(seq 1 50); do "${BIN}gninfo" >/dev/null 2>&1 && break; sleep 0.2; done
  for n in $(seq 0 $(( ${GNOMON_NODES:-1} - 1 ))); do
    GNOMON_NODE="node$n" GNOMON_CORES=${GNOMON_CORES:-8} GNOMON_MEM_MB=${GNOMON_MEM_MB:-16384} \
      "${BIN}gnomon-agent" > "out/gnomon-agent-node$n.log" 2>&1 & echo $! >> "$PIDS"
  done
  sleep 1; "${BIN}gninfo"
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
