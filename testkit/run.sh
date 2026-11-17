#!/usr/bin/env bash
# Gnomon / Slurm comparison test suite.
#
#   ./run.sh --backend gnomon [--tests "t01 t03"] [--burst 50] [--quick]
#   ./run.sh --backend slurm  [--tests ...] [--burst 50]
#   ./run.sh --backend <b> --multinode [--label mn]   # cluster of >= 2 nodes, see multinode.sh
#
# --multinode adds the multi-node tests t08-t11 and runs t03/t04's wide jobs as gangs over every
# node. Results: out/<backend>[-<label>]-<timestamp>/{summary.tsv,metrics.tsv,*.rec}
# Compare two runs: ./compare.py out/gnomon-... out/slurm-...
set -uo pipefail
cd "$(dirname "$0")" || exit 2
TK_BACKEND=""; TESTS=""; BURST=50; MULTI=0; LABEL=""
while [ $# -gt 0 ]; do
  case $1 in
    --backend) TK_BACKEND=$2; shift 2 ;;
    --tests) TESTS=$2; shift 2 ;;
    --burst) BURST=$2; shift 2 ;;
    --quick) export TK_SCALE=${TK_SCALE:-0.25}; shift ;;
    --multinode) MULTI=1; shift ;;
    --label) LABEL=$2; shift 2 ;;
    -h|--help) sed -n 2,11p "$0"; exit 0 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done
export TK_BACKEND
# shellcheck source-path=SCRIPTDIR source=lib.sh
. ./lib.sh
[ "$TK_BACKEND" = slurm ] && [ "${TK_SCALE:-1}" != 1 ] && die "--quick is for Gnomon only (Slurm limits are whole minutes)"

if [ -z "$TESTS" ]; then
  TESTS="t01 t02 t03 t04 t05 t06 t07"
  [ "$MULTI" = 1 ] && TESTS="$TESTS t08 t09 t10 t11 t12"
fi

case $TK_BACKEND in
  gnomon) command -v gnbatch >/dev/null || die "gnbatch not on PATH (see README)"
          gninfo >/dev/null 2>&1 || die "controller not reachable at ${GNOMON_ADDR:-http://127.0.0.1:7631} (./local-cluster.sh start)" ;;
  slurm)  command -v sbatch >/dev/null || die "sbatch not found — run this on a Slurm login node" ;;
esac

CORES=$(tk_node_cores); [ -n "$CORES" ] || die "could not determine node cores"
NODES=$(tk_node_count)
# WIDE: nodes of t03/t04's wide (whole-cluster) jobs; 1 = the single-node suite, pinned.
WIDE=1
if [ "$MULTI" = 1 ]; then
  [ "$NODES" -ge 2 ] || die "--multinode needs at least 2 nodes up (found $NODES)"
  WIDE=$NODES
fi

TS=$(date +%Y%m%d-%H%M%S)
export TK_OUT="$PWD/out/$TK_BACKEND${LABEL:+-$LABEL}-$TS"
mkdir -p "$TK_OUT"; ln -sfn "$TK_OUT" out/current
export TESTKIT_OUT="$TK_OUT"
: > "$TK_OUT/summary.tsv"; : > "$TK_OUT/submits.tsv"
{
  printf 'backend=%s started=%s host=%s scale=%s' "$TK_BACKEND" "$(date -Is)" "$(hostname)" "${TK_SCALE:-1}"
  [ "$MULTI" = 1 ] && printf ' nodes=%s cores/node=%s' "$NODES" "$CORES"
  echo
  [ "$TK_BACKEND" = slurm ] && echo "partition=${SLURM_PARTITION:-compute1} node=${SLURM_NODE:-any} slurm=$(sbatch --version 2>/dev/null)"
} > "$TK_OUT/run.info"

