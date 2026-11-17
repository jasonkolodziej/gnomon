# shellcheck shell=bash
# Backend abstraction: the same tests drive Gnomon (gnbatch/gnqueue/gncancel)
# or Slurm (sbatch/squeue/sacct/scancel). Sourced by run.sh.
#
# Environment:
#   TK_BACKEND        gnomon | slurm
#   TK_OUT            results directory for this run (absolute)
#   SLURM_PARTITION   Slurm partition (default compute1, e.g. UTSA Arc)
#   SLURM_ACCOUNT     optional -A account
#   SLURM_NODE        optional node to pin node-sensitive tests to (-w)
#   SLURM_EXTRA       optional extra sbatch flags (e.g. "--qos=debug")

TK_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
JOB="$TK_DIR/jobs/record.sh"
GANG_JOB="$TK_DIR/jobs/gang.sh"

die() { echo "error: $*" >&2; exit 2; }
log() { printf '[%s] %s\n' "$(date +%H:%M:%S)" "$*" >&2; }

# Seconds -> Slurm time "MM:SS" (minutes may exceed 59, Slurm accepts it).
slurm_time() { printf '%d:%02d' $(( $1 / 60 )) $(( $1 % 60 )); }

# tk_submit <test> <tag> <cpus> <limit_s> <sleep_s> [exit_code] [pin] [nodes]
# Prints the job id. "pin" = run on the pinned test node (Slurm: -w; single-node jobs only).
# nodes > 1 submits a gang (gnbatch --nodes, sbatch -N) that runs jobs/gang.sh on every node;
# <cpus> is then per node. With nodes = 1 the submission is exactly the single-node one.
tk_submit() {
  local test=$1 tag=$2 cpus=$3 limit=$4 sleep_s=$5 code=${6:-0} pin=${7:-} nodes=${8:-1}
  local name="tk-$test-$tag" id script=$JOB
  [ "$nodes" -gt 1 ] && script=$GANG_JOB
  case $TK_BACKEND in
    gnomon)
      local gang=()
      [ "$nodes" -gt 1 ] && gang=(--nodes "$nodes")
      id=$(gnbatch --name "$name" --cpus "$cpus" --mem-mb 256 --time "$limit" "${gang[@]}" \
             "$script" "$test" "$tag" "$sleep_s" "$code" "$TK_OUT" | awk '{print $NF}') ;;
    slurm)
      # shellcheck disable=SC2054  # --export=ALL,VAR=... is one Slurm argument, not two elements
      local args=(--parsable -J "$name" -N "$nodes" -n "$nodes" -c "$cpus" --mem=256M -t "$(slurm_time "$limit")"
                  -p "${SLURM_PARTITION:-compute1}" -o "$TK_OUT/slurm-%j.out"
                  --export=ALL,TESTKIT_OUT="$TK_OUT",TK_GANG="$GANG_JOB")
      [ "$nodes" -gt 1 ] && args+=(--ntasks-per-node=1)
      [ -n "${SLURM_ACCOUNT:-}" ] && args+=(-A "$SLURM_ACCOUNT")
      [ -n "$pin" ] && [ "$nodes" -eq 1 ] && [ -n "${SLURM_NODE:-}" ] && args+=(-w "$SLURM_NODE")
      # shellcheck disable=SC2206
      [ -n "${SLURM_EXTRA:-}" ] && args+=($SLURM_EXTRA)
      id=$(sbatch "${args[@]}" "$script" "$test" "$tag" "$sleep_s" "$code" "$TK_OUT" | cut -d';' -f1) ;;
    *) die "TK_BACKEND must be gnomon or slurm" ;;
  esac
  [ -n "$id" ] || die "submit failed for $name"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$id" "$test" "$tag" "$(date +%s)" "$cpus" "$limit" >> "$TK_OUT/submits.tsv"
  echo "$id"
}

# Job state from the controller's memory (finished jobs stay for MinJobAge). Used when there is
# no accounting storage, e.g. the single-node Slurm of slurm-local.sh, where sacct has nothing.
slurm_scontrol_state() {
  scontrol show job -o "$1" 2>/dev/null | tr ' ' '\n' | awk -F= '$1=="JobState" {print $2; exit}'
}

