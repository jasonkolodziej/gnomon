#!/usr/bin/env bash
# A live demo of everything Gnomon does, for the web UI: three Raft controllers running every WASM
# plugin world, six node agents (two with GPUs), gnomon-web, and a load generator.
#
#   testkit/web-demo.sh start      # then open http://127.0.0.1:8080
#   testkit/web-demo.sh failover   # SIGKILL the current Raft leader (watch the cluster view)
#   testkit/web-demo.sh revive     # restart every stopped controller
#   testkit/web-demo.sh status
#   testkit/web-demo.sh stop
#
# The load generator submits a steady mix: short and long jobs, 2-3 node gangs, GPU jobs (the
# admission plugin moves them to partition gpu), a job too big for admission (rejected), a gang
# whose rank 0 fails (its peers are stopped through the kill channel), and cancellations.
#
# Env: GNOMON_BIN (binaries; default: build release into target/), WEB_DEMO_DIR
# (/tmp/gnomon-web-demo), WEB_DEMO_LISTEN (127.0.0.1:8080), WEB_DEMO_UI (ui/dist).
# Jobs run as the invoking user (as `nobody` when that is root). Linux; uses ports 7641-7643,
# 7701-7703 and the web port.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
dir=${WEB_DEMO_DIR:-/tmp/gnomon-web-demo}
listen=${WEB_DEMO_LISTEN:-127.0.0.1:8080}
ui=${WEB_DEMO_UI:-$root/ui/dist}
peers=1=127.0.0.1:7701,2=127.0.0.1:7702,3=127.0.0.1:7703
addrs=http://127.0.0.1:7641,http://127.0.0.1:7642,http://127.0.0.1:7643
export GNOMON_HEARTBEAT_SECS=2

die() { echo "web-demo: $*" >&2; exit 1; }

bins() {
  if [ -n "${GNOMON_BIN:-}" ]; then echo "$GNOMON_BIN"; return; fi
  (cd "$root" && cargo build -q --release -p gnomon-ctl -p gnomon-agent -p gnomon-cli -p gnomon-web) >&2
  echo "$root/target/release"
}

job_user() { if [ "$(id -u)" -eq 0 ]; then echo nobody; else id -un; fi; }

start_ctl() { # start_ctl <id>
  local i=$1 p="$dir/plugins"
  "$B/gnomon-ctl" --raft-id "$i" --raft-peers "$peers" --state-dir "$dir/ctl$i" \
    --listen "127.0.0.1:764$i" \
    --admission-plugin "$p/admission.wat" --priority-plugin "$p/priority.wat" \
    --shadow-priority-plugin "$p/shadow-priority.wat" --estimator-plugin "$p/estimator.wat" \
    --node-score-plugin "$p/node-score.wat" >> "$dir/ctl$i.log" 2>&1 &
  echo $! > "$dir/ctl$i.pid"
}

