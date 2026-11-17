# Gnomon architecture

This document shows how Gnomon's processes and crates fit together, how a job moves through the
system, and how replication and node-side confinement work. The research and design rationale behind
it is in [SLURM_Scheduler_Research_and_Rust_Design.md](SLURM_Scheduler_Research_and_Rust_Design.md);
installation and per-binary usage are in [INSTALL.md](INSTALL.md).

The core idea: **scheduling is managing the time axis.** Every schedulable vertex of the resource
graph carries a planner of running spans and future reservations, the scheduler plans against
immutable snapshots without holding locks, and a single applier commits plans with per-vertex
optimistic concurrency.

## Deployment

```mermaid
flowchart LR
    subgraph users["Login nodes / workstations"]
        cli["gnbatch · gnqueue · gncancel · gninfo"]
        top["gntop"]
    end

    subgraph control["Controller host(s)"]
        ctl1["gnomon-ctl<br/>(leader)"]
        ctl2["gnomon-ctl<br/>(follower)"]
        ctl3["gnomon-ctl<br/>(follower)"]
        state[("state dir<br/>journal / Raft log")]
    end

    subgraph nodes["Compute nodes"]
        a1["gnomon-agent<br/>node1"]
        a2["gnomon-agent<br/>node2"]
        aN["gnomon-agent<br/>nodeN"]
        jobs["jobs: cgroup v2 or<br/>Docker / Podman"]
    end

    cli -- "gRPC :7631<br/>SubmitJob · CancelJob · ListJobs · ListNodes" --> ctl1
    top -- "gRPC :7631<br/>ListJobs · ListNodes (poll)" --> ctl2
    a1 & a2 & aN -- "gRPC :7631<br/>RegisterNode · Heartbeat · FetchWork<br/>GangReady · ReportCompletion" --> ctl1
    ctl1 <-- "Raft :7700" --> ctl2
    ctl1 <-- "Raft :7700" --> ctl3
    ctl1 --- state
    a1 & a2 & aN --> jobs
```

- One controller is enough; with `--raft-id`/`--raft-peers`, three or five form a Raft group. Any
  replica answers; followers forward writes to the leader, and only the leader schedules.
- Agents take a list of controllers in `GNOMON_ADDR` and fail over on any error; the `gn*` tools try
  the list only until one connects; `gntop` talks to a single controller.
- All traffic is **pull-based**: agents poll `FetchWork` every heartbeat interval; the controller
  never connects to nodes.
- The API is defined in [`proto/gnomon.proto`](../proto/gnomon.proto) (service `Scheduler`).

## Crates

```mermaid
flowchart TD
    core["gnomon-core<br/><i>jobs, resources, events</i>"]
    rgraph["gnomon-graph<br/><i>resource graph, per-vertex planners</i>"]
    fsm["gnomon-fsm<br/><i>state machine, snapshots, journal</i>"]
    sched["gnomon-sched<br/><i>broker, workers, applier, backfill</i>"]
    policy["gnomon-policy<br/><i>policy-as-data, WASM plugins</i>"]
    consensus["gnomon-consensus<br/><i>openraft store, TCP transport, replica</i>"]
    api["gnomon-api<br/><i>tonic gRPC service + client stubs</i>"]
    ctl["gnomon-ctl<br/><b>bin</b> gnomon-ctl"]
    agent["gnomon-agent<br/><b>bin</b> gnomon-agent"]
    cli["gnomon-cli<br/><b>bins</b> gnbatch gnqueue gncancel gninfo"]
    top["gnomon-top<br/><b>bin</b> gntop"]
    sim["gnomon-sim<br/><i>replay, counterfactuals, SWF traces</i>"]

    rgraph --> core
    fsm --> core & rgraph
    sched --> core & fsm & rgraph
    policy --> core & rgraph & fsm & sched
    consensus --> core & fsm
    api --> core & fsm & sched
    ctl --> core & fsm & sched & api & consensus & policy
    agent --> core & api
    cli --> api
    top --> api
    sim --> core & rgraph & fsm & sched & policy

    classDef bin fill:#e8f1ff,stroke:#3b6fd4
    class ctl,agent,cli,top bin
```

