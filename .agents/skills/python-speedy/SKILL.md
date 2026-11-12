---
name: python-speedy
description: High-impact Python performance guidance for CPU-bound, I/O-bound, and memory-heavy code. Covers profiling workflow, built-ins and data structures, caching, concurrency tradeoffs, and when to ask whether the user wants compiled acceleration via Numba or Cython before adopting it.
---

# Python Performance Tuning

Use this skill to improve Python runtime performance without falling into premature micro-optimization. Start by measuring the real bottleneck, then target the hottest path with the smallest, most maintainable fix.

Your knowledge of profiler behavior, Python internals, and performance tradeoffs may be outdated. Prefer retrieval over pre-training when citing exact stdlib or version-specific behavior.

## Retrieval Sources

| Source | How to retrieve | Use for |
|--------|----------------|---------|
| Python docs | `https://docs.python.org/3/library/profile.html` and `https://docs.python.org/3/library/cProfile.html` | `cProfile`, `profile`, `timeit`, and stdlib profiling basics |
| Python 3.15 sampling profiler article | `https://realpython.com/python315-sampling-profiler/` | Sampling vs tracing profilers, wall time vs CPU time, flame graphs, live-process profiling |
| Python docs: `itertools`, `functools`, `asyncio`, `multiprocessing` | Standard library docs | Data structure and concurrency guidance |
| NumPy / Numba / Cython docs | Project docs or package docs | Compiled acceleration only after profiling and user approval |

## First: Ask the Right Question

Before recommending Numba, Cython, or any compiled-speedup path, ask:

- Is the hotspot numeric, loop-heavy, and repeated enough to justify a compiled dependency?
- Does the user want the added build complexity, toolchain requirements, and deployment risk?
- Is the workload still hot after profiling and algorithmic cleanup?
- Would a faster data structure, caching, vectorized library, or multiprocessing change solve the problem without extra complexity?

If the answer is unclear, do not jump to compiled acceleration. Default to Python-level improvements and profiling first.

## Key Guidelines

- Measure before optimizing. A fast-looking code path is not a bottleneck until it shows up in a real profile.
- Prefer the biggest win over the most clever micro-optimization.
- Use the standard library and C-backed operations before rewriting logic in Python.
- Avoid repeated string concatenation inside loops; use `join()` or accumulate in a list when appropriate.
- Prefer list comprehensions for straightforward transforms and generator expressions for large or lazy streams.
- Use `set` and `dict` for membership and lookup-heavy logic instead of repeated list scans.
- Use `functools.lru_cache` for repeated pure-function calls with stable inputs.
- Use `asyncio` for I/O-bound concurrency and `multiprocessing` for true CPU-bound parallelism.
- Remember that Python threads do not speed up pure-Python CPU-bound work because of the GIL.
- For a production-like or long-running workload, use sampling profilers to find hot stacks with low overhead.
- For a small script or exact call counts, use tracing profilers like `cProfile` / `profiling.tracing`.
- Keep benchmarks realistic. The best optimization is the one that helps the real workload, not a contrived micro-benchmark.

## Quick Reference

| Pattern | Use it when | Why |
|---------|-------------|-----|
| `sum()`, `min()`, `max()` | reducing a sequence | implemented in C and usually faster than manual loops |
| list comprehension | mapping/filtering straightforward data | lower overhead than repeated `append()` in Python |
| generator / generator expression | large or streaming data | lazy iteration and lower memory use |
| `" ".join(items)` | building strings from many parts | O(n) instead of repeated O(n²) concatenation |
| `set` / `dict` | membership and lookup-heavy logic | average O(1) lookup |
| `itertools` | filtering, grouping, combinations | efficient iterator primitives |
| `lru_cache` | repeated expensive pure calls | avoids redundant computation |
| `asyncio` | network, I/O, waiting on external systems | concurrency without threads |
| `multiprocessing` | CPU-bound parallel work | bypasses the GIL across processes |
| `Numba` / `Cython` | proven numeric hotspot with explicit user approval | compiled speedup with extra complexity and toolchain risk |

## Example Snippets

Use these as concrete patterns when the profile points to a real hotspot. Prefer the smallest fix that addresses the measured bottleneck, and keep the examples aligned with the user’s workload.

### List comprehensions, generators, and built-ins

```python
numbers = []
for i in range(1000000):
    numbers.append(i * 2)
```

```python
numbers = [i * 2 for i in range(1000000)]
```

```python
numbers = (i * 2 for i in range(1000000))
```

```python
def custom_sum(numbers):
    total = 0
    for num in numbers:
        total += num
    return total

custom_sum([1, 2, 3, 4, 5])
```

```python
sum([1, 2, 3, 4, 5])
```

### String building and lookup-heavy logic

```python
words = ["Python", "is", "fast"]
sentence = ""
for word in words:
    sentence += word + " "
```

```python
sentence = " ".join(words)
```