write_jobs() {
  mkdir -p "$dir/jobs"
  # shellcheck disable=SC2016  # these expand when the job runs, not here
  printf '#!/bin/sh\nsleep "${1:-60}"\n' > "$dir/jobs/sleep.sh"
  # Rank 0 fails after 4 s; the other ranks would run for 5 minutes.
  # shellcheck disable=SC2016
  printf '#!/bin/sh\nif [ "${GNOMON_NODE_RANK:-0}" = 0 ]; then sleep 4; exit 3; fi\nsleep 300\n' \
    > "$dir/jobs/fail-rank0.sh"
  chmod 755 "$dir/jobs"/*.sh
}

submit() { # submit <args...>
  USER=$(job_user) GNOMON_ADDR=$addrs "$B/gnbatch" "$@" >> "$dir/load.log" 2>&1 || true
}

load() {
  local n=0 s="$dir/jobs/sleep.sh"
  while true; do
    n=$((n + 1))
    case $((n % 12)) in
      0) submit -J "train-$n" -c 6 -N 3 -t 900 "$s" $((240 + RANDOM % 300)) ;;
      1|5|9) submit -J "sweep-$n" -c $((1 + RANDOM % 3)) -t 600 "$s" $((20 + RANDOM % 120)) ;;
      2) submit -J "gpu-infer-$n" -c 4 --gpus 1 -t 900 "$s" $((60 + RANDOM % 240)) ;;
      3) submit -J "mpi-$n" -c 4 -N 2 -t 600 "$s" $((60 + RANDOM % 120)) ;;
      4) submit -J "etl-$n" -c 2 -t 300 "$s" $((10 + RANDOM % 40)) ;;
      6) submit -J "too-big-$n" -c 8 -N 9 -t 600 "$s" 60 ;;   # admission: over 64 cores
      7) submit -J "solver-$n" -c 4 -N 3 -t 600 "$dir/jobs/fail-rank0.sh" ;;
      8) submit -J "analysis-$n" -c 8 -t 1800 "$s" $((300 + RANDOM % 600)) ;;
      10) # cancel the oldest running single-node job now and then
          id=$(GNOMON_ADDR=$addrs "$B/gnqueue" 2>/dev/null | awk '$3 == "RUNNING" && $6 == 1 {print $1; exit}')
          if [ -n "${id:-}" ]; then
            GNOMON_ADDR=$addrs "$B/gncancel" "$id" >> "$dir/load.log" 2>&1 || true
          fi ;;
      11) submit -J "render-$n" -c 2 -t 1200 "$s" $((120 + RANDOM % 400)) ;;
    esac
    sleep $((2 + RANDOM % 4))
  done
}

start() {
  [ -f "$dir/running" ] && die "already running (testkit/web-demo.sh stop)"
  B=$(bins)
  mkdir -p "$dir"
  chmod 755 "$dir"
  (cd "$root" && cargo run -q -p gnomon-policy --example demo-plugins -- "$dir/plugins" >/dev/null)
  write_jobs
  for i in 1 2 3; do start_ctl "$i"; done
  sleep 3
  local k=0
  for n in node00 node01 node02 node03 node04 node05; do
    local gpus=0
    case $n in node04|node05) gpus=2 ;; esac
    (cd "$dir" && GNOMON_NODE=$n GNOMON_CORES=8 GNOMON_MEM_MB=16384 GNOMON_GPUS=$gpus GNOMON_ADDR=$addrs \
      exec "$B/gnomon-agent" >> "$dir/agent-$n.log" 2>&1) &
    echo $! > "$dir/agent-$n.pid"
    k=$((k + 1))
  done
  GNOMON_ADDR=$addrs "$B/gnomon-web" --listen "$listen" --ui-dir "$ui" >> "$dir/web.log" 2>&1 &
  echo $! > "$dir/web.pid"
  load &
  echo $! > "$dir/load.pid"
  echo "$B" > "$dir/running"
  [ -f "$ui/index.html" ] || echo "web-demo: no UI in $ui yet: run ui/build.sh (the API is up meanwhile)" >&2
  echo "web-demo: http://$listen  (3 controllers, $k agents, logs in $dir)"
}

leader() { # the id of the controller that reports itself leader
  local i
  for i in 1 2 3; do
    grep -q "now leader" "$dir/ctl$i.log" 2>/dev/null || continue
    # The last leadership line wins: a later "no longer leader" cancels it.
    if [ "$(grep -E "now leader|no longer leader" "$dir/ctl$i.log" | tail -1 | grep -c "now leader")" = 1 ] &&
       kill -0 "$(cat "$dir/ctl$i.pid")" 2>/dev/null; then
      echo "$i"; return
    fi
  done
}

failover() {
  [ -f "$dir/running" ] || die "not running"
  local l; l=$(leader)
  [ -n "$l" ] || die "no live leader found"
  kill -9 "$(cat "$dir/ctl$l.pid")"
  echo "web-demo: SIGKILLed leader ctl$l; the survivors elect a new one (testkit/web-demo.sh revive brings it back)"
}

revive() {
  [ -f "$dir/running" ] || die "not running"
  B=$(cat "$dir/running")
  local i
  for i in 1 2 3; do
    kill -0 "$(cat "$dir/ctl$i.pid")" 2>/dev/null || { start_ctl "$i"; echo "web-demo: restarted ctl$i"; }
  done
}

status() {
  [ -f "$dir/running" ] || { echo "web-demo: not running"; return; }
  local f
  for f in "$dir"/*.pid; do
    if kill -0 "$(cat "$f")" 2>/dev/null; then echo "up    $(basename "$f" .pid)"; else echo "down  $(basename "$f" .pid)"; fi
  done
  echo "leader: ctl$(leader)"
}

stop() {
  local f
  for f in "$dir"/load.pid "$dir"/web.pid "$dir"/agent-*.pid "$dir"/ctl*.pid; do
    if [ -f "$f" ]; then kill "$(cat "$f")" 2>/dev/null || true; fi
  done
  rm -f "$dir/running"
  echo "web-demo: stopped (state and logs kept in $dir; delete it for a fresh cluster)"
}

case ${1:-} in
  start) start ;;
  stop) stop ;;
  status) status ;;
  failover) failover ;;
  revive) revive ;;
  *) sed -n '2,20p' "$0"; exit 2 ;;
esac
