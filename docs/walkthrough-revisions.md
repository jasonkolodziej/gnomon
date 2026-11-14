# Gnomon paper walkthrough: revisions after the future-work iteration

These notes update `gnomon_paper_walkthrough_16x9.mp4` (16 scenes, 8:32) so the video matches `main`
after PR #2. That PR implemented all six future-work items and answered Copilot's 13-finding review.
Every number below comes from the repository (`results/EVALUATION.md` §3b and §7,
`results/testkit/COMPARISON.md`, `results/bench-2026-11/`, `results/replay_baseline.json`) or from runs
made during the PR. Where a number is new or measured differently, its source is given.

**How to use this file:** each scene lists the **graphic** changes, then the **narration** changes as
*old → new*, timestamped against the current transcript. Appendix A has the full revised narration,
ready for TTS (Kokoro `af_heart`), and Appendix B has the timing estimate.

## At a glance

| # | Scene | Change | Why |
|---|---|---|---|
| 01 | Title | none | |
| 02 | What are we doing here? | none | |
| 03 | How was it addressed before? | none | |
| 04 | Why weren't they good enough? | none | |
| 05 | General structure | **major** | 12 crates (not 11), ~7,800 lines (not ~3,800); consensus is durable now; the agent gained isolation |
| 06 | Novelty | minor | runtime-uncertainty card mentions bias correction |
| 07 | Data flow | **major** | durable journal, cgroup and job-user launch, gang barrier, per-rank completion |
| 08 | Module details (1/2) | minor | state-machine card: durable journal and gang barrier state |
| 09 | Module details (2/2) | moderate | new card: gang matching on the graph |
| 10 | How to train it | **major** | second, bias-corrected estimator, with a different loss setup |
| 11 | Main discovery | **major** | finding 2 changed; new finding 4 (snapshot cost) |
| 12 | Explanation and testing | moderate | bias-correction ablations, A/B throughput, the Slurm side-by-side |
| 13 | History | minor (optional) | none required |
| 14 | Experiment results | **major** | re-measured on a 4-vCPU VM; new tiles |
| 15 | Future work | **major** | items delivered; new open list; new limits banner |
| 16 | Close | none | |

## Global changes (every scene)

- **Footers that say "2-core sandbox"** (scenes 14 and 15) → "4-vCPU VM". The results in this iteration
  were re-measured on a 4-vCPU cloud VM. The original OCC numbers came from the 2-core sandbox.
- **Source footers:** wherever a slide cites `results/EVALUATION.md`, add the section: §3b (predictor)
  and §7 (this iteration).
- **The progress chips** (Background · Algorithm · Insights · History · Results) are unchanged. If you
  add the optional scene 14b (below), it belongs under *Results*.
- The palette, typography and layout system stay as they are; every change below fits the existing
  card and tile components.

---

## 05 · General structure: how many modules? `[01:56]`

**Graphic**
- **Subtitle:** `11 crates · ~3,800 lines of Rust` → `12 crates · ~7,800 lines of Rust · 203 tests`. The
  count excludes tests, blank lines and comments; it was 3,692 at the time of the video.
- **New tile `top`:** "gntop · live Gantt TUI". Add it to the grid next to `cli` and `api`; the grid
  becomes 4 × 3 with one empty cell, or put `top` in the bottom row beside `sim`.
- **`consensus` tile:** remove the dashed "not wired yet" border and make it solid like the others.
  New caption: "durable openraft storage · controller journal".
- **`agent` tile caption:** "runs jobs on nodes" → "cgroup v2 · containers · runs as job user".
- **`fsm` tile caption:** "state machine · clock · journal" → "state machine · clock · journal · gang
  barrier".
- **`sched` tile caption:** "broker · workers · applier · pass" → "broker · workers · applier · pass ·
  predictors".
- **`cli` tile caption:** "Slurm-style commands" → "Slurm-style commands · `-N` gangs".

**Narration**
- `01:59` *"Gnomon is eleven Rust modules, about thirty-eight hundred lines."* →
  **"Gnomon is twelve Rust modules, about seventy-eight hundred lines, with two hundred tests."**
