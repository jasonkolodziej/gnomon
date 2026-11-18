//! `gnomon-web`: the server behind Gnomon's browser UI.
//!
//! - **Bridge.** A poller follows the controller's event tail (`ListEvents`,
//!   long-polled, so changes arrive as they happen) and refreshes a full
//!   snapshot (`ListJobs`, `ListNodes`, `ClusterStatus`, `PolicyStatus`)
//!   when something changed, at most a few times a second. Browsers get
//!   both over one Server-Sent Events stream, so a hundred open tabs are
//!   still one gRPC client. `GNOMON_ADDR` may list every controller of a
//!   replicated group; the poller moves on to the next one on any error.
//! - **Actions.** Submit and cancel go straight to the controller
//!   (`--read-only` turns them off).
//! - **Lab.** A policy comparison on a synthetic workload, run in-process
//!   with the production scheduling code (`gnomon-sim`).
//! - **Static files.** The built Rune app (`ui/`, see its README) is served
//!   from `--ui-dir`, with a built-in page explaining how to build it when
//!   it is missing.
//!
//! The JSON types are in `gnomon-web-model`, shared with the UI.

pub mod lab;

use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::{self, Stream, StreamExt};
use gnomon_api::pb::{self, scheduler_client::SchedulerClient};
use gnomon_web_model::{
    ActionReply, ClusterView, ControllerView, EventView, JobView, LabRequest, MemberView, NodeView,
    PluginView, PolicyView, SubmitForm,
};
use std::collections::VecDeque;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, Semaphore};
use tonic::transport::{Channel, Endpoint};

type Client = SchedulerClient<Channel>;

/// Events kept for browsers that connect later.
const RECENT_EVENTS: usize = 500;
/// Most events fetched per `ListEvents` call.
const EVENT_BATCH: u32 = 500;
/// Minimum gap between two snapshot refreshes while events stream in.
const REFRESH_EVERY: Duration = Duration::from_millis(300);
/// Refresh the snapshot at least this often (Raft terms and heartbeats
/// change without a job event).
const REFRESH_AT_LEAST: Duration = Duration::from_secs(2);
const RPC_TIMEOUT: Duration = Duration::from_secs(10);
/// Events a fresh feed starts with, before "now", so a new page has context.
const BOOTSTRAP_EVENTS: u64 = 200;
/// Lab runs computing at once; more get "busy" rather than queueing.
pub const LAB_CONCURRENCY: usize = 2;

// ------------------------------------------------------------- conversions

fn res(r: Option<pb::Resources>) -> pb::Resources {
    r.unwrap_or_default()
}

pub fn job_view(j: &pb::JobInfo) -> JobView {
    let r = res(j.request);
    JobView {
        id: j.id,
        name: j.name.clone(),
        user: j.user.clone(),
        partition: j.partition.clone(),
        state: j.state.clone(),
        cores: r.cores,
        mem_mb: r.mem_mb,
        gpus: r.gpus,
        num_nodes: j.num_nodes.max(1),
        walltime: j.walltime_secs,
        priority: j.priority,
        submit_time: j.submit_time,
        start: j.planned_start,
        span: j.planned_dur,
        nodes: j.nodes.clone(),
        container: j.container.clone(),
    }
}

pub fn node_view(n: &pb::NodeInfo) -> NodeView {
    let (c, a) = (res(n.capacity), res(n.allocated));
    NodeView {
        name: n.name.clone(),
        cores: c.cores,
        mem_mb: c.mem_mb,
        gpus: c.gpus,
        used_cores: a.cores,
        used_mem_mb: a.mem_mb,
        used_gpus: a.gpus,
        last_heartbeat: n.last_heartbeat,
        alive: n.alive,
        kill_pending: n.kill_pending.clone(),
    }
}

