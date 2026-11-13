# Gnomon feature evaluation

- SDSC-SP2: 53999 jobs after filtering, 128 processors
- CTC-SP2: 77199 jobs after filtering, 338 processors
- HPC2N-2002: 202871 jobs after filtering, 240 processors
- windows: 5000 consecutive jobs at offsets [10000, 20000, 30000]; learned models warm-started on the preceding 2000 jobs

## E0 — exact replay (SDSC-SP2)

| check | result |
|---|---|
| journal events | 132421 (12790 KiB JSONL) |
| JSONL round-trip lossless | true |
| replayed state identical | true |
| recovered history, same policy → identical start times (5000 jobs) | true |
| simulate 5000 jobs / rebuild from journal | 1633 ms / 513 ms |

## E3 — shadow mode (SDSC-SP2, candidate q0.90 alongside live walltime policy)

| measure | value |
|---|---|
| decisions compared | 211806 |
| agreement | 93.6% |
| candidate would start a job the live policy deferred/reserved | 6709 |
| live schedule unchanged by shadowing | true |