- `02:09` *"The state machine owns all cluster state."* → unchanged.
- `02:20` *"Consensus wraps openraft, though the controller doesn't use it yet."* →
  **"Consensus keeps Raft's log, votes and snapshots durably on disk, and the controller journals every
  event, so it survives a restart."**
  - Accuracy note: do **not** say the controller replicates through Raft. It persists through its own
    write-ahead journal; Raft network replication is still open.
- `02:24` *"The API, controller, node agent, and Slurm-style command line make it usable."* →
  **"The API, controller, node agent, and Slurm-style command line make it usable, and gntop draws the
  plan live."**

## 06 · Novelty `[02:33]`

**Graphic**
- **"Runtime uncertainty inside planners" card body:** "Spans planned at a learned quantile with a risk
  budget." → "Spans planned at a learned, bias-corrected quantile; wide jobs' reservations are
  protected."
- The other four cards are unchanged. Multi-node gangs, durability and the agent features are
  engineering on top of the novel combination, not new claims, so they get no novelty card.

**Narration**
- `02:57` *"Runtime uncertainty lives inside the planners."* →
  **"Runtime uncertainty lives inside the planners, through a predictor that corrects its own bias."**

## 07 · What is the general data flow? `[03:12]`

**Graphic** (the current flow is gnbatch → API → event log → state machine → broker → worker →
plan applier → agent → job runs → release for backfill)
- **New small box hanging off "event log":** "journal → disk", subtitle `--state-dir`, with a dashed
  connector to show it's optional.
- **"agent · on the node" box subtitle:** → "cgroup · runs as job user".
- **New branch for gangs** between "plan applier" and "agent": a fork into 2–3 stacked agent boxes,
  joining at a small gate labelled **"barrier · GangReady"**, then into "job runs". Caption under the
  gate: "ranks start together (0.14 s apart, measured)".
- **"job runs · report completion" box:** add "per-rank reports → FSM aggregates".
- The bottom caption "freed capacity → next scheduling pass" is unchanged.

**Narration**
- `03:20` *"The state machine applies it, and the broker queues an evaluation."* →
  **"The state machine applies it, after writing it to a durable journal if one is configured, and the
  broker queues an evaluation."**
- `03:33` *"The agent on that node runs the job and reports completion."* →
  **"The agent on that node runs the job inside its own cgroup or container, as the user who submitted
  it. For a multi-node job, every rank waits at a barrier until all ranks have been fetched, so they
  start together, and the job completes when every rank has reported."**

## 08 · Details of each module (1/2) `[03:41]`

**Graphic**
- **`fsm · state machine` card footnote:** "Injected clock + journal: state is a pure function of its
  log." → add a second line: "With `--state-dir`, every event is fsync'd to a checksummed journal before
  it is applied; gang barrier state is replicated too."
- The planner, broker and applier cards are unchanged.

**Narration**
- `03:51` *"The state machine takes an injected clock and journals every event, so state is a pure
  function of its log."* → keep it, then add:
  **"With a state directory, that log is written to disk first, so the controller recovers exactly
  after a restart."**

## 09 · Details of each module (2/2) `[04:10]`

**Graphic**
- **New fifth card `graph · match_gang`:** "Earliest common start for N nodes · checked against a
  brute-force oracle (500 cases)". Re-flow the grid to 3 + 2, or replace the right-hand column of the
  lower row with this card and move `sim · Simulator` up.
- **`sched · backfill_pass`, step 1:** "extend jobs that outran their prediction" is still true; the new
  predictor re-plans an overrunning job to its walltime instead of doubling its span. No visual change
  needed.

**Narration**
- After `04:32` (*"…drives the real state machine with real workload traces."*), add:
  **"And multi-node gang jobs are matched on the graph too: the earliest time at which enough nodes are
  free together, checked against a brute-force oracle."**

## 10 · How to train it: loss functions `[04:38]`

This is the biggest content change. The video describes only the original per-user quantile estimator.
That estimator still exists (`QuantileEstimator`), but the trained component is now
`BiasCorrectedEstimator` (`crates/sched/src/bias.rs`, §3b).

**Graphic**
- **Keep** the "No neural network — one learned estimator" pill, reworded: "No neural network —
  learned runtime estimators".
- **Left panel** (the per-user bar chart, "theta = 0.90 quantile → planned span"): relabel it **"v1 ·
  per-user quantile"** and shrink it to about 40% width.