pub fn controller_view(c: &pb::ClusterStatusReply) -> ControllerView {
    ControllerView {
        replicated: c.replicated,
        node_id: c.node_id,
        role: c.role.clone(),
        leader: c.has_leader.then_some(c.leader_id),
        term: c.term,
        last_log_index: c.last_log_index,
        last_applied: c.last_applied,
        fsm_index: c.fsm_index,
        members: c
            .members
            .iter()
            .map(|m| MemberView {
                id: m.id,
                addr: m.addr.clone(),
                leader: m.leader,
                matched: m.has_matched.then_some(m.matched_index),
            })
            .collect(),
    }
}

pub fn policy_view(p: &pb::PolicyStatusReply) -> PolicyView {
    PolicyView {
        name: p.policy.clone(),
        fuel: p.fuel,
        memory_mb: p.memory_mb,
        plugins: p
            .plugins
            .iter()
            .map(|x| {
                let mut counters: Vec<(String, u64)> =
                    x.counters.iter().map(|(k, v)| (k.clone(), *v)).collect();
                counters.sort();
                PluginView {
                    world: x.world.clone(),
                    source: x.source.clone(),
                    mode: x.mode.clone(),
                    counters,
                }
            })
            .collect(),
    }
}

pub fn event_view(e: &pb::EventInfo) -> EventView {
    EventView {
        index: e.index,
        at: e.at,
        kind: e.kind.clone(),
        job: e.has_job.then_some(e.job_id),
        node: e.node.clone(),
        detail: e.detail.clone(),
        accepted: e.accepted,
    }
}

// --------------------------------------------------------------------- hub

/// What the poller publishes and the HTTP handlers serve.
#[derive(Clone, Debug)]
pub enum Push {
    State(Arc<String>),
    Events(Arc<String>),
    /// The controller's event history changed (a restarted controller
    /// without a state directory, or one with a shorter history): browsers
    /// drop the events they hold before the events that follow.
    Reset,
}

pub struct Hub {
    state: Mutex<Arc<ClusterView>>,
    recent: Mutex<VecDeque<EventView>>,
    tx: broadcast::Sender<Push>,
    pub writable: bool,
}

impl Hub {
    pub fn new(writable: bool) -> Arc<Hub> {
        let (tx, _) = broadcast::channel(256);
        Arc::new(Hub {
            state: Mutex::new(Arc::new(ClusterView {
                writable,
                error: Some("connecting…".into()),
                ..Default::default()
            })),
            recent: Mutex::new(VecDeque::new()),
            tx,
            writable,
        })
    }

    pub fn state(&self) -> Arc<ClusterView> {
        self.state.lock().unwrap().clone()
    }

    pub fn set_state(&self, mut view: ClusterView) {
        view.writable = self.writable;
        let json = Arc::new(serde_json::to_string(&view).expect("serializable"));
        *self.state.lock().unwrap() = Arc::new(view);
        let _ = self.tx.send(Push::State(json));
    }

    /// Cache and broadcast `events`, skipping any index already held, so an
    /// overlapping fetch after a reconnect never duplicates a row.
    pub fn push_events(&self, events: Vec<EventView>) {
        let fresh: Vec<EventView> = {
            let mut r = self.recent.lock().unwrap();
            let mut last = r.back().map(|e| e.index);
            let mut fresh = Vec::new();
            for e in events {
                if last.is_some_and(|l| e.index <= l) {
                    continue;
                }
                last = Some(e.index);
                if r.len() == RECENT_EVENTS {
                    r.pop_front();
                }
                r.push_back(e.clone());
                fresh.push(e);
            }
            fresh
        };
        if fresh.is_empty() {
            return;
        }
        let json = Arc::new(serde_json::to_string(&fresh).expect("serializable"));
        let _ = self.tx.send(Push::Events(json));
    }

    /// Forget the cached events: the history they came from is gone.
    pub fn reset_events(&self) {
        self.recent.lock().unwrap().clear();
        let _ = self.tx.send(Push::Reset);
    }

