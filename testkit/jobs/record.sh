#!/bin/sh
# Portable job body used by every test. Writes one record line on start and
# one on finish to $TESTKIT_OUT/<jobid>.rec, then sleeps / exits as asked.
#   usage (as job script): record.sh <test> <tag> <sleep_s> [exit_code] [output_dir]
# Works unchanged under Slurm (SLURM_* env) and Gnomon (GNOMON_* env).
set -u
TEST=${1:-adhoc}; TAG=${2:-job}; SLEEP=${3:-1}; CODE=${4:-0}
JOBID=${SLURM_JOB_ID:-${GNOMON_JOB_ID:-unknown}}
CPUS=${SLURM_CPUS_PER_TASK:-${GNOMON_CPUS:-?}}
OUT=${5:-${TESTKIT_OUT:-$(cd "$(dirname "$0")/.." && pwd)/out/current}}
mkdir -p "$OUT"
REC="$OUT/$JOBID.rec"
printf 'test=%s tag=%s jobid=%s event=start t=%s host=%s cpus=%s nproc=%s\n' \
  "$TEST" "$TAG" "$JOBID" "$(date +%s)" "$(hostname)" "$CPUS" "$(nproc 2>/dev/null || echo ?)" >> "$REC"
sleep "$SLEEP"
printf 'test=%s tag=%s jobid=%s event=end t=%s code=%s\n' "$TEST" "$TAG" "$JOBID" "$(date +%s)" "$CODE" >> "$REC"
exit "$CODE"
