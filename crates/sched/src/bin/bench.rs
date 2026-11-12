//! Comparative scheduling-core benchmark.
//!
//! Two architectures, identical workload, identical resource graph:
//!
//! * **Serial (Slurm-style)** — the scheduler owns the state: one thread
//!   walks pending jobs in priority order and mutates the reservation
//!   tables directly, decisions and state interleaved under what in
//!   `slurmctld` would be the global job/node/partition mutexes.
//! * **Parallel (gnomon / Omega-style)** — N workers compute plans as pure
//!   functions of one shared immutable snapshot (no locks while deciding);
//!   a serialized plan applier commits them one by one, with the resource
//!   graph revalidating capacity at commit time (optimistic concurrency).
//!   Jobs whose plans conflicted are re-evaluated in a drain pass, modeling
//!   the eval broker's re-enqueue-on-conflict.
//!
//! Metric: end-to-end decision throughput (jobs committed / wall time)
//! across cluster scales and worker counts. Measures the scheduling-core
//! bottleneck identified in the report — not network, RPC, or execcosts.

use gnomon_core::{Event, Job, Resources, Walltime};
use gnomon_fsm::Fsm;
use gnomon_sched::{BackfillPolicy, Plan, Policy};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn make_fsm(nodes: u32) -> Arc<Fsm> {
    let fsm = Arc::new(Fsm::new());
    for n in 0..nodes {
        fsm.apply(Event::NodeRegistered {
            name: format!("node{n:04}"),
            capacity: Resources { cores: 32, mem_mb: 131072, gpus: 4 },
        });
    }
    fsm
}

fn submit_jobs(fsm: &Fsm, count: u64, start_id: u64) {
    for i in 0..count {
        let req = match i % 4 {
            0 => Resources { cores: 16, mem_mb: 32768, gpus: 2 },
            1 => Resources { cores: 8, mem_mb: 16384, gpus: 1 },
            _ => Resources { cores: 4, mem_mb: 8192, gpus: 0 },
        };
        let mut job = Job::new(gnomon_core::JobId(start_id + i),
            &format!("bench-{i}"), "bench", req, Walltime(Duration::from_secs(300)));
        job.priority = 10_000_000 - i;
        fsm.apply(Event::JobSubmitted(job));
    }
}

/// Slurm-style: decide + mutate directly, one serialized pass.
fn bench_serial(fsm: &Fsm) -> (u64, Duration) {
    let t0 = Instant::now();
    let snap = fsm.snapshot();
    let mut jobs: Vec<_> = snap.pending_jobs().cloned().collect();
    jobs.sort_by_key(|j| std::cmp::Reverse(j.priority));
    let mut graph = snap.graph().clone(); // local copy == lock-held state
    let now = gnomon_core::now_secs();
    let mut committed = 0u64;
    let mut events = Vec::new();
    for job in &jobs {
        let dur = job.walltime.0.as_secs();
        if let Some((verts, start)) = graph.match_request(&job.request, now, dur, job.id.0) {
            if graph.commit(&verts, &job.request, start, dur) {
                events.push(Event::JobStarted { job: job.id, vertices: verts, start, dur: None });
                committed += 1;
            }
        }
    }
    for ev in events { fsm.apply(ev); }
    (committed, t0.elapsed())
}

/// gnomon-style: parallel planning against a shared immutable snapshot,
/// serialized optimistic application with commit-time revalidation.
fn bench_parallel(fsm: &Fsm, policy: Arc<dyn Policy>, workers: usize) -> (u64, Duration) {
    let t0 = Instant::now();
    let snap = Arc::new(fsm.snapshot());
    let jobs: Vec<Job> = snap.pending_jobs().cloned().collect();

    // Phase 1 (parallel): pure plan computation, read-only.
    let chunks: Vec<Vec<Job>> = jobs.chunks((jobs.len() / workers.max(1)).max(1))
        .map(|c| c.to_vec()).collect();
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for chunk in chunks {
            let (snap, policy) = (snap.clone(), policy.clone());
            handles.push(s.spawn(move || {
                chunk.iter().map(|j| policy.schedule(&snap, j)).collect::<Vec<Plan>>()
            }));
        }
        let mut plans: Vec<Plan> = Vec::new();
        for h in handles.drain(..) { plans.extend(h.join().unwrap()); }

        // Phase 2 (serial applier): commit; the FSM's graph layer
        // revalidates every reservation at commit time. Conflicts are
        // simply not committed (job stays Pending).
        let mut applied_events = 0u64;
        for plan in &plans {
            let Some(ev) = plan.decision.to_event(plan.job) else { continue };
            fsm.apply(ev);
            applied_events += 1;
        }

        // Phase 3: drain pass — re-evaluate conflicted/unplaced jobs
        // serially (models broker re-enqueue; bounded passes).
        loop {
            let snap2 = fsm.snapshot();
            let pending: Vec<Job> = snap2.pending_jobs().cloned().collect();
            if pending.is_empty() { break; }
            let mut progress = false;
            let now = gnomon_core::now_secs();
            let mut graph = snap2.graph().clone();
            let mut evs = Vec::new();
            for j in &pending {
                let dur = j.walltime.0.as_secs();
                if let Some((verts, start)) = graph.match_request(&j.request, now, dur, j.id.0) {
                    if graph.commit(&verts, &j.request, start, dur) {
                        evs.push(Event::JobStarted { job: j.id, vertices: verts, start, dur: None });
                        progress = true;
                    }
                }
            }
            for ev in evs { fsm.apply(ev); applied_events += 1; }
            if !progress { break; }
        }
        let committed = fsm.snapshot().state().jobs.values()
            .filter(|j| !matches!(j.state, gnomon_core::JobState::Pending))
            .count() as u64;
        (committed, t0.elapsed())
    })
}

fn main() {
    let policy: Arc<dyn Policy> = Arc::new(BackfillPolicy::new(72 * 3600));
    println!("arch,nodes,jobs,workers,committed,millis,jobs_per_sec");
    for &(nodes, jobs) in &[(64u32, 256u64), (256, 1024), (512, 4096), (1024, 8192)] {
        let fsm = make_fsm(nodes);
        submit_jobs(&fsm, jobs, 1);
        let (c, d) = bench_serial(&fsm);
        println!("serial,{nodes},{jobs},1,{c},{},{:.0}", d.as_millis(), c as f64 / d.as_secs_f64());

        for workers in [1usize, 2, 4, 8] {
            let fsm = make_fsm(nodes);
            submit_jobs(&fsm, jobs, 1);
            let (c, d) = bench_parallel(&fsm, policy.clone(), workers);
            println!("parallel,{nodes},{jobs},{workers},{c},{},{:.0}", d.as_millis(), c as f64 / d.as_secs_f64());
        }
    }
}