- **Right panel** keeps the pinball-loss formula; it is still the loss. Under it, add the new pipeline
  as three stacked chips:
  1. **"base: weighted mean of ln(run / walltime)"**, by user → walltime bucket → cores bucket, shrunk
     toward the coarser group;
  2. **"bias tracker: online pinball-loss quantile of the log error, τ = 0.7"**, with the note
     "under-prediction costs τ/(1−τ) = 2.3× more";
  3. **"cautious span τ = 0.9 · protection when a wide reservation slips"**.
- **Bottom chips:** "risk budget = 1 − theta" is unchanged. "online · per-user · warm-started" →
  "online · per-user + feature buckets · warm-started". "overrun → span extended, never cut short" →
  "overrun → re-planned to walltime, never cut short".
- **Footer:** `QuantileEstimator: window 32 · min 4 samples · floor 60 s` → add
  `BiasCorrectedEstimator: τ 0.7 · cautious 0.9 · reservation protection`.

**Narration** (replace `04:42`–`05:14`)
- Old: *"The one learned component is the runtime estimator. For each user, it keeps the ratio of actual
  runtime to requested walltime, over their last thirty-two jobs. It plans each new job at the theta
  quantile of that history, which is exactly the value that minimizes the pinball loss. One minus theta
  is the risk budget: the chance a job outlives its plan. It learns online from completion events,
  falls back to a global history for new users, and is warm-started from two thousand past jobs. If a
  job overruns, its span is extended, never cut short."*
- New:
  **"The learned component is the runtime estimator. The first version kept, for each user, the ratio of
  actual runtime to requested walltime, and planned each job at the theta quantile of that history:
  exactly the value that minimizes the pinball loss. One minus theta is the risk budget, the chance a
  job outlives its plan. The new version corrects its own bias. It predicts from the user, the requested
  walltime and the job's width, then tracks its own errors with that same pinball loss, so an
  under-prediction costs more than twice an over-prediction, and the overrun rate settles where we set
  it. It records a more cautious span when that still fits, and if a wide job's reservation starts
  slipping, it stops squeezing jobs in front of it. It learns online, is warm-started from two thousand
  past jobs, and a job that overruns is never cut short."**

## 11 · What is the main discovery? `[05:19]`

**Graphic**
- **Card 1** (commit check) is unchanged.
- **Card 2** title: "Prediction has headroom, our predictor doesn't capture it" → **"Correcting the bias
  captures most of the headroom"**. Body: "Oracle: −16% mean wait, −29% slowdown. Bias-corrected:
  −10% mean wait in 9/9 windows, −20% slowdown. Wide jobs still wait longer in 4 of 9 windows, so it
  ships opt-in."
- **Card 3** (time-aware capacity) is unchanged; E2 reproduces exactly on the new code.
- **New card 4: "Snapshot cost is the throughput budget".** Body: "Gang support first cut controller
  throughput to 0.55×: a few extra allocations per job, cloned into every snapshot. Sharing them through
  reference counts was little better (cache-line contention). Fixed: 0.92–0.96× of main; now guarded by
  an A/B check on every PR." Re-flow to 4 cards in one row, or 2 × 2.

**Narration**
- `05:28`–`05:41` → replace:
  **"Second: runtime prediction has real headroom. With perfect knowledge, waits drop sixteen percent,
  and slowdown twenty-nine percent. Our first predictor captured little of it, but correcting its bias
  captures most: ten percent shorter waits in all nine windows, and twenty percent less slowdown. Wide
  jobs still wait longer in four of nine windows, so it stays opt-in."**
- After `05:41` (*"Third: …derating the whole day."*), add:
  **"And fourth: in this design, every allocation cloned into a snapshot costs throughput. Adding gang jobs
  first cut it almost in half, until the per-job data was made allocation-free."**

## 12 · Our explanation, and how we tested it `[05:48]`

**Graphic**
- The hypothesis diagram ("predicted too short → overrun → wide job's reservation slips") is unchanged;
  the new results support it.
- **"Prediction" box:** add a line: "+ bias-corrected variants and ablations (τ 0.6–0.8, cautious
  0.85–0.95)".
- **New box "Verification"** under "Commit fix": "every PR: replay gate on 3 traces + throughput A/B vs
  main · weekly: same test kit on Gnomon and a single-node Slurm".