    pub fn recent(&self) -> Vec<EventView> {
        self.recent.lock().unwrap().iter().cloned().collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Push> {
        self.tx.subscribe()
    }
}

// ------------------------------------------------------------------ poller

async fn connect(addr: &str) -> Result<Client, String> {
    let ch = Endpoint::from_shared(addr.to_string())
        .map_err(|e| e.to_string())?
        .connect_timeout(RPC_TIMEOUT)
        .timeout(Duration::from_secs(40)) // above ListEvents' longest wait
        .connect()
        .await
        .map_err(|e| e.to_string())?;
    Ok(SchedulerClient::new(ch))
}

/// One full snapshot from `c`.
pub async fn snapshot(c: &mut Client, controller: &str) -> Result<ClusterView, String> {
    let err = |s: tonic::Status| s.message().to_string();
    let jobs = c
        .list_jobs(pb::ListJobsRequest {
            user: String::new(),
            state: String::new(),
        })
        .await
        .map_err(err)?
        .into_inner();
    let nodes = c
        .list_nodes(pb::ListNodesRequest {})
        .await
        .map_err(err)?
        .into_inner();
    let cluster = c
        .cluster_status(pb::ClusterStatusRequest {})
        .await
        .map_err(err)?
        .into_inner();
    let policy = c
        .policy_status(pb::PolicyStatusRequest {})
        .await
        .map_err(err)?
        .into_inner();
    Ok(ClusterView {
        controller: controller.to_string(),
        connected: true,
        error: None,
        now: gnomon_core::now_secs(),
        state_index: jobs.state_index,
        writable: false,
        cluster: controller_view(&cluster),
        policy: policy_view(&policy),
        nodes: nodes.nodes.iter().map(node_view).collect(),
        jobs: jobs.jobs.iter().map(job_view).collect(),
    })
}

/// Follow the controllers in `addrs` forever, publishing to `hub`.
pub async fn run_poller(addrs: Vec<String>, hub: Arc<Hub>) {
    let mut idx = 0usize;
    // The last event index delivered, kept across reconnects and failovers.
    let mut cursor: Option<u64> = None;
    loop {
        let addr = addrs[idx % addrs.len()].clone();
        match follow(&addr, &hub, &mut cursor).await {
            Ok(()) => {}
            Err(e) => {
                let mut v = (*hub.state()).clone();
                v.connected = false;
                v.error = Some(format!("{addr}: {e}"));
                v.controller = addr.clone();
                hub.set_state(v);
                idx += 1;
                tokio::time::sleep(Duration::from_millis(if addrs.len() > 1 {
                    300
                } else {
                    1000
                }))
                .await;
            }
        }
    }
}

/// Where the event feed resumes on a controller whose history ends at
/// `last_index`: from `cursor` if that history still covers it, else
/// [`BOOTSTRAP_EVENTS`] before its end (the first connection, or a history
/// that went backwards, which also resets the hub).
fn resume_after(cursor: Option<u64>, last_index: u64, hub: &Hub) -> u64 {
    match cursor {
        Some(c) if c <= last_index => c,
        Some(_) => {
            hub.reset_events();
            last_index.saturating_sub(BOOTSTRAP_EVENTS)
        }
        None => last_index.saturating_sub(BOOTSTRAP_EVENTS),
    }
}

/// Follow one controller until an error.
async fn follow(addr: &str, hub: &Hub, cursor: &mut Option<u64>) -> Result<(), String> {
    let mut c = connect(addr).await?;
    let first = snapshot(&mut c, addr).await?;
    let mut after = resume_after(*cursor, first.state_index, hub);
    *cursor = Some(after);
    hub.set_state(first);
    let mut refreshed = tokio::time::Instant::now();
    loop {
        let reply = c
            .list_events(pb::ListEventsRequest {
                after_index: after,
                limit: EVENT_BATCH,
                wait_ms: 1000,
            })
            .await
            .map_err(|s| s.message().to_string())?
            .into_inner();
        let changed = !reply.events.is_empty();
        if let Some(last) = reply.events.last() {
            after = last.index;
        } else if reply.last_index < after {
            // The controller restarted with a shorter history.
            after = resume_after(Some(after), reply.last_index, hub);
        }
        *cursor = Some(after);
        hub.push_events(reply.events.iter().map(event_view).collect());
        let due = refreshed.elapsed() >= REFRESH_AT_LEAST
            || (changed && refreshed.elapsed() >= REFRESH_EVERY);
        if due {
            let v = snapshot(&mut c, addr).await?;
            hub.set_state(v);
            refreshed = tokio::time::Instant::now();
        }
    }
}

// -------------------------------------------------------------------- http

#[derive(Clone)]
pub struct App {
    pub hub: Arc<Hub>,
    pub addrs: Vec<String>,
    pub ui_dir: PathBuf,
    /// Permits for lab runs ([`LAB_CONCURRENCY`]).
    pub lab_slots: Arc<Semaphore>,
}

impl App {
    pub fn new(hub: Arc<Hub>, addrs: Vec<String>, ui_dir: PathBuf) -> App {
        App {
            hub,
            addrs,
            ui_dir,
            lab_slots: Arc::new(Semaphore::new(LAB_CONCURRENCY)),
        }
    }
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/api/state", get(api_state))
        .route("/api/events", get(api_events))
        .route("/api/stream", get(api_stream))
        .route("/api/jobs", post(api_submit))
        .route("/api/jobs/:id/cancel", post(api_cancel))
        .route("/api/lab", post(api_lab))
        .route("/", get(static_index))
        .route("/*path", get(static_file))
        .layer(middleware::from_fn(same_origin_posts))
        .with_state(app)
}

/// Refuse POSTs another site could make from a visitor's browser: each must
/// carry a JSON body (which a cross-origin page can only send after a CORS
/// preflight, and none is answered) and, when the browser names an `Origin`,
/// come from this server's own host.
pub fn post_allowed(headers: &HeaderMap) -> bool {
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.trim_start()
                .to_ascii_lowercase()
                .starts_with("application/json")
        });
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let same_origin = match headers.get(header::ORIGIN).map(|v| v.to_str()) {
        None => true, // not a browser, or a same-origin request that omits it
        Some(Ok(origin)) => {
            let authority = origin.split_once("://").map_or(origin, |(_, a)| a);
            host.is_some_and(|h| h.eq_ignore_ascii_case(authority))
        }
        Some(Err(_)) => false,
    };
    json && same_origin
}

