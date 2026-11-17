#!/bin/sh
# Multi-node (gang) job body. Every rank writes one record line on start and one on finish to
# $OUT/<jobid>.rank<r>.rec; rank 0 also writes the ordinary <jobid>.rec that record.sh writes, so
# the single-job helpers (rec_start, rec_end) work for gangs too.
#   usage (as job script): gang.sh <test> <tag> <sleep_s> [exit_code] [output_dir]
#
# Gnomon starts this script on every node of the gang (GNOMON_NODE_RANK, GNOMON_NODELIST,
# GNOMON_JOB_START). Slurm runs a batch script on the first node only, so there the batch step
# re-runs itself through `srun`, one task per node (SLURM_NODEID). Slurm copies the batch script to
# its spool directory, which other nodes do not have, so the kit passes the shared path in TK_GANG.
set -u
if [ -n "${SLURM_JOB_ID:-}" ] && [ -z "${TK_GANG_STEP:-}" ]; then
  TK_GANG_STEP=1; export TK_GANG_STEP
  exec srun --kill-on-bad-exit=1 "${TK_GANG:-$0}" "$@"
fi
TEST=${1:-adhoc}; TAG=${2:-job}; SLEEP=${3:-1}; CODE=${4:-0}
JOBID=${SLURM_JOB_ID:-${GNOMON_JOB_ID:-unknown}}
RANK=${SLURM_NODEID:-${GNOMON_NODE_RANK:-0}}
NNODES=${SLURM_JOB_NUM_NODES:-${GNOMON_NNODES:-1}}
NODELIST=${SLURM_JOB_NODELIST:-${GNOMON_NODELIST:-}}
CPUS=${SLURM_CPUS_PER_TASK:-${GNOMON_CPUS:-?}}
# Tag "failrank0": rank 0 exits with the code after 2 s, the other ranks would sleep the full time
# and exit 0, unless the scheduler stops them when rank 0 fails (t12).
if [ "$TAG" = failrank0 ]; then
  if [ "$RANK" = 0 ]; then SLEEP=2; else CODE=0; fi
fi
OUT=${5:-${TESTKIT_OUT:-$(cd "$(dirname "$0")/.." && pwd)/out/current}}
mkdir -p "$OUT"
REC="$OUT/$JOBID.rank$RANK.rec"
NOW=$(date +%s.%N)
printf 'test=%s tag=%s jobid=%s rank=%s nnodes=%s event=start t=%s host=%s nodelist=%s cpus=%s\n' \
  "$TEST" "$TAG" "$JOBID" "$RANK" "$NNODES" "$NOW" "$(hostname)" "$NODELIST" "$CPUS" >> "$REC"
[ "$RANK" = 0 ] && printf 'test=%s tag=%s jobid=%s event=start t=%s host=%s cpus=%s nproc=%s\n' \
  "$TEST" "$TAG" "$JOBID" "${NOW%.*}" "$(hostname)" "$CPUS" "$(nproc 2>/dev/null || echo ?)" >> "$OUT/$JOBID.rec"
sleep "$SLEEP"
NOW=$(date +%s.%N)
printf 'test=%s tag=%s jobid=%s rank=%s event=end t=%s code=%s\n' "$TEST" "$TAG" "$JOBID" "$RANK" "$NOW" "$CODE" >> "$REC"
[ "$RANK" = 0 ] && printf 'test=%s tag=%s jobid=%s event=end t=%s code=%s\n' \
  "$TEST" "$TAG" "$JOBID" "${NOW%.*}" "$CODE" >> "$OUT/$JOBID.rec"
exit "$CODE"
