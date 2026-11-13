#!/usr/bin/env bash
# Gnomon / Slurm comparison test suite.
#
#   ./run.sh --backend gnomon [--tests "t01 t03"] [--burst 50] [--quick]
#   ./run.sh --backend slurm  [--tests ...] [--burst 50]
#
# Results: out/<backend>-<timestamp>/{summary.tsv,metrics.tsv,*.rec}
# Compare two runs: ./compare.py out/gnomon-... out/slurm-...
set -uo pipefail
cd "$(dirname "$0")" || exit 2
TK_BACKEND=""; TESTS="t01 t02 t03 t04 t05 t06 t07"; BURST=50
while [ $# -gt 0 ]; do
  case $1 in
    --backend) TK_BACKEND=$2; shift 2 ;;
    --tests) TESTS=$2; shift 2 ;;
    --burst) BURST=$2; shift 2 ;;
    --quick) export TK_SCALE=${TK_SCALE:-0.25}; shift ;;
    -h|--help) sed -n 2,9p "$0"; exit 0 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done
export TK_BACKEND
# shellcheck source-path=SCRIPTDIR source=lib.sh
. ./lib.sh
[ "$TK_BACKEND" = slurm ] && [ "${TK_SCALE:-1}" != 1 ] && die "--quick is for Gnomon only (Slurm limits are whole minutes)"

TS=$(date +%Y%m%d-%H%M%S)
export TK_OUT="$PWD/out/$TK_BACKEND-$TS"
mkdir -p "$TK_OUT"; ln -sfn "$TK_OUT" out/current
export TESTKIT_OUT="$TK_OUT"
: > "$TK_OUT/summary.tsv"; : > "$TK_OUT/submits.tsv"
{
  echo "backend=$TK_BACKEND started=$(date -Is) host=$(hostname) scale=${TK_SCALE:-1}"
  [ "$TK_BACKEND" = slurm ] && echo "partition=${SLURM_PARTITION:-compute1} node=${SLURM_NODE:-any} slurm=$(sbatch --version 2>/dev/null)"
} > "$TK_OUT/run.info"

case $TK_BACKEND in
  gnomon) command -v gnbatch >/dev/null || die "gnbatch not on PATH (see README)"
          gninfo >/dev/null 2>&1 || die "controller not reachable at ${GNOMON_ADDR:-http://127.0.0.1:7631} (./local-cluster.sh start)" ;;
  slurm)  command -v sbatch >/dev/null || die "sbatch not found — run this on a Slurm login node" ;;
esac

CORES=$(tk_node_cores); [ -n "$CORES" ] || die "could not determine node cores"
log "backend=$TK_BACKEND cores/node=$CORES out=$TK_OUT"
# Grace for scheduling latency: Slurm's backfill runs every bf_interval (30 s
# default) and its main loop on events; Gnomon agents poll every heartbeat.
GRACE=180

# ---------------------------------------------------------------------------
t01() { # Smoke: submit, run, complete, output recorded.
  local id; id=$(tk_submit t01 hello 1 "$(sc 120)" "$(sc 5)")
  if tk_wait "$(( $(sc 120) + GRACE ))" "$id" && [ "$(tk_state "$id")" = COMPLETED ] && [ -n "$(rec_end "$id")" ]; then
    result t01 PASS "job $id completed; wait $(( $(rec_start "$id") - $(awk -v i="$id" '$1==i{print $4}' "$TK_OUT/submits.tsv") ))s"
  else result t01 FAIL "job $id state=$(tk_raw_state "$id")"; fi
}

t02() { # Resource request is visible to the job.
  local id; id=$(tk_submit t02 cpus4 4 "$(sc 120)" "$(sc 5)")
  tk_wait "$(( $(sc 120) + GRACE ))" "$id"
  local cpus nproc; cpus=$(rec_field "$id" start cpus); nproc=$(rec_field "$id" start nproc)
  if [ "$cpus" = 4 ]; then result t02 PASS "job sees cpus=4 (nproc=$nproc; equals 4 only with cgroup confinement)"
  else result t02 FAIL "job saw cpus=$cpus nproc=$nproc"; fi
}

t03() { # Backfill: a short small job jumps ahead of a wide job without delaying it;
        # a long small job must not.
  local c=$CORES
  local A B C D
  A=$(tk_submit t03 A-blocker $(( c - 1 )) "$(sc 300)" "$(sc 240)" 0 pin)
  tk_wait_running "$GRACE" "$A" || { result t03 FAIL "blocker never started"; return; }
  sleep 3
  B=$(tk_submit t03 B-wide "$c" "$(sc 120)" "$(sc 30)" 0 pin)
  sleep 3
  C=$(tk_submit t03 C-short 1 "$(sc 120)" "$(sc 30)" 0 pin)
  D=$(tk_submit t03 D-long 1 "$(sc 600)" "$(sc 30)" 0 pin)
  tk_wait "$(( $(sc 900) + GRACE ))" "$A" "$B" "$C" "$D"
  local sB sC sD; sB=$(rec_start "$B"); sC=$(rec_start "$C"); sD=$(rec_start "$D")
  if [ -n "$sB" ] && [ -n "$sC" ] && [ -n "$sD" ] && [ "$sC" -lt "$sB" ] && [ "$sD" -ge "$sB" ]; then
    result t03 PASS "C backfilled $(( sB - sC ))s before wide B; D waited for B (+$(( sD - sB ))s)"
  else result t03 FAIL "starts B=$sB C=$sC D=$sD (need C<B<=D)"; fi
}

