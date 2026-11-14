#!/usr/bin/env python3
"""A/B the controller-path benchmark (occ_bench) of a PR against its base on one machine.

usage: occ_ab.py BASE_BIN PR_BIN [--runs N] [--min-ratio R] [--summary FILE]

Runs the two binaries alternately N times (default 3), sums jobs_per_sec over every row of each
run, and compares the medians. Exits 1 if PR/base < min-ratio (default 0.85). Absolute timings on
shared CI runners are noisy; interleaved runs of both builds on the same runner are not.
"""
import argparse
import csv
import io
import statistics
import subprocess
import sys


def total_throughput(binary: str) -> int:
    try:
        out = subprocess.run([binary], check=True, capture_output=True, text=True, timeout=900).stdout
    except (OSError, subprocess.SubprocessError) as e:
        sys.exit(f"occ_ab: running {binary} failed: {e}")
    rows = list(csv.DictReader(io.StringIO(out)))
    if not rows:
        sys.exit(f"occ_ab: {binary} printed no rows")
    return sum(int(r["jobs_per_sec"]) for r in rows)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("base")
    ap.add_argument("pr")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--min-ratio", type=float, default=0.85)
    ap.add_argument("--summary")
    a = ap.parse_args()
    base, pr = [], []
    for i in range(a.runs):
        base.append(total_throughput(a.base))
        pr.append(total_throughput(a.pr))
        print(f"run {i + 1}: base {base[-1]} jobs/s, PR {pr[-1]} jobs/s", flush=True)
    ratio = statistics.median(pr) / statistics.median(base)
    ok = ratio >= a.min_ratio
    md = (
        "## Controller throughput A/B (`occ_bench`, summed over all rows)\n\n"
        f"| build | runs (jobs/s) | median |\n|---|---|---|\n"
        f"| base | {', '.join(map(str, base))} | {statistics.median(base):.0f} |\n"
        f"| PR | {', '.join(map(str, pr))} | {statistics.median(pr):.0f} |\n\n"
        f"PR / base = **{ratio:.3f}** (floor {a.min_ratio}): {'ok' if ok else '**regressed**'}\n"
    )
    print(md)
    if a.summary:
        with open(a.summary, "a", encoding="utf-8") as f:
            f.write(md)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
