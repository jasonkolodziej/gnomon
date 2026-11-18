# Gnomon and Kubernetes: the gaps, and where Gnomon can differentiate

Research note, November 2026. It asks two questions:

- Where Gnomon falls short of Kubernetes, can it set itself apart anyway?
- Which of those gaps are worth closing?

**Short answer:** Gnomon cannot compete on Kubernetes' strengths, and it should not try to. Those strengths are services, networking, the platform and its ecosystem. Gnomon can differentiate in two ways:

1. **Depth where Kubernetes is still shallow.** Planning on a time axis with walltimes, reservations and backfill. A replicated, deterministic scheduler state. Sandboxed policy. The Slurm interface.
2. **Coexistence, which turns the gaps into Kubernetes' job.** The market is already building "Slurm on/with Kubernetes" products. That shows the demand is for an HPC scheduler that *plugs into* Kubernetes, not one that replaces it.

The recommended path is a Kubernetes bridge (§5, D1), a few small batch features that are cheap to add (D4), and a measured comparison (D5). Each is below.

Nothing here has been measured against Kubernetes, Kueue or Volcano (the design document says the same in its "What has **not** been measured" paragraph). Every comparison is about features and architecture, not performance.

## 1. The gaps

The table lists what Kubernetes does that Gnomon doesn't, as of this branch. It was checked against the code:

- no preemption
- no job arrays or dependencies
- no Prometheus endpoint
- a node loss requeues only work that never started there (`crates/fsm/src/lib.rs`, `requeue`)

| Kubernetes strength | Gnomon today | Close it? |
|---|---|---|
| Long-running services: restarts, rescheduling after node loss | Every job has a walltime; a crash or node loss ends a started job as FAILED | **Concede.** Run services on Kubernetes |
| Service discovery, load balancing, ingress | Host networking; no discovery | **Concede** |
| Replicas, autoscaling, rolling updates | One job is one placement; a gang is a fixed set of ranks | **Concede** |
| Liveness and readiness probes | None | **Concede** for services. A one-job helper server can poll its own port |
| Storage: CSI, PersistentVolumes | Shared filesystem assumed, as in Slurm | **Concede.** Same model as HPC sites |
| Multi-tenancy: RBAC, namespaces, quotas, admission webhooks | Runs as the submitting user; the admission WASM plugin rejects or rewrites; the gRPC API has no authentication | **Partly close.** Authentication on the API is needed regardless (§6) |
| Ecosystem: Helm, operators, observability, cloud integrations | Single binaries, CLI and gRPC, the web UI on `claude/web-ui` | **Bridge** (§5) |
| Elasticity: cluster autoscaler, Karpenter | Static node set; agents register on start | **Bridge.** Let Kubernetes provide the nodes |
| Maturity and production use | Young research code | Only time and measurement close this |
| Preemption and priority classes | Priority orders the queue; nothing is evicted | **Close** (D4). Batch schedulers need it too |
| Job arrays, dependencies, requeue of started work | None | **Close** (D4). Slurm users expect them |

Most of the gaps are the *service* half of Kubernetes. Gnomon doesn't need that half, because no batch scheduler competes there. The batch-side gaps (preemption, arrays, dependencies, requeue) are what Slurm users will notice, and they are inside Gnomon's scope.

## 2. How the Kubernetes batch stack is closing in (late 2026)

Kubernetes has been adding batch features for years. Where it stands now:

- **Native gang scheduling.** KEP-4671 adds a core `Workload` type and `PodGroup`.
  - It went alpha in 1.35 and was still alpha in 1.36 ([Palark, 1.35](https://palark.com/blog/kubernetes-1-35-release-features/), [Palark, 1.36](https://palark.com/blog/kubernetes-1-36-release-features/)).
  - Kubernetes 1.37 (26 August 2026) promoted the Workload and PodGroup APIs to beta, with workload-aware preemption and an alpha `CompositePodGroup` for hierarchical groups ([1.37 release](https://kubernetes.io/blog/2026/08/26/kubernetes-v1-37-release/)).
  - Reports say the beta ships **disabled by default**, and that a Job-level `minCount` needs a further alpha gate, `WorkloadWithJob` ([Zetta360](https://www.zetta360.com/blog/gang-scheduling-is-beta-and-beta-means-off), [SREKubeCraft](https://srekubecraft.io/posts/kubernetes-gang-scheduling-workload-api/)).
  - Sources disagree on the API version (`v1beta1` or `v1alpha3`). Check the release notes before citing it.
  - I could not open these pages myself (the session's network blocked them); the details come from search summaries.
- **Kueue.** Admission and queueing alongside kube-scheduler.
  - Topology-aware scheduling has been beta and on by default since v0.14; multi-layer topology is still an alpha gate ([Kueue TAS](https://kueue.sigs.k8s.io/v0.20/docs/tasks/run/topology_aware_scheduling/)).
  - MultiKueue sends work across clusters, combined with TAS since v0.15, still alpha ([MultiKueue](https://kueue.sigs.k8s.io/docs/tasks/manage/setup_multikueue/)).
  - JobSet is supported ([features](https://kueue.sigs.k8s.io/docs/overview/)). Its roadmap includes integrating with the native workload-aware scheduler ([README](https://github.com/kubernetes-sigs/kueue/blob/main/README.md)).
- **Volcano.**
  - Has gang, queue, fair-share, reserve and backfill as scheduler actions and plugins ([CNCF](https://www.cncf.io/blog/2021/02/26/volcano-collision-between-containers-and-batch-computing/)).
  - Its `backfill` targets pods without resource requests, and its `reserve` holds nodes for a starving job. Neither plans on a time axis from declared walltimes, as EASY or conservative backfill does. This is from Volcano's docs as I understand them; it was not tested.
  - One group replaced Volcano with its own scheduler: Volcano's eviction and reclaim work per node, and its topology lives in the API server and etcd ([Laguna technical report](https://arxiv.org/pdf/2605.27605)).
- **Fluence and the Flux Operator (LLNL).**
  - Fluence puts Fluxion's graph-based matching into Kubernetes as a scheduler plugin; it has run MPI applications at up to 3,000 ranks ([Flux Operator paper](https://arxiv.org/pdf/2309.17420)).
  - The Flux Operator runs a whole Flux instance *inside* Kubernetes ([same paper](https://arxiv.org/pdf/2309.17420), [converged computing](https://arxiv.org/pdf/2109.03739)).
- **Slurm with Kubernetes.**
  - **Slinky** (SchedMD, now developed with NVIDIA) has two parts ([Slinky docs](https://slinky.schedmd.com/docs/), [NVIDIA](https://developer.nvidia.com/blog/running-large-scale-gpu-workloads-on-kubernetes-with-slurm/)):
    - `slurm-operator` runs Slurm's daemons as pods, with one worker pod per node.
    - `slurm-bridge` makes Slurm a Kubernetes scheduler. Each pod becomes a placeholder Slurm job, and the bridge binds pods to the nodes Slurm allocates. It gives each pod a whole node exclusively ([slurm-bridge](https://slinky.schedmd.com/docs/slurm-bridge/)).
  - **CoreWeave SUNK** runs Slurm nodes as pods and lets Slurm schedule alongside Kubernetes workloads, "Slurm for finite batch, Kubernetes for long-running inference" ([SUNK docs](https://docs.coreweave.com/products/sunk.md), [CoreWeave](https://www.coreweave.com/blog/sunk-slurm-on-kubernetes-implementations)).
- **Kubernetes on HPC.**
  - **HPK** is a Virtual Kubelet that turns pods into Slurm and Apptainer jobs and hands all scheduling to Slurm ([HPK paper](https://arxiv.org/pdf/2409.16919), [repo](https://github.com/CARV-ICS-FORTH/HPK)).
  - **interLink** (CNCF) generalises this with plugins for Slurm and HTCondor ([interLink](https://github.com/interlink-hq/interlink)).

**What this means for Gnomon:**

- **The gap on gang scheduling is closing.** "Gang scheduling built in" stops being a differentiator once the native Workload API is on by default. Gnomon should not lead with it.
- **The gap on time-axis planning is not closing.** No project above plans the future from declared walltimes, with reservations and a no-delay backfill guarantee: not native Kubernetes, not Kueue, not Volcano. Kueue decides *whether* a workload is admitted now. kube-scheduler decides *where* a pod goes now. Neither decides *when* in the future. Fluxion, and so Fluence, does plan on a time-indexed graph, so it is the closest relative. Slurm does it too, which is why the Slurm-on-Kubernetes products exist.
- **The industry answer is coexistence.** Slinky, SUNK, HPK and interLink all assume a site wants one HPC scheduler and Kubernetes together. That is the slot Gnomon should aim for.

## 3. What Gnomon has that the Kubernetes batch stack lacks

These are claims about Gnomon's design, each with where to find it in the code. None of them is a measured advantage.

1. **Planning on a time axis.** Per-node planners hold running spans and future reservations. Backfill places short jobs in gaps without delaying any reservation (`crates/sched`, `crates/graph`). The timeline view on `claude/web-ui` draws this plan.
2. **Runtime estimates in the plan.** Quantile and bias-corrected predictors, plus a WASM estimator hook (ADR 0004). These fill the gap between requested and actual walltime, which is where backfill gains or loses.
3. **One deterministic, replicated state machine.** Every write goes through Raft. Each plan carries its read set, and every replica re-checks it, so all replicas accept or reject a plan alike (ADR 0001). A journal can be replayed bit-for-bit. Kubernetes coordinates many controllers through etcd with eventual reconciliation. That is fine for services, but it makes "why did this job start there?" hard to answer.
4. **Sandboxed policy.** Admission, priority, shadow priority, estimator and node-score hooks run as WASM components with fuel and memory limits, and fall back on failure (ADRs 0002–0004). Kubernetes scheduler plugins are Go compiled into the scheduler; extenders and admission webhooks are network calls. A *shadow* priority, compared but never applied, lets a site trial a policy on live traffic. I found no counterpart in the projects above.
5. **The Slurm interface** (`gnbatch`, `gnqueue`, `gncancel`, `#SBATCH`). Existing scripts and user habits carry over. Slinky and SUNK get this by being Slurm; Kueue and Volcano do not have it.
6. **Fewer moving parts.** Controller, agent and CLIs, with no etcd and no operator stack. The design document quotes AMD's Spur team on Kubernetes batch needing "a full ecosystem of operators, custom schedulers, and queueing layers".

## 4. Can Gnomon differentiate on the gaps themselves?

On the *service* gaps, no. Re-implementing Deployments, Services or ingress would make Gnomon a worse Kubernetes. But the gaps can still be turned into a position:

- **The gap as scope.** "Gnomon schedules finite work over time; Kubernetes keeps services up." SUNK markets the same split.
- **Bridge the gap and keep the edge.** Gnomon's planner decides *when* and *where*; Kubernetes runs the pod and provides networking, storage and secrets. Compared with `slurm-bridge`:
  - Gnomon is not tied to whole-node exclusive allocation.
  - Gnomon's state is replicated by Raft without an external database.
  - Gnomon's policy is sandboxed.

  These are plausible advantages, not demonstrated ones.
- **Close the batch-side gaps cheaply.** Preemption, arrays, dependencies and requeue are inside Gnomon's scope, and Slurm users miss them first.

## 5. Options, in recommended order

| # | Option | What it is | Effort | Payoff |
|---|---|---|---|---|
| D1 | **Kubernetes scheduler bridge** | A `gnomon-k8s` controller watches pods with `schedulerName: gnomon` (and Workload/PodGroup or JobSet for gangs). It submits each as a job with the pod's requests and an annotation for walltime, then binds the pods to the nodes Gnomon plans, so the plan and its reservations stay visible. Node agents run as a DaemonSet or reuse the kubelet's cgroups | Medium | Highest. Gnomon becomes usable on any Kubernetes cluster and competes with `slurm-bridge` on its own ground |
| D2 | **Virtual Kubelet provider** | Gnomon appears as one or more virtual nodes; pods sent there become Gnomon jobs (HPK and interLink pattern) | Medium | Lets Kubernetes users send batch pods to an existing Gnomon (HPC) cluster without touching the HPC side |
| D3 | **Helm chart / operator** | Run Raft controllers as a StatefulSet and agents as a DaemonSet with host cgroup access (`slurm-operator` and SUNK pattern) | Low–medium | Easy evaluation; needed by D1 anyway |
| D4 | **Batch-side gaps** | Priority preemption (log a `JobPreempted` event so replicas agree), requeue of started work on node loss (opt in per job, as Slurm's `--requeue`), job arrays, `afterok` dependencies, a Prometheus endpoint | Small each | Removes the first objections from Slurm users; independent of Kubernetes |
| D5 | **Measure** | Run one workload on a kind or k3d cluster under Kueue + kube-scheduler, Volcano, and Gnomon via D1. Compare wait time, slowdown, utilisation and gang start skew. The lab view and `gnomon-sim` already produce Gnomon's side from the same trace | Medium | Turns §3 from claims into evidence; the design document asks for exactly this |
| — | Service features (Deployments, Services, ingress, autoscaling) | — | Large | **Don't.** Every hour spent here makes Gnomon a worse Kubernetes |

D1 and D3 together are the "Slinky without Slurm" story. Its pitch is a Rust scheduler with Slurm's interface, time-axis planning, replicated state and sandboxed policy, plugged into Kubernetes rather than competing with it. D2 serves the opposite case: an HPC site that keeps Gnomon as the system of record and admits Kubernetes users.

## 6. Risks and open questions

- **Native catch-up.** Once the Workload API is on by default and Kueue integrates with it, the gang scheduling and queueing gaps will mostly close. Time-axis planning would then be Gnomon's main scheduling edge. If Kubernetes or Kueue ever adds reservation planning from declared runtimes, revisit this note.
- **Slinky has the lead and the backing.** It is real Slurm, maintained by its authors and NVIDIA, with users already. Gnomon has to win on what Slurm cannot easily change: Rust, replicated deterministic state, sandboxed policy, the replayable journal. Being "like Slurm" is not enough.
- **Authentication.** The gRPC API has none (which is why `gnomon-web` binds to localhost). Any Kubernetes integration exposes it to a multi-tenant control plane, so mTLS or token authentication has to come before D1 ships.
- **Two sources of truth.** A bridge means pods (in etcd) and jobs (in Gnomon's log) describe the same work. `slurm-bridge` handles this with placeholder jobs. D1 needs a clear rule that Gnomon owns *when*, Kubernetes owns *what runs*, and failures in either side reach the other.
- **Unmeasured.** Until D5 is done, nothing in §3 should be written as a performance claim.

## Sources

- Kubernetes gang scheduling:
  - [Kubernetes v1.37 release](https://kubernetes.io/blog/2026/08/26/kubernetes-v1-37-release/)
  - [Palark: 1.35 alpha features](https://palark.com/blog/kubernetes-1-35-release-features/)
  - [Palark: 1.36 alpha features](https://palark.com/blog/kubernetes-1-36-release-features/)
  - [ScaleOps: 1.36 resource management](https://scaleops.com/blog/kubernetes-1-36/)
  - [Zetta360: gang scheduling is beta and off](https://www.zetta360.com/blog/gang-scheduling-is-beta-and-beta-means-off)
  - [SREKubeCraft: 1.37 Workload API](https://srekubecraft.io/posts/kubernetes-gang-scheduling-workload-api/)
  - [1.37 gang scheduling vs Volcano and Kueue](https://iotdigitaltwinplm.com/kubernetes-1-37-gang-scheduling-vs-volcano-vs-kueue-2026/)
  - [TechTimes on 1.37](https://www.techtimes.com/articles/325879/20260828/kubernetes-137-lands-gang-scheduling-beta-cuts-gpu-idle-costs-default.htm)
- Kueue:
  - [features overview](https://kueue.sigs.k8s.io/docs/overview/)
  - [topology-aware scheduling](https://kueue.sigs.k8s.io/v0.20/docs/tasks/run/topology_aware_scheduling/)
  - [TAS KEP](https://github.com/kubernetes-sigs/kueue/blob/main/keps/2724-topology-aware-scheduling/README.md)
  - [MultiKueue setup](https://kueue.sigs.k8s.io/docs/tasks/manage/setup_multikueue/)
  - [README and roadmap](https://github.com/kubernetes-sigs/kueue/blob/main/README.md)
- Volcano:
  - [CNCF: Volcano](https://www.cncf.io/blog/2021/02/26/volcano-collision-between-containers-and-batch-computing/)
  - [The New Stack](https://thenewstack.io/volcano-a-kubernetes-native-batch-system-for-ai-big-data-and-hpc-workloads/)
  - [Laguna technical report](https://arxiv.org/pdf/2605.27605)
  - [Evaluation of Kubernetes schedulers (PEARC'24)](https://dl.acm.org/doi/fullHtml/10.1145/3626203.3670520)
- Flux:
  - [The Flux Operator](https://arxiv.org/pdf/2309.17420)
  - [A dynamic, hierarchical resource model for converged computing](https://arxiv.org/pdf/2109.03739)
- Slurm with Kubernetes:
  - [Slinky docs](https://slinky.schedmd.com/docs/)
  - [slurm-bridge](https://slinky.schedmd.com/docs/slurm-bridge/)
  - [NVIDIA: GPU workloads on Kubernetes with Slurm](https://developer.nvidia.com/blog/running-large-scale-gpu-workloads-on-kubernetes-with-slurm/)
  - [SchedMD: Slinky](https://slurm.schedmd.com/slinky.html)
  - [CoreWeave SUNK docs](https://docs.coreweave.com/products/sunk.md)
  - [Introducing SUNK](https://www.coreweave.com/blog/sunk-slurm-on-kubernetes-implementations)
- Kubernetes on HPC:
  - [HPK paper](https://arxiv.org/pdf/2409.16919)
  - [HPK repo](https://github.com/CARV-ICS-FORTH/HPK)
  - [interLink](https://github.com/interlink-hq/interlink)
- Gnomon's own:
  - [design document](SLURM_Scheduler_Research_and_Rust_Design.md)
  - ADRs 0001–0005 in `.github/plans/`
