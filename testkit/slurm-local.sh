#!/usr/bin/env bash
# Start/stop a single-machine Slurm cluster (slurmctld + slurmd + munge) so the test kit can run
# Gnomon and Slurm side by side on the same host, e.g. a CI runner.
#
#   sudo ./slurm-local.sh start    # writes slurm.conf, starts munged, slurmctld, slurmd
#   ./slurm-local.sh status
#   sudo ./slurm-local.sh stop
#
# Needs the Ubuntu/Debian packages `slurm-wlm` and `munge` (run as root: the daemons drop to the
# `slurm` user). Env: SLURM_CORES (8: CPUs advertised for the node, matching local-cluster.sh's
# GNOMON_CORES; may exceed the real core count), SLURM_MEM_MB (16384), SLURM_PARTITION (debug).
#
# No slurmdbd: jobs stay queryable through `scontrol show job` for MinJobAge (1 h), which the test
# kit falls back to when `sacct` has no accounting storage. This is a one-node Slurm, not a
# production cluster: compare scheduler behaviour and latency, not scale.
set -euo pipefail
cd "$(dirname "$0")"
CONF_DIR=${SLURM_CONF_DIR:-/etc/slurm}
STATE=/var/spool/slurmctld
SPOOL=/var/spool/slurmd
LOGS=${SLURM_LOG_DIR:-/var/log/slurm}
CORES=${SLURM_CORES:-8}
MEM=${SLURM_MEM_MB:-16384}
PARTITION=${SLURM_PARTITION:-debug}
NODE=$(hostname -s)

write_conf() {
  mkdir -p "$CONF_DIR" "$STATE" "$SPOOL" "$LOGS"
  chown slurm:slurm "$STATE" "$LOGS"
  cat > "$CONF_DIR/slurm.conf" <<EOF
ClusterName=testkit
SlurmctldHost=$NODE
SlurmUser=slurm
AuthType=auth/munge
StateSaveLocation=$STATE
SlurmdSpoolDir=$SPOOL
SlurmctldLogFile=$LOGS/slurmctld.log
SlurmdLogFile=$LOGS/slurmd.log
SlurmctldPidFile=/run/slurmctld.pid
SlurmdPidFile=/run/slurmd.pid
# No cgroup plugins: CI containers rarely delegate cgroups (matches Gnomon without confinement).
ProctrackType=proctrack/linuxproc
TaskPlugin=task/none
JobAcctGatherType=jobacct_gather/none
AccountingStorageType=accounting_storage/none
SchedulerType=sched/backfill
SchedulerParameters=bf_interval=5,bf_continue
SelectType=select/cons_tres
SelectTypeParameters=CR_Core_Memory
MinJobAge=3600
ReturnToService=2
SlurmdParameters=config_overrides
NodeName=$NODE CPUs=$CORES RealMemory=$MEM State=UNKNOWN
PartitionName=$PARTITION Nodes=$NODE Default=YES MaxTime=INFINITE State=UP
EOF
}

start() {
  [ "$(id -u)" -eq 0 ] || { echo "run as root (sudo)" >&2; exit 2; }
  command -v slurmctld >/dev/null || { echo "install slurm-wlm and munge first" >&2; exit 2; }
  write_conf
  [ -s /etc/munge/munge.key ] || { mungekey -c -f 2>/dev/null || create-munge-key -f; }
  chown munge:munge /etc/munge/munge.key; chmod 400 /etc/munge/munge.key
  mkdir -p /run/munge; chown munge:munge /run/munge
  pgrep -x munged >/dev/null || su -s /bin/sh munge -c munged
  slurmctld
  slurmd
  for _ in $(seq 1 30); do
    sinfo -h -o %T 2>/dev/null | grep -qE '^(idle|mixed|allocated)' && break
    scontrol update NodeName="$NODE" State=RESUME >/dev/null 2>&1 || true
    sleep 1
  done
  sinfo
}

stop() {
  scontrol shutdown >/dev/null 2>&1 || true
  pkill -x slurmd 2>/dev/null || true
  pkill -x slurmctld 2>/dev/null || true
  echo "stopped"
}

case ${1:-} in
  start) start ;;
  stop) stop ;;
  status) sinfo && squeue ;;
  *) sed -n 2,16p "$0"; exit 2 ;;
esac