**Narration**
- After `06:19` (*"…compared walltime planning, four risk budgets, and a perfect-knowledge oracle…"*):
  insert before "and measured wide-job waits separately": **"…, a perfect-knowledge oracle, and the
  bias-corrected predictor with its ablations, …"**.
- After `06:25` (*"For the commit fix, we ran the real controller loop with live heartbeats."*), add:
  **"And every change is now measured before it merges: a replay gate on real traces, a throughput
  check against main, and the same test suite run against Slurm."**

## 13 · Notable works, state of the art, trade-offs `[06:30]` (optional)

No change is required. Optionally:
- **"Trade-offs" box:** add "isolation vs launch latency (cgroups, containers, privilege drop)".
- **"Variants" box:** "batch vs services" could add "single-node vs gang (multi-node) jobs".

## 14 · Experiment results `[07:19]`

**Graphic:** the tile grid becomes 4 × 3. Change "2-core sandbox" to "4-vCPU VM" in the footer. Tiles
(new values in **bold**):

| Tile | Old | New | Caption |
|---|---|---|---|
| 1 | 92–100% | **97–100%** | fewer plan conflicts · controller path |
| 2 | 1.99× | **1.45×** | throughput, best case · 2 workers, 1,000 heartbeats/s |
| 3 | 5,000 / 5,000 | 5,000 / 5,000 | start times on replay · exact |
| 4 | 0 | 0 | power-cap violations · time-aware planner |
| 5 | −85% / −67% | −85% / −67% | median wait vs derating · power cap / carbon |
| 6 | ~1.5× | **~1.4×** | parallel vs serial · 4–8k job bursts |
| 7 | 7 / 7 | **7 / 7 · 7 / 7** | end-to-end tests · Gnomon and Slurm |
| 8 (new) | — | **−10%** | mean wait, bias-corrected · 9/9 windows |
| 9 (new) | — | **120 vs 43** | jobs/s submitted · Gnomon vs Slurm, same host |
| 10 (new) | — | **0.14 s** | gang rank start spread · release barrier |
| 11 (new) | — | **0.95×** | controller throughput vs before gangs · A/B |
| 12 (new) | — | **87%** | line coverage · 203 tests |

*Tile 2 provenance:* 1.99× was measured in the 2-core sandbox (§1). The 4-vCPU re-run peaks at 1.45×
(2 workers, 1,000 heartbeats/s; `results/bench-2026-11/occ_conflicts.csv`). Show one consistent run.
If you'd rather keep 1.99×, label it "2-core sandbox" and drop the footer change.

**Narration** (replace `07:21`–`07:51`)
- **"On the controller path: ninety-seven to a hundred percent fewer plan conflicts, and up to one and a
  half times the throughput. Replay is exact: five thousand of five thousand start times reproduced.
  Time-aware capacity gives zero cap violations, with eighty-five percent shorter median waits than
  derating all day, and sixty-seven percent under a carbon window. Parallel workers win by about one
  point four times on bursts of four to eight thousand jobs. The bias-corrected predictor cuts mean wait
  by ten percent. And all seven end-to-end tests pass, on Gnomon and on Slurm, where Gnomon accepts
  submissions nearly three times as fast."**

### 14b · Verification, side by side with Slurm (optional new scene, about 25 s)

One new scene would show the "same test suite next to Slurm" item, which the results grid can only
summarize.
- **Layout:** two columns, Gnomon | Slurm 23.11, with the seven test rows from
  `results/testkit/COMPARISON.md`, all PASS.
- **Highlight rows:** t03 backfill ("short job backfilled 237 s vs 234 s ahead of the wide one") and t05
  burst ("120 vs 43 jobs/s submit, drained in 27 vs 37 s").
- **Caveat chip:** "single-node Slurm on the same VM · behaviour and latency, not scale".
- **Narration:** **"We ran the same seven tests against Slurm, on the same machine. Both pass all seven,
  and backfill identically. On a burst of fifty jobs, Gnomon accepts submissions nearly three times as
  fast and drains the queue ten seconds sooner. It's a one-node Slurm, so this compares behaviour, not
  scale."**

## 15 · Future work `[07:55]`

**Graphic**
- **Retitle** "Future work" → **"Delivered, and what's next"** (or keep the title and add a
  "Delivered" column).
- **Tick each checkbox** and add a status tag on the right:

