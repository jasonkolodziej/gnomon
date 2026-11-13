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

#[allow(clippy::result_large_err)]
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
    pub fn server(self) -> SchedulerServer<Self> {
        SchedulerServer::new(self)
    }
}

fn state_name(s: JobState) -> String {
    match s {
        JobState::Pending => "PENDING",
        JobState::Reserved => "RESERVED",
        JobState::Running => "RUNNING",
        JobState::Completed => "COMPLETED",
        JobState::Cancelled => "CANCELLED",
        JobState::Failed => "FAILED",
    }
    .into()
}

/// Wire view of a job. `nodes` lists every node the job holds (all ranks
/// of a gang, in rank order) once it has been placed; `planned_dur` falls
/// back to the walltime when the scheduler has not chosen a shorter
/// (predicted) span yet, so clients such as gntop can draw the plan.
fn job_info(j: &Job, nodes: Option<&Vec<String>>) -> JobInfo {
    JobInfo {
        id: j.id.0,
        name: j.name.clone(),
        user: j.user.clone(),
        partition: j.partition.clone(),
        state: state_name(j.state),
        request: Some(pb::Resources {
            cores: j.request.cores,
            mem_mb: j.request.mem_mb,
            gpus: j.request.gpus,
        }),
        walltime_secs: j.walltime.secs(),
        priority: j.priority,
        submit_time: j.submit_time,
        planned_start: j.planned_start.unwrap_or(0),
        nodes: nodes.cloned().unwrap_or_default(),
        num_nodes: j.node_count(),
        planned_dur: j.planned_dur.unwrap_or_else(|| j.walltime.secs()),
        container: j.container.clone().unwrap_or_default(),
    }
}

#[tonic::async_trait]
impl Scheduler for SchedulerSvc {
    async fn submit_job(
        &self,
        req: Request<SubmitJobRequest>,
    ) -> Result<Response<SubmitJobReply>, Status> {
        let r = req.into_inner();
        let res = r.request.unwrap_or_default();
        let id = self.fsm.next_job_id();
        let mut job = Job::new(
            id,
            &r.name,
            &r.user,
            Resources {
                cores: res.cores,
                mem_mb: res.mem_mb,
                gpus: res.gpus,
            },
            Walltime(Duration::from_secs(r.walltime_secs.max(1))),
        );
        job.partition = if r.partition.is_empty() {
            "normal".into()
        } else {
            r.partition
        };
        job.priority = r.priority;
        job.command = r.command;
        job.nodes = r.nodes.max(1);
        job.container = Some(r.container).filter(|c| !c.is_empty());
        self.fsm.apply(Event::JobSubmitted(job.clone()));
        self.broker.enqueue(&job, self.fsm.index());
        Ok(Response::new(SubmitJobReply { job_id: id.0 }))
    }

    async fn cancel_job(
        &self,
        req: Request<CancelJobRequest>,
    ) -> Result<Response<CancelJobReply>, Status> {
        let r = req.into_inner();
        self.fsm.apply(Event::JobCancelled(JobId(r.job_id)));
        Ok(Response::new(CancelJobReply { ok: true }))
    }