```python
items = [1, 2, 3, 4, 5]
if 3 in items:
    print("Found")
```

```python
items = {1, 2, 3, 4, 5}
if 3 in items:
    print("Found")
```

### Caching and iterator helpers

```python
from functools import lru_cache

@lru_cache(maxsize=1000)
def slow_function(n):
    print(f"Computing {n}...")
    return n * n

print(slow_function(10))
print(slow_function(10))
```

```python
from itertools import combinations

items = ["a", "b", "c"]
for combo in combinations(items, 2):
    print(combo)
```

### Multiprocessing for true CPU-bound parallelism

```python
from multiprocessing import Pool


def compute(_):
    return sum(i * i for i in range(10**7))


with Pool(4) as p:
    p.map(compute, range(4))
```

### Profiling and compiled acceleration examples

```bash
python -m cProfile -s time my_script.py
```

```python
from numba import jit

@jit(nopython=True)
def fast_function(n):
    return sum(i * i for i in range(n))

print(fast_function(10**7))
```

Use the `Numba` example only after a real profile shows a hot numeric loop and after explicitly confirming that the user wants the added build complexity, dependency risk, and maintenance burden.

## Profiling Workflow

1. Reproduce the slowdown with a realistic workload.
2. Classify the bottleneck: CPU, I/O, memory, or synchronization/lock contention.
3. Choose the right profiler:
   - `cProfile` / `profiling.tracing` for exact call counts and tight scripts
   - `profiling.sampling` (Python 3.15+) for low-overhead profiling of long-running or production-like code
   - `timeit` for small code snippets and micro-benchmarks
   - `py-spy`, `perf`, or flame-graph tools for live or attached processes
4. Focus on the dominant stack frames and hot functions, not the first suspicious line.
5. Fix the actual bottleneck and re-measure.
6. Validate on a realistic workload before calling the change complete.

### Python 3.15 Sampling Profiler Tips

- Use sampling when the app is long-running, production-like, or too expensive to perturb with tracing.
- Sampling captures call stacks at intervals and shows which code is consuming the most time; this is ideal for finding the hot path without a large overhead penalty.
- Use wall-clock sampling to expose waiting and blocking; use CPU-focused or GIL-aware modes to separate real compute from stalls.
- Use a flame graph or heatmap to spot concentrated hotspots rather than reading raw percentages in isolation.
- When the profile shows time in native code or external libraries, check whether `NumPy` vectorization or a compiled helper is the right next step.
- When the profile is dominated by I/O wait, locks, or thread contention, optimize scheduling, batching, or async flow rather than rewriting Python loops.
- Sampling is a great way to find the hidden bottleneck; it is not a license to ignore call counts or correctness.

### How to Read a Profile

- High time in one function with low call count: likely algorithmic or data-structure pressure.
- High time in repeated Python loops: optimize data structures, vectorize, or compile only when justified.
- High time in waiting or blocking: optimize I/O batching, async execution, or queue design.
- High time in native code: investigate whether a `numpy` or compiled path would help, but confirm before adding more dependencies.

## Optimization Patterns

### CPU-bound workloads

- Replace repeated list scans with `set`/`dict` membership and lookup.
- Use vectorized libraries such as NumPy when the operation is array-oriented.
- Cache repeated pure-function results with `lru_cache`.
- Use multiprocessing when the workload is truly parallelizable and the overhead is acceptable.
- Ask before suggesting `Numba` or `Cython`; only recommend them if profiling proves a hot numeric loop and the user wants compiled acceleration.

### I/O-bound workloads

- Batch requests and reads instead of issuing tiny round trips.
- Reuse connections and avoid redundant fetches.
- Use `asyncio` for large fan-out or wait-heavy flows.
- Measure throughput, latency, and memory per request rather than micro-optimizing a single helper.

### Memory-heavy workloads

- Stream large data instead of materializing huge intermediate lists.
- Avoid repeated copies of large objects in loops.
- Use generators or chunked processing to keep memory bounded.
- Profile memory churn and object creation if the app is falling over at the allocator level.

## Output Format

When proposing a fix, include:

1. The bottleneck type: CPU, I/O, memory, or lock contention.
2. The profiler used and what it showed.
3. The exact optimization being proposed.
4. Why the change is likely to help.
5. Before/after validation with a realistic benchmark or command.
6. If compiled acceleration is involved, explicitly note whether the user wants `Numba` or `Cython` and explain the tradeoff: speed vs build complexity and maintenance cost.

## Practical Rule of Thumb

- First: profile and classify.
- Second: fix algorithmic or data-structure inefficiencies in Python.
- Third: consider `NumPy`, `multiprocessing`, or async if the bottleneck fits.
- Fourth: only after user confirmation, consider `Numba` or `Cython` for hot numeric kernels where the payoff justifies the complexity.

This keeps performance work grounded in evidence instead of guesswork.