t04() { # Cancel a queued job: it never runs and ends CANCELLED.
  local c=$CORES A X
  A=$(tk_submit t04 blocker "$c" "$(sc 120)" "$(sc 60)" 0 pin)
  tk_wait_running "$GRACE" "$A" || { result t04 FAIL "blocker never started"; return; }
  X=$(tk_submit t04 victim "$c" "$(sc 120)" "$(sc 10)" 0 pin)
  sleep 3; tk_cancel "$X"
  tk_wait "$(( $(sc 120) + GRACE ))" "$A" "$X"
  if [ "$(tk_state "$X")" = CANCELLED ] && [ -z "$(rec_start "$X")" ]; then result t04 PASS "queued job $X cancelled, never ran"
  else result t04 FAIL "victim state=$(tk_raw_state "$X") ran=$([ -n "$(rec_start "$X")" ] && echo yes || echo no)"; fi
}

t05() { # Throughput: N short 1-core jobs; submit rate and time to drain.
  local ids=() t0 t1 i
  t0=$(date +%s.%N)
  for i in $(seq 1 "$BURST"); do ids+=("$(tk_submit t05 "b$i" 1 "$(sc 120)" 2)"); done
  t1=$(date +%s.%N)
  tk_wait $(( BURST * 10 + GRACE )) "${ids[@]}"
  local done=0 last=0 e
  for i in "${ids[@]}"; do e=$(rec_end "$i"); [ -n "$e" ] && { done=$((done+1)); [ "$e" -gt "$last" ] && last=$e; }; done
  local sub; sub=$(awk -v a="$t0" -v b="$t1" -v n="$BURST" 'BEGIN{printf "%.1f", n/(b-a)}')
  local drain=$(( last - ${t0%.*} ))
  if [ "$done" -eq "$BURST" ]; then result t05 PASS "$BURST jobs; submit ${sub} jobs/s; drained in ${drain}s"
  else result t05 FAIL "$done/$BURST completed"; fi
  printf 't05_submit_rate\t%s\nt05_drain_s\t%s\n' "$sub" "$drain" >> "$TK_OUT/metrics_extra.tsv"
}

t06() { # Walltime: a job that outlives its limit is killed.
  local lim; lim=$(sc 60)
  local id; id=$(tk_submit t06 overrun 1 "$lim" "$(( lim * 3 ))")
  tk_wait "$(( lim * 3 + GRACE ))" "$id"
  local st; st=$(tk_state "$id")
  if [ -n "$(rec_start "$id")" ] && [ -z "$(rec_end "$id")" ] && [ "$st" = FAILED ]; then
    result t06 PASS "killed at limit (state $(tk_raw_state "$id"))"
  else result t06 FAIL "state=$(tk_raw_state "$id") finished=$([ -n "$(rec_end "$id")" ] && echo yes || echo no)"; fi
}

t07() { # Non-zero exit code is reported as a failure.
  local id; id=$(tk_submit t07 exit3 1 "$(sc 120)" 2 3)
  tk_wait "$(( $(sc 120) + GRACE ))" "$id"
  if [ "$(tk_state "$id")" = FAILED ]; then result t07 PASS "exit 3 -> $(tk_raw_state "$id")"
  else result t07 FAIL "state=$(tk_raw_state "$id")"; fi
}

# ---------------------------------------------------------------------------
for t in $TESTS; do
  declare -F "$t" >/dev/null || { result "$t" SKIP "unknown test"; continue; }
  log "running $t"; "$t"
done

# Per-job metrics: id test tag submit start end wait run state
{
  printf 'jobid\ttest\ttag\tsubmit\tstart\tend\twait_s\trun_s\tstate\n'
  while IFS=$'\t' read -r id test tag sub _cpus _lim; do
    s=$(rec_start "$id"); e=$(rec_end "$id")
    w=""; r=""; [ -n "$s" ] && w=$(( s - sub )); [ -n "$s" ] && [ -n "$e" ] && r=$(( e - s ))
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$id" "$test" "$tag" "$sub" "$s" "$e" "$w" "$r" "$(tk_raw_state "$id")"
  done < "$TK_OUT/submits.tsv"
} > "$TK_OUT/metrics.tsv"

echo; column -t -s $'\t' "$TK_OUT/summary.tsv" 2>/dev/null || cat "$TK_OUT/summary.tsv"
echo "results: $TK_OUT"
! grep -q $'\tFAIL\t' "$TK_OUT/summary.tsv"