async fn same_origin_posts(req: Request, next: Next) -> Response {
    if req.method() == Method::POST && !post_allowed(req.headers()) {
        return refused("cross-origin or non-JSON POST refused").into_response();
    }
    next.run(req).await
}

async fn api_state(State(app): State<App>) -> Json<ClusterView> {
    Json((*app.hub.state()).clone())
}

async fn api_events(State(app): State<App>) -> Json<Vec<EventView>> {
    Json(app.hub.recent())
}

/// SSE: the current state and the retained events (`history`) at once, then
/// every update (`state`, `events`, `reset`).
async fn api_stream(
    State(app): State<App>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    // Subscribe before the snapshot, so nothing published in between is lost
    // (an event in both is dropped by index in the browser).
    let rx = app.hub.subscribe();
    let state = serde_json::to_string(&*app.hub.state()).expect("serializable");
    let recent = serde_json::to_string(&app.hub.recent()).expect("serializable");
    let initial = vec![
        Ok(SseEvent::default().event("state").data(state)),
        // The whole retained history, which replaces what the browser holds:
        // after a reconnect it may be from a different (restarted) controller.
        Ok(SseEvent::default().event("history").data(recent)),
    ];
    let live = stream::unfold(rx, |mut rx| async move {
        let event = match rx.recv().await {
            Ok(Push::State(s)) => SseEvent::default().event("state").data(&*s),
            Ok(Push::Events(e)) => SseEvent::default().event("events").data(&*e),
            Ok(Push::Reset) => SseEvent::default().event("reset").data("{}"),
            // A slow browser missed pushes, events among them. End the stream:
            // EventSource reconnects and gets the state and retained events again.
            Err(broadcast::error::RecvError::Lagged(_) | broadcast::error::RecvError::Closed) => {
                return None
            }
        };
        Some((Ok(event), rx))
    });
    Sse::new(stream::iter(initial).chain(live)).keep_alive(KeepAlive::default())
}

