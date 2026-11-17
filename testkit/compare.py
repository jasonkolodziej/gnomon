#!/usr/bin/env python3
"""Compare two test-kit runs (e.g. Gnomon vs Slurm) as a Markdown table.

usage: ./compare.py out/gnomon-<ts> out/slurm-<ts> [> comparison.md]
"""
import csv, statistics, sys
from pathlib import Path


def load(d):
    d = Path(d)
    summary = {r[0]: (r[1], r[2]) for r in csv.reader(open(d / "summary.tsv"), delimiter="\t") if r}
    metrics = list(csv.DictReader(open(d / "metrics.tsv"), delimiter="\t"))
    extra = {}
    if (d / "metrics_extra.tsv").exists():
        extra = {r[0]: r[1] for r in csv.reader(open(d / "metrics_extra.tsv"), delimiter="\t") if r}
    info = (d / "run.info").read_text().split("\n")[0] if (d / "run.info").exists() else d.name
    return summary, metrics, extra, info


def med(vals):
    v = [int(x) for x in vals if x not in ("", None)]
    return f"{statistics.median(v):.0f}" if v else "—"


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    a, b = load(sys.argv[1]), load(sys.argv[2])
    na, nb = Path(sys.argv[1]).name, Path(sys.argv[2]).name
    print(f"# Test-kit comparison\n\n- A: `{na}` ({a[3]})\n- B: `{nb}` ({b[3]})\n")
    print(f"| test | A result | B result | A detail | B detail |\n|---|---|---|---|---|")
    for t in sorted(set(a[0]) | set(b[0])):
        ra, rb = a[0].get(t, ("—", "")), b[0].get(t, ("—", ""))
        print(f"| {t} | {ra[0]} | {rb[0]} | {ra[1]} | {rb[1]} |")
    print("\n| metric | A | B |\n|---|---|---|")
    for t, label in [("t01", "t01 queue wait (s)"), ("t05", "t05 median wait (s)")]:
        print(f"| {label} | {med(r['wait_s'] for r in a[1] if r['test'] == t)} | "
              f"{med(r['wait_s'] for r in b[1] if r['test'] == t)} |")
    for k, label in [("t05_submit_rate", "t05 submit rate (jobs/s)"), ("t05_drain_s", "t05 drain time (s)")]:
        print(f"| {label} | {a[2].get(k, '—')} | {b[2].get(k, '—')} |")
    # Multi-node runs (run.sh --multinode) add these; single-node runs print exactly as before.
    multi = [("t08", "t08 gang queue wait (s)"), ("t09", "t09 whole-node median wait (s)"),
             ("t10", "t10 fill median wait (s)")]
    for t, label in multi:
        wa = [r["wait_s"] for r in a[1] if r["test"] == t]
        wb = [r["wait_s"] for r in b[1] if r["test"] == t]
        if wa or wb:
            print(f"| {label} | {med(wa)} | {med(wb)} |")
    labels = {"t08_rank_start_spread_ms": "t08 gang rank start spread (ms)",
              "t10_fill_s": "t10 time to fill every core (s)",
              "t11_distinct_nodes": "t11 nodes used by N small jobs",
              "t12_gang_failure_to_end_s": "t12 failed gang: submit to job end (s)"}
    for k in sorted((set(a[2]) | set(b[2])) - {"t05_submit_rate", "t05_drain_s"}):
        print(f"| {labels.get(k, k)} | {a[2].get(k, '—')} | {b[2].get(k, '—')} |")
    print("\nWaits include scheduler poll intervals (Gnomon agent heartbeat, Slurm "
          "sched/backfill intervals) and, on a shared Slurm cluster, other users' load.")


if __name__ == "__main__":
    main()