| Item | Tag |
|---|---|
| Bias-correcting runtime predictor that protects wide jobs | `built · opt-in` (wide jobs still worse in 4/9) |
| Persistent Raft · cgroup confinement | `built` |
| Multi-node gang jobs | `built · release barrier` |
| Rootless Docker containers · live Gantt TUI (gntop) | `built` |
| Same test suite, side by side on a real Slurm cluster | `single-node Slurm · weekly CI` (multi-node still open) |
| Every change measured with the replay harness | `CI gate on every PR` |

- **New "Next" list** (unticked, smaller):
  - Raft network layer and multi-node controller replication
  - Cancel a gang's peers when one rank fails (agent kill channel)
  - Predictor that never slows wide jobs
  - Side by side on a real multi-node Slurm cluster
  - Validate cgroup v2 limits and containers on real hosts
- **Limits banner:** "Limits: simulation and a 2-core sandbox — cluster-scale validation comes next" →
  **"Limits: simulation, a single-node Slurm and a 4-vCPU VM — cluster-scale validation comes next"**.

**Narration** (replace `07:57`–`08:18`)
- **"All six items from our last plan are built. A predictor that corrects its bias, shipped opt-in
  until it never slows wide jobs. Durable Raft storage, and cgroup confinement, with every job run as
  the user who submitted it. Multi-node gang jobs, whose ranks start together. Rootless containers, and
  a live Gantt view in the terminal. The same test suite, side by side with Slurm. And every change
  measured with the replay harness before it merges. Next: Raft replication across controllers, and a
  real multi-node cluster, because these results still come from simulation and a single machine."**

---

## Appendix A · Revised narration (full, for TTS)

Unchanged lines are reproduced so the file can be fed to TTS as-is. Changed lines are marked ✎.

### 01 Title
Gnomon: planning HPC schedules on the time axis.
In this walkthrough: the problem, the design and its modules, what is new, how its one learned component is trained, what we found, where it sits in the history of schedulers, and the results so far.

### 02 What are we doing here?
What are we doing here?
A cluster scheduler decides which job runs on which machine, and when.
On shared HPC clusters, that decision sets everyone's queue wait.
Today it's mostly made by Slurm, which runs on roughly half of the Top five hundred systems.
Gnomon is a research scheduler, written in Rust, that rethinks how the decision is made.

### 03 How was it addressed before?
How was this addressed before?
Four families of designs.
Monolithic schedulers, like Slurm and Google's Borg, make every decision in one central loop.
Two-level designs, like Mesos, hand resource offers to frameworks.
Shared-state designs, like Omega and Nomad, let many schedulers plan in parallel and commit optimistically.
And Flux, from Lawrence Livermore, models resources as a graph with a timeline on every vertex, and nests schedulers inside allocations.
Most recently, AMD's Spur rebuilt the Slurm experience in Rust.

### 04 Why weren't they good enough?
Why weren't they good enough?
Slurm's scheduling loop holds global locks, so while it plans, the controller can't answer queries.
Standard setups manage around a hundred jobs per second, and about five hundred only with heavy tuning.
Its node-record model treats GPUs and topology as add-ons.
Flux has the graph and the hierarchy, but one matching loop per instance.
Omega and Nomad parallelize placement, but have no HPC resource graph, and no backfilling along a time axis.
Spur modernizes the plumbing, but keeps a single central backfill loop.
No system combines all of these.

### 05 General structure
The general structure.
✎ Gnomon is twelve Rust modules, about seventy-eight hundred lines, with two hundred tests.
Core holds the shared types and events.
Graph holds the resources, with a planner on every vertex.
The state machine owns all cluster state.
Sched holds the broker, the parallel workers, and the plan applier.
Policy holds fair-share rules and sandboxed Web Assembly plugins.
✎ Consensus keeps Raft's log, votes and snapshots durably on disk, and the controller journals every event, so it survives a restart.
✎ The API, controller, node agent, and Slurm-style command line make it usable, and gntop draws the plan live.
And sim replays history through all of it.