fn refused(msg: &str) -> (StatusCode, Json<ActionReply>) {
    (
        StatusCode::FORBIDDEN,
        Json(ActionReply {
            ok: false,
            job: None,
            message: msg.into(),
        }),
    )
}

/// Any reachable controller (followers forward writes to the leader).
async fn any_controller(addrs: &[String]) -> Result<Client, String> {
    let mut errs = Vec::new();
    for a in addrs {
        match connect(a).await {
            Ok(c) => return Ok(c),
            Err(e) => errs.push(format!("{a}: {e}")),
        }
    }
    Err(errs.join("; "))
}

async fn api_submit(
    State(app): State<App>,
    Json(f): Json<SubmitForm>,
) -> (StatusCode, Json<ActionReply>) {
    if !app.hub.writable {
        return refused("this gnomon-web is read-only (--read-only)");
    }
    if f.cores == 0 || f.walltime_secs == 0 || f.command.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ActionReply {
                ok: false,
                job: None,
                message: "cores, walltime and command are required".into(),
            }),
        );
    }
    let req = pb::SubmitJobRequest {
        name: if f.name.is_empty() {
            "web".into()
        } else {
            f.name
        },
        user: f.user,
        partition: f.partition,
        request: Some(pb::Resources {
            cores: f.cores,
            mem_mb: f.mem_mb,
            gpus: f.gpus,
        }),
        walltime_secs: f.walltime_secs,
        priority: f.priority,
        command: f.command,
        nodes: f.nodes.max(1),
        container: f.container,
    };
    let reply = match any_controller(&app.addrs).await {
        Ok(mut c) => c.submit_job(req).await.map(|r| r.into_inner().job_id),
        Err(e) => Err(tonic::Status::unavailable(e)),
    };
    match reply {
        Ok(id) => (
            StatusCode::OK,
            Json(ActionReply {
                ok: true,
                job: Some(id),
                message: format!("submitted job {id}"),
            }),
        ),
        Err(s) => (
            StatusCode::BAD_GATEWAY,
            Json(ActionReply {
                ok: false,
                job: None,
                message: s.message().to_string(),
            }),
        ),
    }
}

/// The body is ignored but must be JSON (see [`post_allowed`]).
async fn api_cancel(
    State(app): State<App>,
    Path(id): Path<u64>,
    Json(_): Json<serde_json::Value>,
) -> (StatusCode, Json<ActionReply>) {
    if !app.hub.writable {
        return refused("this gnomon-web is read-only (--read-only)");
    }
    let reply = match any_controller(&app.addrs).await {
        Ok(mut c) => c
            .cancel_job(pb::CancelJobRequest {
                job_id: id,
                user: String::new(),
            })
            .await
            .map(|r| r.into_inner().ok),
        Err(e) => Err(tonic::Status::unavailable(e)),
    };
    match reply {
        Ok(ok) => (
            StatusCode::OK,
            Json(ActionReply {
                ok,
                job: Some(id),
                message: if ok {
                    format!("cancelling job {id}")
                } else {
                    format!("job {id} is unknown or already finished")
                },
            }),
        ),
        Err(s) => (
            StatusCode::BAD_GATEWAY,
            Json(ActionReply {
                ok: false,
                job: Some(id),
                message: s.message().to_string(),
            }),
        ),
    }
}