    async fn get_job(&self, req: Request<GetJobRequest>) -> Result<Response<GetJobReply>, Status> {
        let r = req.into_inner();
        // Index-aware blocking query: wait until state moves past wait_index.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let snap = self.fsm.snapshot();
            if snap.index > r.wait_index {
                let job = snap
                    .job(JobId(r.job_id))
                    .ok_or_else(|| Status::not_found("no such job"))?;
                let nodes = snap.state().allocations.get(&job.id);
                return Ok(Response::new(GetJobReply {
                    job: Some(job_info(job, nodes)),
                }));
            }
            if std::time::Instant::now() > deadline {
                return Err(Status::deadline_exceeded("state index not reached"));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn list_jobs(
        &self,
        req: Request<ListJobsRequest>,
    ) -> Result<Response<ListJobsReply>, Status> {
        let r = req.into_inner();
        let snap = self.fsm.snapshot(); // lock-free read
        let jobs = snap
            .state()
            .jobs
            .values()
            .filter(|j| r.user.is_empty() || j.user == r.user)
            .filter(|j| r.state.is_empty() || state_name(j.state) == r.state)
            .map(|j| job_info(j, snap.state().allocations.get(&j.id)))
            .collect();
        Ok(Response::new(ListJobsReply {
            jobs,
            state_index: snap.index,
        }))
    }

    async fn list_nodes(
        &self,
        _req: Request<ListNodesRequest>,
    ) -> Result<Response<ListNodesReply>, Status> {
        let snap = self.fsm.snapshot();
        let now = gnomon_core::now_secs();
        let nodes = snap
            .nodes()
            .map(|n| NodeInfo {
                name: n.name.clone(),
                capacity: Some(pb::Resources {
                    cores: n.capacity.cores,
                    mem_mb: n.capacity.mem_mb,
                    gpus: n.capacity.gpus,
                }),
                allocated: Some(pb::Resources {
                    cores: n.allocated.cores,
                    mem_mb: n.allocated.mem_mb,
                    gpus: n.allocated.gpus,
                }),
                last_heartbeat: n.last_heartbeat,
                alive: now.saturating_sub(n.last_heartbeat) < 30,
            })
            .collect();
        Ok(Response::new(ListNodesReply { nodes }))
    }

    async fn register_node(
        &self,
        req: Request<RegisterNodeRequest>,
    ) -> Result<Response<RegisterNodeReply>, Status> {
        let r = req.into_inner();
        let cap = r.capacity.unwrap_or_default();
        self.fsm.apply(Event::NodeRegistered {
            name: r.name,
            capacity: Resources {
                cores: cap.cores,
                mem_mb: cap.mem_mb,
                gpus: cap.gpus,
            },
        });
        // Agents poll for work on this interval; GNOMON_HEARTBEAT_SECS on the
        // controller shortens it (the test kit uses 2 s).
        let hb = std::env::var("GNOMON_HEARTBEAT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);
        Ok(Response::new(RegisterNodeReply {
            ok: true,
            heartbeat_interval_secs: hb,
        }))
    }

    async fn heartbeat(
        &self,
        req: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatReply>, Status> {
        let r = req.into_inner();
        let a = r.allocated.unwrap_or_default();
        self.fsm.apply(Event::Heartbeat {
            name: r.name,
            allocated: Resources {
                cores: a.cores,
                mem_mb: a.mem_mb,
                gpus: a.gpus,
            },
        });
        Ok(Response::new(HeartbeatReply { ok: true }))
    }

    async fn fetch_work(
        &self,
        req: Request<FetchWorkRequest>,
    ) -> Result<Response<FetchWorkReply>, Status> {
        let node = req.into_inner().node;
        // Hand out the queued assignments, then drain exactly that many via a
        // replicated event so each job is dispatched once (the skeleton
        // re-sent the whole queue on every poll). Jobs cancelled meanwhile are
        // skipped but still drained.
        let snap = self.fsm.snapshot();
        let queued = snap
            .state()
            .assignments
            .get(&node)
            .cloned()
            .unwrap_or_default();
        let assignments = queued
            .iter()
            .filter_map(|a| {
                let j = snap
                    .job(a.job)
                    .filter(|j| j.state == gnomon_core::JobState::Running)?;
                Some(fetch_work_reply::Assignment {
                    job_id: a.job.0,
                    command: a.command.clone(),
                    start: a.start,
                    request: Some(pb::Resources {
                        cores: j.request.cores,
                        mem_mb: j.request.mem_mb,
                        gpus: j.request.gpus,
                    }),
                    walltime_secs: j.walltime.secs(),
                    name: j.name.clone(),
                    container: j.container.clone().unwrap_or_default(),
                    nodelist: a.nodelist.clone(),
                    rank: a.rank,
                })
            })
            .collect();
        if !queued.is_empty() {
            self.fsm.apply(Event::AssignmentsFetched {
                node,
                count: queued.len(),
            });
        }
        Ok(Response::new(FetchWorkReply { assignments }))
    }

    async fn report_completion(
        &self,
        req: Request<ReportCompletionRequest>,
    ) -> Result<Response<ReportCompletionReply>, Status> {
        let r = req.into_inner();
        let job = JobId(r.job_id);
        let end = gnomon_core::now_secs();
        // Gang ranks report individually; the FSM aggregates them so replay
        // reproduces exactly when the job completed. Single-node jobs keep
        // the plain JobFinished event.
        let gang = self
            .fsm
            .read(|st| st.jobs.get(&job).is_some_and(Job::is_gang));
        let ev = if gang {
            Event::JobRankFinished {
                job,
                node: r.node,
                end,
                code: r.exit_code,
            }
        } else {
            Event::JobFinished {
                job,
                end,
                code: r.exit_code,
            }
        };
        let ok = self.fsm.apply_checked(ev).accepted;
        Ok(Response::new(ReportCompletionReply { ok }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnomon_core::SystemClock;
    use gnomon_sched::{backfill_pass, BackfillPolicy, PassOptions, PlanApplier};

    fn svc() -> SchedulerSvc {
        SchedulerSvc {
            fsm: Arc::new(Fsm::recording(Arc::new(SystemClock))),
            broker: Arc::new(EvalBroker::new()),
        }
    }

    async fn register(s: &SchedulerSvc, name: &str) {
        s.register_node(Request::new(RegisterNodeRequest {
            name: name.into(),
            capacity: Some(pb::Resources {
                cores: 4,
                mem_mb: 1024,
                gpus: 0,
            }),
        }))
        .await
        .unwrap();
    }

    async fn submit(s: &SchedulerSvc, nodes: u32) -> u64 {
        s.submit_job(Request::new(SubmitJobRequest {
            name: "mpi".into(),
            user: "u".into(),
            partition: String::new(),
            request: Some(pb::Resources {
                cores: 4,
                mem_mb: 0,
                gpus: 0,
            }),
            walltime_secs: 600,
            priority: 1,
            command: "app".into(),
            nodes,
            container: String::new(),
        }))
        .await
        .unwrap()
        .into_inner()
        .job_id
    }

    async fn job(s: &SchedulerSvc, id: u64) -> JobInfo {
        s.get_job(Request::new(GetJobRequest {
            job_id: id,
            wait_index: 0,
        }))
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap()
    }

    async fn report(s: &SchedulerSvc, node: &str, id: u64, code: i32) -> bool {
        s.report_completion(Request::new(ReportCompletionRequest {
            node: node.into(),
            job_id: id,
            exit_code: code,
        }))
        .await
        .unwrap()
        .into_inner()
        .ok
    }

    fn schedule(s: &SchedulerSvc) {
        backfill_pass(
            &s.fsm,
            &BackfillPolicy::default(),
            &PlanApplier::new(),
            PassOptions::default(),
        );
    }

    #[tokio::test]
    async fn gang_job_flows_through_submit_fetch_and_per_rank_completion() {
        let s = svc();
        for n in ["a", "b", "c"] {
            register(&s, n).await;
        }
        let id = submit(&s, 2).await;
        let pending = job(&s, id).await;
        assert_eq!(pending.num_nodes, 2);
        assert!(pending.nodes.is_empty());
        schedule(&s);
        let info = job(&s, id).await;
        assert_eq!(info.state, "RUNNING");
        assert_eq!(info.nodes.len(), 2, "every node of the gang is listed");
        let listed = s
            .list_jobs(Request::new(ListJobsRequest {
                user: String::new(),
                state: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(listed.jobs[0].nodes, info.nodes);

        for (rank, node) in info.nodes.iter().enumerate() {
            let work = s
                .fetch_work(Request::new(FetchWorkRequest { node: node.clone() }))
                .await
                .unwrap()
                .into_inner()
                .assignments;
            assert_eq!(work.len(), 1);
            assert_eq!(work[0].rank, rank as u32);
            assert_eq!(work[0].nodelist, info.nodes);
            assert_eq!(work[0].job_id, id);
        }
        // First rank done: the gang keeps running; a foreign node is refused.
        assert!(report(&s, &info.nodes[1], id, 0).await);
        assert_eq!(job(&s, id).await.state, "RUNNING");
        let outsider = ["a", "b", "c"]
            .into_iter()
            .find(|n| !info.nodes.iter().any(|m| m == n))
            .unwrap();
        assert!(!report(&s, outsider, id, 0).await);
        assert!(report(&s, &info.nodes[0], id, 0).await);
        assert_eq!(job(&s, id).await.state, "COMPLETED");
        let ranks = s
            .fsm
            .journal()
            .iter()
            .filter(|e| matches!(e.event, Event::JobRankFinished { .. }))
            .count();
        assert_eq!(ranks, 3);
    }

    #[tokio::test]
    async fn single_node_job_keeps_job_finished_and_lists_its_node() {
        let s = svc();
        register(&s, "a").await;
        let id = submit(&s, 0).await; // 0 = unset on the wire -> 1 node
        schedule(&s);
        let info = job(&s, id).await;
        assert_eq!(info.num_nodes, 1);
        assert_eq!(info.nodes, vec!["a".to_string()]);
        let work = s
            .fetch_work(Request::new(FetchWorkRequest { node: "a".into() }))
            .await
            .unwrap()
            .into_inner()
            .assignments;
        assert_eq!(
            (work[0].rank, work[0].nodelist.clone()),
            (0, vec!["a".into()])
        );
        assert!(report(&s, "a", id, 1).await);
        assert_eq!(job(&s, id).await.state, "FAILED");
        let journal = s.fsm.journal();
        assert!(matches!(
            journal.last().unwrap().event,
            Event::JobFinished { code: 1, .. }
        ));
    }
}

#[cfg(test)]
mod container_tests {
    use super::*;

    fn svc() -> SchedulerSvc {
        SchedulerSvc {
            fsm: Arc::new(Fsm::new()),
            broker: Arc::new(EvalBroker::new()),
        }
    }

    fn submit(container: &str) -> SubmitJobRequest {
        SubmitJobRequest {
            name: "c".into(),
            user: "u".into(),
            partition: String::new(),
            request: Some(pb::Resources {
                cores: 1,
                mem_mb: 0,
                gpus: 0,
            }),
            walltime_secs: 60,
            priority: 1,
            command: "true".into(),
            nodes: 1,
            container: container.into(),
        }
    }

    async fn submit_id(s: &SchedulerSvc, container: &str) -> u64 {
        s.submit_job(Request::new(submit(container)))
            .await
            .unwrap()
            .into_inner()
            .job_id
    }

    #[tokio::test]
    async fn container_image_flows_from_submit_to_job_info_and_fetch_work() {
        let s = svc();
        let id = submit_id(&s, "alpine:3.20").await;
        assert_eq!(
            s.fsm
                .snapshot()
                .job(JobId(id))
                .unwrap()
                .container
                .as_deref(),
            Some("alpine:3.20")
        );
        let jobs = s
            .list_jobs(Request::new(ListJobsRequest::default()))
            .await
            .unwrap()
            .into_inner()
            .jobs;
        assert_eq!(jobs[0].container, "alpine:3.20");

        s.register_node(Request::new(RegisterNodeRequest {
            name: "n0".into(),
            capacity: Some(pb::Resources {
                cores: 4,
                mem_mb: 1024,
                gpus: 0,
            }),
        }))
        .await
        .unwrap();
        let vertex = s.fsm.read(|st| st.graph.by_name("n0").unwrap());
        s.fsm.apply(Event::JobStarted {
            job: JobId(id),
            vertices: vec![vertex],
            start: gnomon_core::now_secs(),
            dur: None,
        });
        let work = s
            .fetch_work(Request::new(FetchWorkRequest { node: "n0".into() }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(work.assignments.len(), 1);
        assert_eq!(work.assignments[0].container, "alpine:3.20");
        assert_eq!(work.assignments[0].nodelist, vec!["n0".to_string()]);
        assert_eq!(work.assignments[0].rank, 0);
    }

    #[tokio::test]
    async fn empty_container_means_none() {
        let s = svc();
        let id = submit_id(&s, "").await;
        assert_eq!(s.fsm.snapshot().job(JobId(id)).unwrap().container, None);
    }
}

#[cfg(test)]
mod job_info_tests {
    use super::*;
    use gnomon_core::ManualClock;

    fn fsm_with_node() -> Fsm {
        let fsm = Fsm::with_clock(Arc::new(ManualClock::new(1000)));
        fsm.apply(Event::NodeRegistered {
            name: "n0".into(),
            capacity: Resources {
                cores: 4,
                mem_mb: 0,
                gpus: 0,
            },
        });
        let req = Resources {
            cores: 2,
            mem_mb: 0,
            gpus: 0,
        };
        let wall = Walltime(Duration::from_secs(600));
        fsm.apply(Event::JobSubmitted(Job::new(JobId(1), "a", "u", req, wall)));
        fsm.apply(Event::JobSubmitted(Job::new(JobId(2), "b", "u", req, wall)));
        fsm
    }

    #[test]
    fn job_info_reports_planned_span_and_nodes_once_placed() {
        let fsm = fsm_with_node();
        let n0 = fsm.read(|s| s.graph.by_name("n0")).unwrap();
        fsm.apply(Event::JobStarted {
            job: JobId(1),
            vertices: vec![n0],
            start: 1000,
            dur: Some(120),
        });
        let info = fsm.read(|s| job_info(&s.jobs[&JobId(1)], s.allocations.get(&JobId(1))));
        assert_eq!(info.state, "RUNNING");
        assert_eq!(info.planned_start, 1000);
        assert_eq!(info.planned_dur, 120);
        assert_eq!(info.nodes, vec!["n0".to_string()]);
    }

    #[test]
    fn job_info_falls_back_to_walltime_and_no_nodes_while_pending() {
        let fsm = fsm_with_node();
        let info = fsm.read(|s| job_info(&s.jobs[&JobId(2)], s.allocations.get(&JobId(2))));
        assert_eq!(info.state, "PENDING");
        assert_eq!(info.planned_dur, 600);
        assert!(info.nodes.is_empty());
    }
}