# Normalised state: PENDING RUNNING COMPLETED FAILED CANCELLED UNKNOWN
tk_state() {
  local id=$1 raw=""
  case $TK_BACKEND in
    gnomon) raw=$(gnqueue 2>/dev/null | awk -v id="$id" '$1==id {print $3}') ;;
    slurm)
      raw=$(squeue -h -j "$id" -o %T 2>/dev/null | head -1)
      [ -z "$raw" ] && raw=$(sacct -n -X -P -j "$id" -o State 2>/dev/null | head -1 | awk '{print $1}')
      [ -z "$raw" ] && raw=$(slurm_scontrol_state "$id") ;;
  esac
  case $raw in
    PENDING|RESERVED|CONFIGURING|REQUEUED) echo PENDING ;;
    RUNNING|COMPLETING) echo RUNNING ;;
    COMPLETED) echo COMPLETED ;;
    FAILED|TIMEOUT|NODE_FAIL|OUT_OF_MEMORY|PREEMPTED|BOOT_FAIL|DEADLINE) echo FAILED ;;
    CANCELLED*) echo CANCELLED ;;
    *) echo UNKNOWN ;;
  esac
}

# Raw backend state (for the report).
tk_raw_state() {
  case $TK_BACKEND in
    gnomon) gnqueue 2>/dev/null | awk -v id="$1" '$1==id {print $3}' ;;
    slurm) { squeue -h -j "$1" -o %T 2>/dev/null; sacct -n -X -P -j "$1" -o State 2>/dev/null
             slurm_scontrol_state "$1"; } | grep -v '^$' | head -1 | awk '{print $1}' ;;
  esac
}

tk_cancel() {
  case $TK_BACKEND in gnomon) gncancel "$1" ;; slurm) scancel "$1" ;; esac
}

# tk_wait <timeout_s> <id...>: wait until every job is terminal.
tk_wait() {
  local timeout=$1; shift
  local deadline=$(( $(date +%s) + timeout )) id s pending
  while :; do
    pending=0
    for id in "$@"; do
      s=$(tk_state "$id")
      case $s in COMPLETED|FAILED|CANCELLED) ;; *) pending=1 ;; esac
    done
    [ $pending -eq 0 ] && return 0
    [ "$(date +%s)" -ge $deadline ] && return 1
    sleep 2
  done
}

# tk_wait_running <timeout_s> <id>
tk_wait_running() {
  local deadline=$(( $(date +%s) + $1 )) s
  while [ "$(date +%s)" -lt $deadline ]; do
    s=$(tk_state "$2"); [ "$s" = RUNNING ] || [ "$s" = COMPLETED ] && return 0
    sleep 1
  done
  return 1
}

# Record lookups (written by jobs/record.sh).
rec_field() { # rec_field <id> <event> <field>
  [ -f "$TK_OUT/$1.rec" ] || return 0
  awk -v ev="event=$2" -v f="$3=" '$0 ~ ev { for (i=1;i<=NF;i++) if (index($i,f)==1) print substr($i,length(f)+1) }' "$TK_OUT/$1.rec" | head -1
}
rec_start() { rec_field "$1" start t; }
rec_end()   { rec_field "$1" end t; }
rec_host()  { rec_field "$1" start host; }

# Gang rank records (written by jobs/gang.sh): one line per rank with the asked fields of its
# <event> line, space separated. rank_fields <id> <event> <field...>
rank_fields() {
  local id=$1 ev=$2 f; shift 2
  for f in "$TK_OUT/$id".rank*.rec; do
    [ -f "$f" ] || continue
    awk -v ev="event=$ev" -v want="$*" '$0 ~ ev {
      n = split(want, w, " "); line = ""
      for (j = 1; j <= n; j++) for (i = 1; i <= NF; i++)
        if (index($i, w[j] "=") == 1) line = line (j > 1 ? " " : "") substr($i, length(w[j]) + 2)
      print line; exit }' "$f"
  done
}

# Cores of the node used for node-sensitive tests.
tk_node_cores() {
  case $TK_BACKEND in
    gnomon) gninfo 2>/dev/null | awk 'NR==2 {print $2}' ;;
    slurm)
      if [ -n "${SLURM_NODE:-}" ]; then sinfo -h -n "$SLURM_NODE" -o %c | head -1
      else sinfo -h -p "${SLURM_PARTITION:-compute1}" -o %c | sort -n | head -1; fi ;;
  esac
}

# Nodes of the test partition that are up (multi-node tests need at least 2).
tk_node_count() {
  case $TK_BACKEND in
    gnomon) gninfo 2>/dev/null | awk 'NR > 1 && $NF != "down*"' | wc -l ;;
    slurm) sinfo -h -N -p "${SLURM_PARTITION:-compute1}" -o '%N %t' 2>/dev/null |
             awk '$2 !~ /down|drain|fail/ {print $1}' | sort -u | wc -l ;;
  esac
}

result() { # result <test> <PASS|FAIL|INFO|SKIP> <detail>
  printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$TK_OUT/summary.tsv"
  log "$1: $2 — $3"
}

# Scale durations: TK_SCALE=0.25 for quick local runs (min 2 s).
sc() { awk -v s="$1" -v k="${TK_SCALE:-1}" 'BEGIN { v = int(s * k + 0.5); if (v < 2) v = 2; print v }'; }