log "backend=$TK_BACKEND nodes=$NODES cores/node=$CORES wide=$WIDE out=$TK_OUT"
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
        # a long small job must not. Multi-node: the blocker and the wide job span every node.
  local c=$CORES
  local A B C D
  A=$(tk_submit t03 A-blocker $(( c - 1 )) "$(sc 300)" "$(sc 240)" 0 pin "$WIDE")
  tk_wait_running "$GRACE" "$A" || { result t03 FAIL "blocker never started"; return; }
  sleep 3
  B=$(tk_submit t03 B-wide "$c" "$(sc 120)" "$(sc 30)" 0 pin "$WIDE")
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
  A=$(tk_submit t04 blocker "$c" "$(sc 120)" "$(sc 60)" 0 pin "$WIDE")
  tk_wait_running "$GRACE" "$A" || { result t04 FAIL "blocker never started"; return; }
  X=$(tk_submit t04 victim "$c" "$(sc 120)" "$(sc 10)" 0 pin "$WIDE")
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

# --- multi-node tests (--multinode) -----------------------------------------
# Start-time spread in ms of "<host> <t>" lines (t = epoch seconds with fraction).
spread_ms() { awk 'NF { if (n++ == 0 || $2 < lo) lo = $2; if (n == 1 || $2 > hi) hi = $2 }
                   END { if (n) printf "%.0f", (hi - lo) * 1000 }'; }
# "node1:4 node2:4" from host names on stdin.
per_host() { sort | uniq -c | awk '{printf "%s%s:%s", (NR > 1 ? " " : ""), $2, $1}'; }

t08() { # Gang: one job over k nodes; every rank on a distinct node, all ranks start together.
  local k=$(( NODES < 3 ? NODES : 3 )) id ranks n hosts ends spread
  id=$(tk_submit t08 gang 2 "$(sc 120)" "$(sc 10)" 0 "" "$k")
  tk_wait "$(( $(sc 120) + GRACE ))" "$id"
  ranks=$(rank_fields "$id" start host t)
  n=$(printf '%s\n' "$ranks" | grep -c .)
  hosts=$(printf '%s\n' "$ranks" | awk 'NF {print $1}' | sort -u | grep -c .)
  ends=$(rank_fields "$id" end t | grep -c .)
  spread=$(printf '%s\n' "$ranks" | spread_ms)
  printf 't08_rank_start_spread_ms\t%s\n' "${spread:-}" >> "$TK_OUT/metrics_extra.tsv"
  if [ "$(tk_state "$id")" = COMPLETED ] && [ "$n" -eq "$k" ] && [ "$hosts" -eq "$k" ] &&
     [ "$ends" -eq "$k" ] && [ "${spread:-99999}" -le 2000 ]; then
    result t08 PASS "$k ranks on $hosts distinct nodes ($(printf '%s\n' "$ranks" | awk 'NF {print $1}' | per_host)); start spread ${spread} ms"
  else
    result t08 FAIL "state=$(tk_raw_state "$id") ranks started=$n ended=$ends distinct nodes=$hosts (want $k) spread=${spread:-?} ms"
  fi
}

t12() { # Gang failure: when one rank fails, the scheduler stops its peers and the job fails fast.
  local k=$(( NODES < 3 ? NODES : 3 )) id t0 took started peer_ends
  t0=$SECONDS
  id=$(tk_submit t12 failrank0 1 "$(sc 600)" "$(sc 300)" 3 "" "$k")
  tk_wait "$(( $(sc 300) + GRACE ))" "$id"
  took=$(( SECONDS - t0 ))
  started=$(rank_fields "$id" start t | grep -c .)
  # Peers that ran to the end would have written an end record; stopped ones did not.
  peer_ends=$(rank_fields "$id" end t | grep -c . || true)
  printf 't12_gang_failure_to_end_s\t%s\n' "$took" >> "$TK_OUT/metrics_extra.tsv"
  if [ "$(tk_state "$id")" = FAILED ] && [ "$started" -eq "$k" ] && [ "$peer_ends" -le 1 ] &&
     [ "$took" -lt 90 ]; then
    result t12 PASS "rank 0 exited 3 after 2 s; $((k - 1)) peers stopped, job FAILED ${took}s after submit"
  else
    result t12 FAIL "state=$(tk_raw_state "$id") started=$started end records=$peer_ends after ${took}s (want FAILED < 90 s, peers stopped)"
  fi
}

t09() { # Spread: one whole-node job per node; all on distinct nodes, all running at once.
  local ids=() i hosts first_end=0 last_start=0 s e
  for i in $(seq 1 "$NODES"); do ids+=("$(tk_submit t09 "whole$i" "$CORES" "$(sc 120)" "$(sc 30)")"); done
  tk_wait "$(( $(sc 120) + GRACE ))" "${ids[@]}"
  hosts=$(for i in "${ids[@]}"; do rec_host "$i"; done | sort -u | grep -c .)
  for i in "${ids[@]}"; do
    s=$(rec_start "$i"); e=$(rec_end "$i")
    [ -n "$s" ] && [ "$s" -gt "$last_start" ] && last_start=$s
    [ -n "$e" ] && { [ "$first_end" -eq 0 ] || [ "$e" -lt "$first_end" ]; } && first_end=$e
  done
  if [ "$hosts" -eq "$NODES" ] && [ "$first_end" -gt 0 ] && [ "$last_start" -lt "$first_end" ]; then
    result t09 PASS "$NODES whole-node jobs on $hosts distinct nodes, all running together"
  else result t09 FAIL "distinct nodes=$hosts (want $NODES); last start=$last_start first end=$first_end"; fi
}

t10() { # Fill: nodes x cores single-core jobs all run in one wave, cores-per-node on every node.
  local total=$(( NODES * CORES )) ids=() i t0 dist done=0 first_end=0 last_start=0 s e fill
  t0=$(date +%s)
  for i in $(seq 1 "$total"); do ids+=("$(tk_submit t10 "c$i" 1 "$(sc 120)" "$(sc 30)")"); done
  tk_wait "$(( $(sc 120) + GRACE ))" "${ids[@]}"
  for i in "${ids[@]}"; do
    s=$(rec_start "$i"); e=$(rec_end "$i")
    [ -n "$e" ] && done=$(( done + 1 ))
    [ -n "$s" ] && [ "$s" -gt "$last_start" ] && last_start=$s
    [ -n "$e" ] && { [ "$first_end" -eq 0 ] || [ "$e" -lt "$first_end" ]; } && first_end=$e
  done
  dist=$(for i in "${ids[@]}"; do rec_host "$i"; done | per_host)
  fill=$(( last_start - t0 ))
  printf 't10_fill_s\t%s\n' "$fill" >> "$TK_OUT/metrics_extra.tsv"
  local even=1 h
  for h in $dist; do [ "${h#*:}" = "$CORES" ] || even=0; done
  if [ "$done" -eq "$total" ] && [ "$last_start" -lt "$first_end" ] && [ "$even" = 1 ]; then
    result t10 PASS "$total jobs in one wave, all started ${fill}s after the first submit ($dist)"
  else result t10 FAIL "$done/$total completed; last start=$last_start first end=$first_end ($dist)"; fi
}

t11() { # Placement policy on an idle cluster (informational): do a few small jobs pack or spread?
  local ids=() i hosts dist
  for i in $(seq 1 "$NODES"); do ids+=("$(tk_submit t11 "small$i" 1 "$(sc 120)" "$(sc 20)")"); done
  tk_wait "$(( $(sc 120) + GRACE ))" "${ids[@]}"
  dist=$(for i in "${ids[@]}"; do rec_host "$i"; done | per_host)
  hosts=$(for i in "${ids[@]}"; do rec_host "$i"; done | sort -u | grep -c .)
  printf 't11_distinct_nodes\t%s\n' "$hosts" >> "$TK_OUT/metrics_extra.tsv"
  if [ "$hosts" -eq 1 ]; then result t11 INFO "packs: $NODES 1-core jobs on one node ($dist)"
  elif [ "$hosts" -eq "$NODES" ]; then result t11 INFO "spreads: $NODES 1-core jobs on $hosts nodes ($dist)"
  else result t11 INFO "mixed: $NODES 1-core jobs on $hosts nodes ($dist)"; fi
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
