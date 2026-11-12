#!/usr/bin/env bash
# Build the repository's graphify knowledge graph (./graphify-out/) from the directories
# listed in .github/graphify/targets.txt.
# Used by the VS Code tasks (manual) and by .github/workflows/graphify.yml (CI).
#
# One target:    graphify extract <dir> --out .   (writes ./graphify-out/ directly)
# Several:       graphify extract <dir> per target into <dir>/graphify-out/, then
#                graphify merge-graphs into ./graphify-out/graph.json + cluster-only .
if [ -n "${ZSH_VERSION:-}" ]; then
    exec bash "$0" "$@"
fi
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: .github/graphify/update.sh [options] [DIR ...]

Rebuild the knowledge graph in ./graphify-out/ (the location graphify's own tools expect).
With no DIR, every target in .github/graphify/targets.txt is rebuilt; a DIR must be listed there.
With several targets each one is also kept in DIR/graphify-out/ and merged into the root graph.

Options:
  --changed REF   only rebuild targets with files changed between REF and HEAD
                  (everything is rebuilt when ./graphify-out/graph.json is missing)
  --full          also run semantic (LLM) extraction on docs/papers/images (needs an API key)
  --code-only     local AST extraction only: no API key, no network
  --no-viz        skip graph.html generation
  --list          print the configured target directories and exit
  -h, --help      show this help

Mode precedence: --full/--code-only > the target's mode column > $GRAPHIFY_MODE > code-only.

Environment:
  GRAPHIFY_MODE   default mode (code-only|full)
  GRAPHIFY_BIN    graphify executable (default: graphify)
EOF
}

repo_root=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
cd "$repo_root"

targets_file=".github/graphify/targets.txt"
graphify_bin=${GRAPHIFY_BIN:-graphify}
forced_mode=""
changed_ref=""
no_viz=0
list_only=0
explicit_dirs=()

warn() {
    if [[ -n ${GITHUB_ACTIONS:-} ]]; then
        echo "::warning::$*"
    else
        echo "warning: $*" >&2
    fi
}

die() {
    if [[ -n ${GITHUB_ACTIONS:-} ]]; then
        echo "::error::$*"
    else
        echo "error: $*" >&2
    fi
    exit 1
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --changed)
            [[ $# -ge 2 ]] || die "--changed needs a git ref, e.g. --changed origin/main"
            changed_ref=$2
            shift 2
            ;;
        --full) forced_mode=full; shift ;;
        --code-only) forced_mode=code-only; shift ;;
        --no-viz) no_viz=1; shift ;;
        --list) list_only=1; shift ;;
        -h | --help) usage; exit 0 ;;
        --) shift; explicit_dirs+=("$@"); break ;;
        -*) die "unknown option '$1' (see --help)" ;;
        *) explicit_dirs+=("$1"); shift ;;
    esac
done

