#!/usr/bin/env bash
# Validate Gnomon's job confinement on a real host: the agent's launcher against the kernel's
# cgroup v2 and a real container runtime (crates/agent/tests/host_validation.rs).
#
#   testkit/validate-host.sh                 # cgroup v2 and container checks
#   testkit/validate-host.sh cgroup          # only cgroup_* (limits, OOM, CPU cap, kills)
#   testkit/validate-host.sh container       # only container_* (env, user, limits, OOM, stops)
#
# Needs root (re-runs itself under sudo), a cgroup v2 hierarchy with the cpu and memory
# controllers at GNOMON_CGROUP_ROOT (/sys/fs/cgroup), and Docker or Podman
# (GNOMON_CONTAINER_RUNTIME, default docker). Pulls GNOMON_VALIDATE_IMAGE (alpine:3.20) once.
# A missing prerequisite FAILS the run with the reason: nothing is skipped silently.
set -euo pipefail
self=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
cd "$(dirname "$self")/.."
filter=${1:-}
case "$filter" in "" | cgroup | container) ;; *) echo "usage: $0 [cgroup|container]" >&2; exit 2 ;; esac

if [ "$(id -u)" -ne 0 ]; then
  # Build as the invoking user so target/ stays theirs; run the tests as root.
  cargo test -p gnomon-agent --test host_validation --no-run --quiet
  exec sudo --preserve-env=PATH,HOME,CARGO_HOME,RUSTUP_HOME,CARGO_TARGET_DIR,GNOMON_CGROUP_ROOT,GNOMON_CONTAINER_RUNTIME,GNOMON_CONTAINER_ROOTLESS,GNOMON_VALIDATE_IMAGE \
    "$self" "$@"
fi

root=${GNOMON_CGROUP_ROOT:-/sys/fs/cgroup}
runtime=${GNOMON_CONTAINER_RUNTIME:-docker}
echo "== host"
echo "kernel:      $(uname -r)"
echo "cgroup root: $root ($(stat -fc %T "$root" 2>/dev/null || echo missing))"
echo "controllers: $(cat "$root/cgroup.controllers" 2>/dev/null || echo 'none (not cgroup v2)')"
echo "runtime:     $("$runtime" --version 2>/dev/null || echo "$runtime not found")"
echo "cpus:        $(nproc)"
echo

cargo test -p gnomon-agent --test host_validation -- --ignored --test-threads=1 ${filter:+"${filter}_"}