### 06 Novelty
What is new?
Every ingredient has a precedent, but the combination doesn't exist: a resource graph with time-axis planners, scheduled by optimistic parallel workers.
On top of that come four mechanisms.
Backfill reservations are computed in parallel, and checked per node at commit time.
The scheduler is its own simulator: it can replay its history exactly.
✎ Runtime uncertainty lives inside the planners, through a predictor that corrects its own bias.
And capacity can change over time, for power caps and carbon windows.
Delegating part of the cluster to a child scheduler is checked by Rust's type system, not by convention.

### 07 Data flow
Here's the data flow.
A job submitted with gnbatch reaches the API and becomes an event in the log.
✎ The state machine applies it, after writing it to a durable journal if one is configured, and the broker queues an evaluation.
A worker takes an immutable snapshot, and the policy plans a start time on the graph.
The plan applier commits it only if the nodes it uses still fit.
✎ The agent on that node runs the job inside its own cgroup or container, as the user who submitted it.
✎ For a multi-node job, every rank waits at a barrier until all ranks have been fetched, so they start together, and the job completes when every rank has reported.
And the rest of its reserved time is released for backfill.

### 08 Module details (1/2)
Inside the modules.
The planner keeps reserved spans on each node, and finds the earliest start in a single sweep, checked against a brute-force oracle.
The state machine takes an injected clock and journals every event, so state is a pure function of its log.
✎ With a state directory, that log is written to disk first, so the controller recovers exactly after a restart.
The broker drops duplicate evaluations.
And the applier tracks a version for every node: a plan conflicts only if a node it used has changed, and the plan no longer fits.

### 09 Module details (2/2)
Each scheduling pass extends jobs that outran their prediction, re-plans the reservations, then places jobs in priority order, with up to thirty-two reservations per pass.
Capacity profiles turn a power cap into capacity that varies through the day.
A shadow policy runs a candidate next to the live one, and records where they disagree.
And the simulator drives the real state machine with real workload traces.
✎ And multi-node gang jobs are matched on the graph too: the earliest time at which enough nodes are free together, checked against a brute-force oracle.

### 10 How to train it
How is it trained?
There's no neural network.
✎ The learned component is the runtime estimator.
✎ The first version kept, for each user, the ratio of actual runtime to requested walltime, and planned each job at the theta quantile of that history: exactly the value that minimizes the pinball loss.
✎ One minus theta is the risk budget, the chance a job outlives its plan.
✎ The new version corrects its own bias.
✎ It predicts from the user, the requested walltime and the job's width, then tracks its own errors with that same pinball loss, so an under-prediction costs more than twice an over-prediction, and the overrun rate settles where we set it.
✎ It records a more cautious span when that still fits, and if a wide job's reservation starts slipping, it stops squeezing jobs in front of it.
✎ It learns online, is warm-started from two thousand past jobs, and a job that overruns is never cut short.

### 11 Main discovery
What did we find?
First: the hidden bottleneck was the commit check.
Any event, even a heartbeat, invalidated every plan in flight.
Second: runtime prediction has real headroom.
With perfect knowledge, waits drop sixteen percent, and slowdown twenty-nine percent.
✎ Our first predictor captured little of it, but correcting its bias captures most: ten percent shorter waits in all nine windows, and twenty percent less slowdown.
✎ Wide jobs still wait longer in four of nine windows, so it stays opt-in.
Third: a time-aware planner can obey a power cap at a fraction of the cost of derating the whole day.
✎ And fourth: in this design, every allocation cloned into a snapshot costs throughput. Adding gang jobs first cut it almost in half, until the per-job data was made allocation-free.

### 12 Explanation and testing
Our explanation: when spans are predicted too short, reservations for wide jobs slip, because the jobs ahead of them overrun.
That matches published work showing that estimation bias, not accuracy, drives backfilling outcomes.
To test it, we replayed three production traces from the Parallel Workloads Archive: nine windows of five thousand jobs each.
✎ We compared walltime planning, four risk budgets, a perfect-knowledge oracle, and the bias-corrected predictor with its ablations, and measured wide-job waits separately.
For power caps, we compared ignoring the cap, derating all day, and time-aware planning.
For the commit fix, we ran the real controller loop with live heartbeats.
✎ And every change is now measured before it merges: a replay gate on real traces, a throughput check against main, and the same test suite run against Slurm.