normalize_dir() {
    local d=${1%/}
    d=${d#./}
    [[ -z $d ]] && d="."
    printf '%s\n' "$d"
}

# Prints "dir<TAB>mode" for every configured target (mode may be empty).
read_targets() {
    [[ -f $targets_file ]] || return 0
    local line dir mode
    while IFS= read -r line || [[ -n $line ]]; do
        line=${line%%#*}
        read -r dir mode _ <<<"$line" || true
        [[ -z ${dir:-} ]] && continue
        printf '%s\t%s\n' "$(normalize_dir "$dir")" "${mode:-}"
    done <"$targets_file"
}

configured_mode_for() {
    local want=$1 dir mode
    while IFS=$'\t' read -r dir mode; do
        if [[ $dir == "$want" ]]; then
            printf '%s\n' "$mode"
            return 0
        fi
    done < <(read_targets)
    printf '\n'
}

if [[ $list_only -eq 1 ]]; then
    read_targets | cut -f1
    exit 0
fi

root_out="graphify-out"
root_graph="$root_out/graph.json"

# True when files under $1 (excluding graphify output) are in the changed-file list.
dir_changed() {
    local dir=$1
    awk -v p="$dir/" -v root="$([[ $dir == "." ]] && echo 1 || echo 0)" '
        /(^|\/)graphify-out\// { next }
        root == 1 || index($0, p) == 1 { found = 1 }
        END { exit !found }
    ' <<<"$changed_files"
}

is_target() {
    local want=$1 dir mode
    while IFS=$'\t' read -r dir mode; do
        [[ $dir == "$want" ]] && return 0
    done < <(read_targets)
    return 1
}

# Every configured target ("dir<TAB>mode"); the root graph always covers all of them.
all_targets=()
while IFS= read -r entry; do
    all_targets+=("$entry")
done < <(read_targets)

# The work list: targets selected for re-extraction.
work=()
if [[ ${#explicit_dirs[@]} -gt 0 ]]; then
    for d in "${explicit_dirs[@]}"; do
        d=$(normalize_dir "$d")
        is_target "$d" || die "'$d' is not listed in $targets_file. Add it there first: the root graph combines all targets."
        work+=("$d"$'\t'"$(configured_mode_for "$d")")
    done
elif [[ ${#all_targets[@]} -gt 0 ]]; then
    work=("${all_targets[@]}")
fi

if [[ -n $changed_ref && ! -f $root_graph && ${#all_targets[@]} -gt 0 ]]; then
    echo "$root_graph does not exist yet: rebuilding every target."
    changed_ref=""
    work=("${all_targets[@]}")
fi

if [[ -n $changed_ref ]]; then
    if git rev-parse --verify --quiet "${changed_ref}^{commit}" >/dev/null; then
        changed_files=$(git diff --name-only "$changed_ref" HEAD --)
        # Ignore and config files can change a target's scope without touching any file
        # beneath it (e.g. the root .graphifyignore), so treat them as invalidating all.
        scope_changes=$(grep -E '(^|/)(\.graphifyignore|\.gitignore|\.graphifyrc[^/]*)$' <<<"$changed_files" \
            | grep -vE '(^|/)graphify-out/' || true)
        if grep -qxF "$targets_file" <<<"$changed_files"; then
            echo "$targets_file changed: rebuilding every selected target."
        elif [[ -n $scope_changes ]]; then
            echo "ignore/config file(s) changed ($(tr '\n' ' ' <<<"$scope_changes")): rebuilding every selected target."
        else
            filtered=()
            for entry in "${work[@]}"; do
                if dir_changed "${entry%%$'\t'*}"; then
                    filtered+=("$entry")
                fi
            done
            work=("${filtered[@]+"${filtered[@]}"}")
        fi
    else
        warn "cannot resolve '$changed_ref' (first push or force-push?): rebuilding every selected target."
    fi
fi

if [[ ${#work[@]} -eq 0 ]]; then
    echo "graphify: nothing to rebuild."
    exit 0
fi

if ! command -v "$graphify_bin" >/dev/null 2>&1; then
    die "graphify CLI not found. Install it with: pipx install graphifyy (or: uv tool install graphifyy, pip install graphifyy)"
fi

any_env_set() {
    local v
    for v in "$@"; do
        [[ -n ${!v:-} ]] && return 0
    done
    return 1
}

# True when the URL's host is loopback (graphify only lets Ollama run keyless locally).
is_local_url() {
    local host=${1#*://}
    host=${host%%/*}
    host=${host##*@}
    case $host in
        \[::1\]* | localhost | localhost:* | 127.*) return 0 ;;
        *) return 1 ;;
    esac
}

# Mirrors graphify's own credential checks (graphify/llm.py BACKENDS + `extract`), so a
# "full" target falls back to code-only instead of failing when its backend can't run.
have_llm_backend() {
    local backend=${GRAPHIFY_BACKEND:-}
    case $backend in
        none) return 1 ;; # disables every LLM step, even when keys are present
        "") any_env_set ANTHROPIC_API_KEY OPENAI_API_KEY GEMINI_API_KEY GOOGLE_API_KEY \
            MOONSHOT_API_KEY DEEPSEEK_API_KEY ;;
        claude) any_env_set ANTHROPIC_API_KEY ;;
        openai) any_env_set OPENAI_API_KEY ;;
        gemini) any_env_set GEMINI_API_KEY GOOGLE_API_KEY ;;
        kimi) any_env_set MOONSHOT_API_KEY ;;
        deepseek) any_env_set DEEPSEEK_API_KEY ;;
        azure) any_env_set AZURE_OPENAI_API_KEY ;;
        ollama) any_env_set OLLAMA_API_KEY || is_local_url "${OLLAMA_BASE_URL:-http://localhost:11434/v1}" ;;
        bedrock) any_env_set AWS_PROFILE AWS_REGION AWS_DEFAULT_REGION AWS_ACCESS_KEY_ID ;;
        claude-cli) command -v claude >/dev/null 2>&1 ;;
        *) custom_provider_ready "$backend" ;;
    esac
}

# Prints the API-key env var names a custom provider (`graphify provider add`) declares.
# Exit 0 = provider found; 2 = no such provider; 3 = python3 unavailable.
# Reads the same files as graphify: ~/.graphify/providers.json, plus ./.graphify/providers.json
# only when GRAPHIFY_ALLOW_LOCAL_PROVIDERS is truthy.
custom_provider_env_keys() {
    command -v python3 >/dev/null 2>&1 || return 3
    python3 -I - "$1" <<'PY'
import json, os, re, sys
from pathlib import Path

name = sys.argv[1]
paths = [Path.home() / ".graphify" / "providers.json"]
if os.environ.get("GRAPHIFY_ALLOW_LOCAL_PROVIDERS", "").strip().lower() in ("1", "true", "yes"):
    paths.insert(0, Path(".graphify") / "providers.json")
for path in paths:
    try:
        cfg = json.loads(path.read_text(encoding="utf-8")).get(name)
    except (OSError, ValueError, AttributeError):
        continue
    if not isinstance(cfg, dict) or not str(cfg.get("base_url", "")).startswith(("http://", "https://")):
        continue
    keys = cfg.get("env_keys") or ([cfg["env_key"]] if cfg.get("env_key") else [])
    for key in keys if isinstance(keys, list) else []:
        if isinstance(key, str) and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key):
            print(key)
    sys.exit(0)
sys.exit(2)
PY
}

custom_provider_ready() {
    local name=$1 keys status=0
    keys=$(custom_provider_env_keys "$name") || status=$?
    case $status in
        0) ;;
        2) warn "GRAPHIFY_BACKEND='$name' is not a built-in backend or a configured custom provider (typo?)."
           return 1 ;;
        *) warn "cannot verify custom provider '$name' (python3 not found)."
           return 1 ;;
    esac
    if [[ -z $keys ]]; then
        warn "custom provider '$name' declares no env_key, so graphify cannot authenticate it."
        return 1
    fi
    # shellcheck disable=SC2086 # one validated variable name per line
    any_env_set $keys
}

# Sets $mode to the effective mode (code-only|full) for "dir<TAB>mode", applying the
# precedence rules and the credential fallback.
effective_mode() {
    local dir=${1%%$'\t'*}
    mode=${1#*$'\t'}
    mode=${forced_mode:-${mode:-${GRAPHIFY_MODE:-code-only}}}
    case $mode in
        code-only | full) ;;
        *) warn "'$dir' has unknown mode '$mode': using code-only"; mode=code-only ;;
    esac
    if [[ $mode == full ]] && ! have_llm_backend; then
        warn "'$dir' wants full extraction but backend '${GRAPHIFY_BACKEND:-auto-detect}' has no usable credentials (or is 'none'): falling back to code-only."
        mode=code-only
    fi
}

# extract <dir> [--out .] in the given mode.
extract_target() {
    local dir=$1 mode=$2
    shift 2
    if [[ ! -d $dir ]]; then
        warn "target '$dir' does not exist (fix $targets_file)"
        return 1
    fi
    local args=(extract "$dir" "$@")
    [[ $mode == code-only ]] && args+=(--code-only)
    echo "==> graphify extract $dir ($mode)"
    "$graphify_bin" "${args[@]}"
}

# cluster-only on the root graph: writes GRAPH_REPORT.md and graph.html next to it.
# Communities are only named by an LLM when at least one target was built in full mode.
cluster_root() {
    local label=$1
    local args=(cluster-only . --graph "$root_graph")
    [[ $label -eq 1 ]] || args+=(--no-label)
    [[ $no_viz -eq 1 ]] && args+=(--no-viz)
    echo "==> graphify cluster-only (root graph)"
    "$graphify_bin" "${args[@]}"
}

in_work() {
    local want=$1 entry
    for entry in "${work[@]}"; do
        [[ ${entry%%$'\t'*} == "$want" ]] && return 0
    done
    return 1
}

if [[ ${#all_targets[@]} -eq 1 ]]; then
    # Single target: scan it, write straight to ./graphify-out/. Node source_file paths
    # are relative to the target directory.
    entry=${all_targets[0]}
    dir=${entry%%$'\t'*}
    effective_mode "$entry"
    if [[ $dir != "." && -d $dir/$root_out ]]; then
        warn "$dir/$root_out is left over from the multi-target or older per-directory layout and is no longer used: delete it."
    fi
    extract_target "$dir" "$mode" --out . || die "graphify failed for: $dir"
    label=0
    [[ $mode == full ]] && label=1
    cluster_root "$label" || die "graphify cluster-only failed for the root graph"
else
    # Several targets: (re)build each selected one into <dir>/graphify-out/ (plus any
    # that have never been built), then merge every target into the root graph.
    failed=()
    label=0
    inputs=()
    for entry in "${all_targets[@]}"; do
        dir=${entry%%$'\t'*}
        effective_mode "$entry"
        [[ $mode == full ]] && label=1
        if in_work "$dir" || [[ ! -f $dir/$root_out/graph.json ]]; then
            extract_target "$dir" "$mode" || { failed+=("$dir"); continue; }
        fi
        inputs+=("$dir/$root_out/graph.json")
    done
    if [[ ${#failed[@]} -gt 0 ]]; then
        die "graphify failed for: ${failed[*]} (root graph not updated)"
    fi
    merge_args=(merge-graphs "${inputs[@]}" --out "$root_graph")
    # Keep the previous community ids so cluster-only can reuse existing labels.
    [[ -f $root_graph ]] && merge_args+=(--previous "$root_graph")
    mkdir -p "$root_out"
    echo "==> graphify merge-graphs (${#inputs[@]} targets -> $root_graph)"
    "$graphify_bin" "${merge_args[@]}" || die "graphify merge-graphs failed"
    cluster_root "$label" || die "graphify cluster-only failed for the root graph"
fi
echo "graphify: done ($root_graph)."
