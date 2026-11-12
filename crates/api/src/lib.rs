//! Phase 3: lock-free async API tier (report section 4.3).
//!
//! Every read handler serves from an immutable FSM snapshot — the scheduler
//! never blocks the API, structurally fixing `slurmctld`'s RPC stalls.
//! `ListJobs` returns the snapshot's Raft-style index; `GetJob(wait_index)`
//! implements Nomad-style blocking queries ("return once state passes index
//! N"), giving clients linearizable-ish reads without polling.

use gnomon_core::{Event, Job, JobId, JobState, Resources, Walltime};
use gnomon_fsm::Fsm;
use gnomon_sched::EvalBroker;
use std::sync::Arc;
use std::time::Duration;
use tonic::{Request, Response, Status};

pub mod pb {
    tonic::include_proto!("gnomon.v1");
}

use pb::scheduler_server::{Scheduler, SchedulerServer};
use pb::*;

pub struct SchedulerSvc {
    pub fsm: Arc<Fsm>,
    pub broker: Arc<EvalBroker>,
}

impl SchedulerSvc {
    pub fn server(self) -> SchedulerServer<Self> { SchedulerServer::new(self) }
}

fn state_name(s: JobState) -> String {
    match s {
        JobState::Pending => "PENDING", JobState::Reserved => "RESERVED",
        JobState::Running => "RUNNING", JobState::Completed => "COMPLETED",
        JobState::Cancelled => "CANCELLED", JobState::Failed => "FAILED",
    }.into()
}

fn job_info(j: &Job) -> JobInfo {
    JobInfo {
        id: j.id.0,
        name: j.name.clone(),
        user: j.user.clone(),
        partition: j.partition.clone(),
        state: state_name(j.state),
        request: Some(pb::Resources {
            cores: j.request.cores, mem_mb: j.request.mem_mb, gpus: j.request.gpus }),
        walltime_secs: j.walltime.secs(),
        priority: j.priority,
        submit_time: j.submit_time,
        planned_start: j.planned_start.unwrap_or(0),
        nodes: vec![],
    }
}

#[tonic::async_trait]
impl Scheduler for SchedulerSvc {
    async fn submit_job(&self, req: Request<SubmitJobRequest>)
        -> Result<Response<SubmitJobReply>, Status>
    {
        let r = req.into_inner();
        let res = r.request.unwrap_or_default();
        let id = self.fsm.next_job_id();
        let mut job = Job::new(id, &r.name, &r.user,
            Resources { cores: res.cores, mem_mb: res.mem_mb, gpus: res.gpus },
            Walltime(Duration::from_secs(r.walltime_secs.max(1))));
        job.partition = if r.partition.is_empty() { "normal".into() } else { r.partition };
        job.priority = r.priority;
        job.command = r.command;
        self.fsm.apply(Event::JobSubmitted(job.clone()));
        self.broker.enqueue(&job, self.fsm.index());
        Ok(Response::new(SubmitJobReply { job_id: id.0 }))
    }

    async fn cancel_job(&self, req: Request<CancelJobRequest>)
        -> Result<Response<CancelJobReply>, Status>
    {
        let r = req.into_inner();
        self.fsm.apply(Event::JobCancelled(JobId(r.job_id)));
        Ok(Response::new(CancelJobReply { ok: true }))
    }