### 13 History
Where does this sit in the history?
On the central path: Slurm, Borg, and Firmament's flow-based placement.
On the parallel path: Omega, then Nomad.
On the hierarchical path: Flux.
On the distributed path: Sparrow and Hawk, for very short tasks.
And in Rust: Spur.
The state of the art splits two ways: Flux reports forty-five to sixty times higher throughput through nesting, while Slurm remains the production default.
The problem has variants: placement only, or reservations over time; batch jobs, or long-running services; uniform nodes, or heterogeneous GPUs.
And the trade-offs: optimal placement versus decision latency, global visibility versus isolation, and how far backfill can trust walltime estimates.

### 14 Results
The results.
✎ On the controller path: ninety-seven to a hundred percent fewer plan conflicts, and up to one and a half times the throughput.
Replay is exact: five thousand of five thousand start times reproduced.
Time-aware capacity gives zero cap violations, with eighty-five percent shorter median waits than derating all day, and sixty-seven percent under a carbon window.
✎ Parallel workers win by about one point four times on bursts of four to eight thousand jobs.
✎ The bias-corrected predictor cuts mean wait by ten percent.
✎ And all seven end-to-end tests pass, on Gnomon and on Slurm, where Gnomon accepts submissions nearly three times as fast.

### 14b Verification, side by side with Slurm (optional)
✎ We ran the same seven tests against Slurm, on the same machine.
✎ Both pass all seven, and backfill identically.
✎ On a burst of fifty jobs, Gnomon accepts submissions nearly three times as fast and drains the queue ten seconds sooner.
✎ It's a one-node Slurm, so this compares behaviour, not scale.

### 15 Delivered, and what's next
✎ All six items from our last plan are built.
✎ A predictor that corrects its bias, shipped opt-in until it never slows wide jobs.
✎ Durable Raft storage, and cgroup confinement, with every job run as the user who submitted it.
✎ Multi-node gang jobs, whose ranks start together.
✎ Rootless containers, and a live Gantt view in the terminal.
✎ The same test suite, side by side with Slurm.
✎ And every change measured with the replay harness before it merges.
✎ Next: Raft replication across controllers, and a real multi-node cluster, because these results still come from simulation and a single machine.

### 16 Close
Gnomon.
Scheduling that knows what time it is.

## Appendix B · Timing estimate

Each scene keeps its current narration pace, measured from the existing transcript (2.38 words/s
on average, pauses included). Unchanged scenes keep their exact length.

| Scene | Now | Revised (est.) | Δ |
|---|---|---|---|
| 01 Title | 0:17 | 0:17 | +0 s |
| 02 What are we doing here? | 0:24 | 0:24 | +0 s |
| 03 How was it addressed before? | 0:36 | 0:36 | +0 s |
| 04 Why weren't they good enough? | 0:39 | 0:39 | +0 s |
| 05 General structure | 0:37 | 0:46 | +9 s |
| 06 Novelty | 0:39 | 0:42 | +3 s |
| 07 Data flow | 0:29 | 0:47 | +18 s |
| 08 Module details (1/2) | 0:29 | 0:36 | +7 s |
| 09 Module details (2/2) | 0:28 | 0:39 | +11 s |
| 10 How to train it | 0:41 | 1:01 | +20 s |
| 11 Main discovery | 0:29 | 0:51 | +22 s |
| 12 Explanation and testing | 0:42 | 0:57 | +15 s |
| 13 History | 0:49 | 0:49 | +0 s |
| 14 Results | 0:36 | 0:44 | +8 s |
| 14b Verification, side by side with Slurm (optional) | — | 0:21 | +21 s |
| 15 Delivered, and what's next | 0:30 | 0:42 | +12 s |
| 16 Close | 0:07 | 0:07 | +0 s |
| **Total** | **8:32** | **10:58** | **+146 s** |

**About 10:37 without the optional scene 14b, 10:58 with it**, against 8:32 today. If the video must stay
near 9 minutes:
- **Scene 10:** keep only the new estimator's description. Drop the sentence about the first version
  (−9 s).
- **Scene 11:** finding 4 can move to scene 12 as one clause, "a throughput check against main, which
  caught a 45% regression" (−10 s).
- **Scene 07:** fold the barrier into one clause, "…and a multi-node job's ranks start together at a
  barrier" (−7 s).
- **Skip scene 14b.** The results tiles already carry its headline numbers (−21 s).

Together these save about 47 s, bringing it to roughly 10:10; going lower means trimming unchanged scenes.