Arrows point from a crate to what it depends on. The shipped binaries (blue) are thin:
`gnomon-agent`, `gnomon-cli` and `gnomon-top` only share `gnomon-api`'s generated client with the
controller, so node and user tools never link the scheduler. `gnomon-sim` runs the production
`backfill_pass` against recorded or synthetic workloads, which is how the replay gate in CI checks
schedule quality. `gnomon-policy` (declarative policy and sandboxed WASM plugins for admission,
priority, runtime estimation and node scoring) builds on the scheduler: its estimator and node-score
plugins plug into `gnomon-sched`'s `RuntimeEstimator` and `Policy`. `gnomon-ctl` links it and hands
the submit handler admission and priority through `gnomon-api`'s `SubmitPolicy` trait, so the API
client crates never link wasmtime.

## Inside the controller

```mermaid
flowchart LR
    grpc["gRPC API<br/>(gnomon-api SchedulerSvc)"]
    broker["EvalBroker<br/>coalescing priority queue"]
    worker["scheduler worker<br/>Policy::schedule(snapshot, job)"]
    applier["PlanApplier<br/>per-vertex OCC"]
    backfill["backfill_pass<br/>every 1 s"]
    fsm["Fsm<br/>events → state"]
    snap(["immutable Snapshot"])
    log[("journal / Raft log")]

    grpc -- "writes: JobSubmitted, NodeRegistered,<br/>Heartbeat, Completion, Cancel" --> fsm
    grpc -- "enqueue eval" --> broker
    grpc -. "lock-free reads" .-> snap
    broker --> worker
    worker -. reads .-> snap
    worker -- Plan --> applier
    backfill -. reads .-> snap
    backfill -- Plan --> applier
    applier -- "commit (or re-enqueue on conflict)" --> fsm
    fsm -- "apply each event" --> log
    fsm -- publishes --> snap
```

- **State machine (`gnomon-fsm`).** All cluster state is a deterministic function of an ordered
  event log and an injected clock. Readers (API, workers, `gntop`) get immutable snapshots and never
  wait on a writer: the structural fix for `slurmctld`'s global lock.
- **Planning (`gnomon-sched`).** Each submitted job becomes an *evaluation*; re-enqueueing a job
  whose snapshot has not changed is a no-op. A worker computes a plan as a pure function of the
  snapshot. The applier is the single commit point and rejects a plan only if a vertex it uses has
  changed *and* the plan no longer fits there.
- **Backfill.** Once a second the leader re-plans reservations (so reserved jobs start on time or
  earlier) and extends spans of jobs that overran their predicted runtime.
- **Durability.** With `--state-dir` every applied event goes to an fsync'd journal before it takes
  effect; a restart replays it and re-queues pending jobs.

## Job lifecycle

```mermaid
sequenceDiagram
    autonumber
    actor U as user
    participant B as gnbatch
    participant C as gnomon-ctl
    participant A1 as agent node1 (rank 0)
    participant A2 as agent node2 (rank 1)

    U->>B: gnbatch -N 2 -c 8 job.sh
    B->>C: SubmitJob
    C-->>B: job_id (Pending)
    Note over C: eval → plan on snapshot → apply:<br/>both nodes reserved at a common start<br/>(Running now, or Reserved for later)
    A1->>C: FetchWork
    C-->>A1: assignment rank 0
    A2->>C: FetchWork (up to one poll later)
    C-->>A2: assignment rank 1
    loop gang barrier, every 200 ms
        A1->>C: GangReady?
        A2->>C: GangReady?
    end
    C-->>A1: all ranks fetched
    C-->>A2: all ranks fetched
    par rank 0
        A1->>A1: launch in cgroup / container
    and rank 1
        A2->>A2: launch in cgroup / container
    end
    A1->>C: ReportCompletion(exit 0)
    A2->>C: ReportCompletion(exit 0)
    Note over C: all ranks reported → Completed,<br/>rest of the reserved window released
```

If a rank exits non-zero (or the job is cancelled while running), the controller puts every
unreported rank on a per-node **kill list** that is returned with the next `FetchWork`. Agents stop
those ranks (exit 143), release ranks still waiting at the barrier, and report ranks they never
received, so a gang always finishes, ending `Failed` or `Cancelled`.

### Lost nodes

The leader also watches heartbeats. A node silent for longer than `GNOMON_NODE_TIMEOUT_SECS` is
declared lost through a replicated `NodeLost` event, so every replica and every replay agree on it:

```mermaid
flowchart LR
    hb["Heartbeat / RegisterNode<br/>(last_heartbeat)"]
    check{"leader, every 1 s:<br/>silent > timeout?<br/>(counted from when it became leader)"}
    stale["dropped: node<br/>heartbeated meanwhile"]
    lost["NodeLost event"]
    down["node down:<br/>out of placement"]
    fail["running ranks end<br/>(exit -2): job Failed,<br/>gang peers on kill list"]
    requeue["unfetched work and<br/>reservations → Pending"]
    remember["lost ranks kept on the<br/>node's kill list"]
    back["agent heartbeats again:<br/>node up, stops lost ranks;<br/>placeable once they report"]

    hb --> check
    check -- yes --> lost
    lost -. "last_heartbeat changed" .-> stale
    lost --> down & fail & requeue & remember
    remember -. partition heals .-> back
```

### Job states

```mermaid
stateDiagram-v2
    [*] --> Pending: SubmitJob
    Pending --> Running: planned to start now
    Pending --> Reserved: backfilled, start in the future
    Reserved --> Running: start time reached (or earlier after re-planning)
    Reserved --> Pending: reservation dropped on re-plan, or its node lost
    Pending --> Cancelled: gncancel
    Reserved --> Cancelled: gncancel
    Running --> Completed: every rank exit 0
    Running --> Failed: a rank non-zero, walltime, launch error, or node lost
    Running --> Pending: node lost before its agent fetched the work
    Running --> Cancelled: gncancel (kill channel)
    Completed --> [*]
    Failed --> [*]
    Cancelled --> [*]
```

## Replication

```mermaid
flowchart LR
    client["client / agent"]
    subgraph follower["gnomon-ctl follower"]
        fapi["gRPC API"]
        ffsm["Fsm (applies committed entries)"]
    end
    subgraph leader["gnomon-ctl leader"]
        lapi["gRPC API"]
        sched2["broker · workers · backfill<br/>(leader only)"]
        prop["Proposer hook"]
        raft["openraft"]
        lfsm["Fsm"]
    end
    disk[("DiskStore<br/>fsync'd log, vote, snapshot")]

    client -- write --> fapi
    fapi -- forward --> lapi
    client -- read --> fapi
    fapi -. "own caught-up state" .-> ffsm
    lapi --> prop
    sched2 --> prop
    prop -- client_write --> raft
    raft -- replicate --> ffsm
    raft -- commit --> lfsm
    raft --- disk
```

Every write, whether from the API, the plan applier or the backfill pass, goes through the
`Proposer` hook into the Raft log, and every replica applies the same committed events, including
the plan's read set, so each one revalidates identically. Without a quorum, writes fail with
`UNAVAILABLE` instead of being accepted and lost. A newly elected leader re-plans everything pending
on its first backfill pass. Membership is static (`--raft-peers`).

## On the node

```mermaid
flowchart TD
    fetch["FetchWork reply:<br/>assignments + kill list"]
    barrier{"gang job?"}
    wait["wait at barrier<br/>(GangReady, 200 ms)"]
    img{"container image?"}
    cg{"writable cgroup2<br/>with cpu + memory?"}
    docker["docker / podman run<br/>--cpus --memory --user …"]
    confined["sh -c in &lt;root&gt;/gnomon/job_&lt;id&gt;_r&lt;rank&gt;<br/>cpu.max · memory.max · swap 0"]
    plain["sh -c in own process group<br/>(unconfined)"]
    user["root agent: drop to the<br/>submitting user (never root)"]
    stop["walltime / kill list:<br/>cgroup.kill · docker kill · killpg"]
    report["ReportCompletion(exit code)"]

    fetch --> barrier
    barrier -- yes --> wait --> img
    barrier -- no --> img
    img -- yes --> docker
    img -- no --> cg
    cg -- yes --> confined
    cg -- no --> plain
    confined --> user
    plain --> user
    docker --> stop
    user --> stop
    stop --> report
```

The agent probes cgroup v2 and the container runtime once at start-up and logs which mode it uses.
Setting it up on real hosts (systemd delegation, rootless Docker) is covered in
[INSTALL.md](INSTALL.md#job-confinement-with-cgroup-v2).

## Testing and evaluation

| Layer | Where | What it covers |
|---|---|---|
| Unit and integration | `cargo test --workspace` | planner (with a brute-force oracle), OCC, replay, estimators, 3-replica Raft over TCP, agent launch against a fake cgroupfs and a fake `docker` |
| Replay gate | `bench.yml` | production scheduling path on synthetic and SWF traces against `results/replay_baseline.json` |
| Test kit | `testkit/run.sh`, `testkit/multinode.sh` | the same scheduler tests against Gnomon and Slurm, one node or a 3-node container cluster |
| Host validation | `testkit/validate-host.sh` | real cgroup v2 limits, OOM, `cgroup.kill`, real Docker and Podman |