    async fn get_job(&self, req: Request<GetJobRequest>)
        -> Result<Response<GetJobReply>, Status>
    {
        let r = req.into_inner();
        // Index-aware blocking query: wait until state moves past wait_index.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let snap = self.fsm.snapshot();
            if snap.index > r.wait_index {
                let job = snap.job(JobId(r.job_id))
                    .ok_or_else(|| Status::not_found("no such job"))?;
                return Ok(Response::new(GetJobReply { job: Some(job_info(job)) }));
            }
            if std::time::Instant::now() > deadline {
                return Err(Status::deadline_exceeded("state index not reached"));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn list_jobs(&self, req: Request<ListJobsRequest>)
        -> Result<Response<ListJobsReply>, Status>
    {
        let r = req.into_inner();
        let snap = self.fsm.snapshot(); // lock-free read
        let jobs = snap.state().jobs.values()
            .filter(|j| r.user.is_empty() || j.user == r.user)
            .filter(|j| r.state.is_empty() || state_name(j.state) == r.state)
            .map(job_info)
            .collect();
        Ok(Response::new(ListJobsReply { jobs, state_index: snap.index }))
    }

    async fn list_nodes(&self, _req: Request<ListNodesRequest>)
        -> Result<Response<ListNodesReply>, Status>
    {
        let snap = self.fsm.snapshot();
        let now = gnomon_core::now_secs();
        let nodes = snap.nodes().map(|n| NodeInfo {
            name: n.name.clone(),
            capacity: Some(pb::Resources {
                cores: n.capacity.cores, mem_mb: n.capacity.mem_mb, gpus: n.capacity.gpus }),
            allocated: Some(pb::Resources {
                cores: n.allocated.cores, mem_mb: n.allocated.mem_mb, gpus: n.allocated.gpus }),
            last_heartbeat: n.last_heartbeat,
            alive: now.saturating_sub(n.last_heartbeat) < 30,
        }).collect();
        Ok(Response::new(ListNodesReply { nodes }))
    }

    async fn register_node(&self, req: Request<RegisterNodeRequest>)
        -> Result<Response<RegisterNodeReply>, Status>
    {
        let r = req.into_inner();
        let cap = r.capacity.unwrap_or_default();
        self.fsm.apply(Event::NodeRegistered {
            name: r.name,
            capacity: Resources { cores: cap.cores, mem_mb: cap.mem_mb, gpus: cap.gpus },
        });
        // Agents poll for work on this interval; GNOMON_HEARTBEAT_SECS on the
        // controller shortens it (the test kit uses 2 s).
        let hb = std::env::var("GNOMON_HEARTBEAT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
        Ok(Response::new(RegisterNodeReply { ok: true, heartbeat_interval_secs: hb }))
    }

    async fn heartbeat(&self, req: Request<HeartbeatRequest>)
        -> Result<Response<HeartbeatReply>, Status>
    {
        let r = req.into_inner();
        let a = r.allocated.unwrap_or_default();
        self.fsm.apply(Event::Heartbeat {
            name: r.name,
            allocated: Resources { cores: a.cores, mem_mb: a.mem_mb, gpus: a.gpus },
        });
        Ok(Response::new(HeartbeatReply { ok: true }))
    }

    async fn fetch_work(&self, req: Request<FetchWorkRequest>)
        -> Result<Response<FetchWorkReply>, Status>
    {
        let node = req.into_inner().node;
        // Hand out the queued assignments, then drain exactly that many via a
        // replicated event so each job is dispatched once (the skeleton
        // re-sent the whole queue on every poll). Jobs cancelled meanwhile are
        // skipped but still drained.
        let snap = self.fsm.snapshot();
        let queued = snap.state().assignments.get(&node).cloned().unwrap_or_default();
        let assignments = queued.iter()
            .filter_map(|a| {
                let j = snap.job(a.job).filter(|j| j.state == gnomon_core::JobState::Running)?;
                Some(fetch_work_reply::Assignment {
                    job_id: a.job.0, command: a.command.clone(), start: a.start,
                    request: Some(pb::Resources { cores: j.request.cores, mem_mb: j.request.mem_mb, gpus: j.request.gpus }),
                    walltime_secs: j.walltime.secs(), name: j.name.clone(),
                })
            }).collect();
        if !queued.is_empty() {
            self.fsm.apply(Event::AssignmentsFetched { node, count: queued.len() });
        }
        Ok(Response::new(FetchWorkReply { assignments }))
    }

    async fn report_completion(&self, req: Request<ReportCompletionRequest>)
        -> Result<Response<ReportCompletionReply>, Status>
    {
        let r = req.into_inner();
        self.fsm.apply(Event::JobFinished {
            job: JobId(r.job_id), end: gnomon_core::now_secs(), code: r.exit_code });
        Ok(Response::new(ReportCompletionReply { ok: true }))
    }
}