async fn api_lab(State(app): State<App>, Json(req): Json<LabRequest>) -> Response {
    if let Err(e) = lab::validate(&req) {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }
    let Ok(permit) = app.lab_slots.clone().try_acquire_owned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("the lab is busy ({LAB_CONCURRENCY} runs at once): try again shortly"),
        )
            .into_response();
    };
    // The permit moves into the blocking task, so it is held until the
    // simulations finish even if the browser goes away first.
    let work = move || {
        let report = lab::run(&req);
        drop(permit);
        report
    };
    match tokio::task::spawn_blocking(work).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ------------------------------------------------------------ static files

/// Content type by extension; `.wasm` must be `application/wasm`.
pub fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// `rel` with every component a plain name (no `..`, no absolute paths).
pub fn safe_relative(rel: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        if part == "." || part == ".." || part.contains('\\') {
            return None;
        }
        out.push(part);
    }
    Some(out)
}

const FALLBACK_PAGE: &str = include_str!("fallback.html");

async fn static_index(State(app): State<App>) -> Response {
    serve(&app, "index.html").await
}

async fn static_file(State(app): State<App>, Path(path): Path<String>) -> Response {
    serve(&app, &path).await
}

async fn serve(app: &App, rel: &str) -> Response {
    let Some(rel_path) = safe_relative(rel) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let path = app.ui_dir.join(&rel_path);
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, content_type(rel)),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) if rel == "index.html" => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            FALLBACK_PAGE.replace("{UI_DIR}", &app.ui_dir.display().to_string()),
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_paths_cannot_escape_the_ui_dir() {
        assert_eq!(safe_relative("app.js"), Some(PathBuf::from("app.js")));
        assert_eq!(
            safe_relative("snippets/x/a.js"),
            Some(PathBuf::from("snippets/x/a.js"))
        );
        for bad in ["../etc/passwd", "a/../../b", "./x", "a\\b"] {
            assert_eq!(safe_relative(bad), None, "{bad}");
        }
    }

    #[test]
    fn wasm_is_served_as_application_wasm() {
        assert_eq!(content_type("app_bg.wasm"), "application/wasm");
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("Inter-Regular.ttf"), "font/ttf");
    }

    #[test]
    fn conversions_carry_every_field_the_ui_reads() {
        let c = controller_view(&pb::ClusterStatusReply {
            replicated: true,
            node_id: 2,
            role: "follower".into(),
            has_leader: true,
            leader_id: 1,
            term: 7,
            last_log_index: 40,
            last_applied: 39,
            fsm_index: 30,
            members: vec![pb::ClusterMember {
                id: 3,
                addr: "h:1".into(),
                leader: false,
                has_matched: false,
                matched_index: 0,
            }],
        });
        assert_eq!(c.leader, Some(1));
        assert_eq!(c.members[0].matched, None);
        let e = event_view(&pb::EventInfo {
            index: 5,
            at: 9,
            kind: "JobStarted".into(),
            has_job: true,
            job_id: 4,
            node: String::new(),
            detail: "d".into(),
            accepted: true,
        });
        assert_eq!(e.job, Some(4));
        let p = policy_view(&pb::PolicyStatusReply {
            policy: "backfill/walltime".into(),
            plugins: vec![pb::PluginStatus {
                world: "admission".into(),
                source: "a.wasm".into(),
                mode: "fail open".into(),
                counters: [("rejected".to_string(), 1), ("calls".to_string(), 3)].into(),
            }],
            fuel: 1,
            memory_mb: 2,
        });
        assert_eq!(
            p.plugins[0].counters,
            [("calls".to_string(), 3), ("rejected".to_string(), 1)],
            "sorted by name"
        );
    }

    fn ev(i: u64) -> EventView {
        EventView {
            index: i,
            ..Default::default()
        }
    }

    #[test]
    fn overlapping_event_batches_are_cached_and_sent_once() {
        let hub = Hub::new(true);
        let mut rx = hub.subscribe();
        hub.push_events((1..=5).map(ev).collect());
        hub.push_events((3..=8).map(ev).collect());
        let idx: Vec<u64> = hub.recent().iter().map(|e| e.index).collect();
        assert_eq!(idx, (1..=8).collect::<Vec<_>>());
        assert!(matches!(rx.try_recv(), Ok(Push::Events(_))));
        match rx.try_recv() {
            Ok(Push::Events(json)) => {
                let sent: Vec<EventView> = serde_json::from_str(&json).unwrap();
                assert_eq!(sent.iter().map(|e| e.index).collect::<Vec<_>>(), [6, 7, 8]);
            }
            other => panic!("expected the new events only, got {other:?}"),
        }
        hub.push_events((1..=8).map(ev).collect());
        assert!(rx.try_recv().is_err(), "nothing new, nothing sent");
    }

    #[test]
    fn the_event_cursor_survives_reconnects_and_resets_on_a_shorter_history() {
        let hub = Hub::new(true);
        let mut rx = hub.subscribe();
        assert_eq!(
            resume_after(None, 1000, &hub),
            800,
            "first connection: context"
        );
        assert_eq!(
            resume_after(Some(1000), 1500, &hub),
            1000,
            "nothing skipped"
        );
        assert!(rx.try_recv().is_err());
        hub.push_events(vec![ev(999)]);
        let _ = rx.try_recv();
        assert_eq!(
            resume_after(Some(1000), 40, &hub),
            0,
            "restarted controller"
        );
        assert!(matches!(rx.try_recv(), Ok(Push::Reset)));
        assert!(hub.recent().is_empty());
        hub.push_events(vec![ev(1)]);
        assert_eq!(hub.recent().len(), 1, "low indices accepted after a reset");
    }

    #[test]
    fn only_same_origin_json_posts_are_allowed() {
        let h = |pairs: &[(header::HeaderName, &str)]| {
            let mut m = HeaderMap::new();
            for (k, v) in pairs {
                m.insert(k.clone(), v.parse().unwrap());
            }
            m
        };
        let json = (header::CONTENT_TYPE, "application/json");
        let host = (header::HOST, "127.0.0.1:8080");
        assert!(post_allowed(&h(&[json.clone(), host.clone()])));
        assert!(post_allowed(&h(&[
            json.clone(),
            host.clone(),
            (header::ORIGIN, "http://127.0.0.1:8080"),
        ])));
        assert!(!post_allowed(&h(&[
            json.clone(),
            host.clone(),
            (header::ORIGIN, "https://evil.example"),
        ])));
        assert!(!post_allowed(&h(&[
            (header::CONTENT_TYPE, "application/x-www-form-urlencoded"),
            host.clone(),
        ])));
        assert!(!post_allowed(&h(&[host])), "no body type");
    }

    #[test]
    fn hub_keeps_recent_events_and_marks_writability() {
        let hub = Hub::new(false);
        let mut rx = hub.subscribe();
        hub.set_state(ClusterView::default());
        assert!(!hub.state().writable);
        assert!(matches!(rx.try_recv(), Ok(Push::State(_))));
        hub.push_events(
            (0..RECENT_EVENTS as u64 + 5)
                .map(|i| EventView {
                    index: i,
                    ..Default::default()
                })
                .collect(),
        );
        let recent = hub.recent();
        assert_eq!(recent.len(), RECENT_EVENTS);
        assert_eq!(recent[0].index, 5, "oldest dropped");
        hub.push_events(vec![]);
        assert!(matches!(rx.try_recv(), Ok(Push::Events(_))));
        assert!(rx.try_recv().is_err(), "no push for an empty batch");
    }
}
