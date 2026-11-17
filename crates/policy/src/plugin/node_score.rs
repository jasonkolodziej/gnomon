//! `node-score` world (ADR 0004): choose *which* nodes a job runs on, after
//! the scheduler has chosen *when*.
//!
//! [`ScoredPolicy`] wraps a [`Policy`]. The inner policy places the job as
//! usual (start time, span, nodes). If more nodes than the job needs could
//! hold it for that whole span, the plugin scores all of them in one call
//! and the job goes to the highest-scoring ones. The start time never
//! changes, so the plugin cannot delay a job or break a reservation, and the
//! plan event records the chosen nodes, so replicas never run it.

use super::bindings::node_score::NodeScorePre;
use super::bindings::types::Node as WitNode;
use super::{plugin_loaders, wit_job, Host, PluginError, Sandbox};
use gnomon_core::Job;
use gnomon_graph::{ResourceGraph, VertexId};
use gnomon_sched::{Decision, Policy};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A candidate node as the plugin sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub capacity: gnomon_core::Resources,
    /// Free for the placement's whole span (capacity minus the peak use of
    /// other jobs during it).
    pub free: gnomon_core::Resources,
}

/// A loaded node-score plugin.
pub struct NodeScorePlugin(Sandbox<NodeScorePre<Host>>);

plugin_loaders!(NodeScorePlugin, NodeScorePre<super::Host>);

impl NodeScorePlugin {
    /// One score per candidate, in order.
    pub fn score(
        &self,
        job: &Job,
        candidates: &[Candidate],
        now: u64,
    ) -> Result<Vec<i64>, PluginError> {
        let nodes: Vec<WitNode> = candidates
            .iter()
            .map(|c| WitNode {
                name: c.name.clone(),
                cores: c.capacity.cores,
                mem_mb: c.capacity.mem_mb,
                gpus: c.capacity.gpus,
                free_cores: c.free.cores,
                free_mem_mb: c.free.mem_mb,
                free_gpus: c.free.gpus,
            })
            .collect();
        let scores = self.0.call(|store, pre| {
            pre.instantiate(&mut *store)?
                .call_score(&mut *store, &wit_job(job), &nodes, now)
        })?;
        if scores.len() != candidates.len() {
            return Err(PluginError::Invalid(format!(
                "{} scores for {} candidates",
                scores.len(),
                candidates.len()
            )));
        }
        Ok(scores)
    }
}

/// A [`Policy`] whose node choice is delegated to a node-score plugin.
/// Everything else (timing, spans, learning) is the inner policy's. A
/// failed call keeps the inner policy's nodes.
pub struct ScoredPolicy {
    pub inner: Arc<dyn Policy>,
    plugin: NodeScorePlugin,
    /// Most candidates offered per call besides the inner policy's own
    /// choice; `None` offers every feasible node (ADR 0004).
    pub max_candidates: Option<usize>,
    /// Plugin calls (placements with a choice to make).
    pub calls: AtomicU64,
    pub failures: AtomicU64,
}

/// Default for [`ScoredPolicy::max_candidates`]: keeps a call in the tens
/// of microseconds on large clusters (see `plugin_bench`).
pub const DEFAULT_MAX_CANDIDATES: usize = 64;

impl ScoredPolicy {
    pub fn new(inner: Arc<dyn Policy>, plugin: NodeScorePlugin) -> Self {
        ScoredPolicy {
            inner,
            plugin,
            max_candidates: Some(DEFAULT_MAX_CANDIDATES),
            calls: AtomicU64::new(0),
            failures: AtomicU64::new(0),
        }
    }

    /// Offer at most `max` candidates besides the default choice per call
    /// (`None`: all of them).
    pub fn with_max_candidates(mut self, max: Option<usize>) -> Self {
        self.max_candidates = max;
        self
    }

    /// The inner policy's `chosen` nodes (feasible by construction, possibly
    /// only for its shorter planned span) plus up to `max_candidates` other
    /// nodes that can hold `job` for `[start, start + span)`, found by a
    /// scan starting at a job-seeded offset (like `match_gang`) so that
    /// capped calls do not all see the lowest-numbered nodes. Vertex order.
    fn candidates(
        &self,
        graph: &ResourceGraph,
        job: &Job,
        start: u64,
        span: u64,
        chosen: &[VertexId],
    ) -> Vec<(VertexId, Candidate)> {
        let all = graph.schedulable_nodes();
        let off = (job.id.0 as usize) % all.len().max(1);
        let limit = self.max_candidates.unwrap_or(usize::MAX);
        let mut ids: Vec<VertexId> = chosen.to_vec();
        let others = (0..all.len())
            .map(|k| all[(off + k) % all.len()])
            .filter(|id| !chosen.contains(id))
            .filter(|id| graph.vertex(*id).earliest_fit(&job.request, start, span) == Some(start))
            .take(limit);
        ids.extend(others);
        ids.sort_unstable();
        ids.into_iter()
            .map(|id| {
                let v = graph.vertex(id);
                let used = v.planner.peak_usage(start, start + span);
                let c = Candidate {
                    name: v.name.clone(),
                    capacity: v.capacity,
                    free: v.capacity.saturating_sub(&used),
                };
                (id, c)
            })
            .collect()
    }

    /// Re-pick `chosen` among the candidates by score, or `None` to keep it.
    fn rescore(
        &self,
        graph: &ResourceGraph,
        job: &Job,
        now: u64,
        start: u64,
        chosen: &[VertexId],
    ) -> Option<Vec<VertexId>> {
        let schedulable = graph.schedulable_nodes();
        if !chosen.iter().all(|id| schedulable.contains(id)) {
            return None; // not node leaves: nothing this hook understands
        }
        // The walltime span is the conservative one: a node that holds the
        // job for its walltime holds it for any shorter planned span.
        let span = job.walltime.secs();
        let cands = self.candidates(graph, job, start, span, chosen);
        if cands.len() <= chosen.len() {
            return None; // no choice to make: skip the call
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        let list: Vec<Candidate> = cands.iter().map(|(_, c)| c.clone()).collect();
        let scores = match self.plugin.score(job, &list, now) {
            Ok(s) => s,
            Err(e) => {
                let n = self.failures.fetch_add(1, Ordering::Relaxed) + 1;
                eprintln!(
                    "gnomon: node-score plugin failed for job {} ({n} failures); \
                     keeping the default placement: {e}",
                    job.id.0
                );
                return None;
            }
        };
        // Highest score first; ties keep the inner policy's choice, then
        // vertex order, so a constant scorer changes nothing.
        let mut ranked: Vec<(i64, bool, VertexId)> = cands
            .iter()
            .zip(&scores)
            .map(|((id, _), &s)| (s, !chosen.contains(id), *id))
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let mut picked: Vec<VertexId> = ranked.iter().take(chosen.len()).map(|r| r.2).collect();
        picked.sort_unstable(); // ascending vertex order defines gang ranks
        Some(picked)
    }
}

impl Policy for ScoredPolicy {
    fn name(&self) -> String {
        format!("{}+node-score", self.inner.name())
    }
    fn planned_secs(&self, job: &Job) -> u64 {
        self.inner.planned_secs(job)
    }
    fn observe(&self, job: &Job, actual: u64) {
        self.inner.observe(job, actual)
    }
    fn begin_pass(&self, queue: &[&Job], graph: &ResourceGraph, now: u64) {
        self.inner.begin_pass(queue, graph, now)
    }
    fn extend_secs(&self, job: &Job, elapsed: u64) -> u64 {
        self.inner.extend_secs(job, elapsed)
    }

    fn place(&self, graph: &ResourceGraph, job: &Job, now: u64, allow_reserve: bool) -> Decision {
        let d = self.inner.place(graph, job, now, allow_reserve);
        match d {
            Decision::Start {
                vertices,
                start,
                dur,
            } => Decision::Start {
                vertices: self
                    .rescore(graph, job, now, start, &vertices)
                    .unwrap_or(vertices),
                start,
                dur,
            },
            Decision::Reserve {
                vertices,
                start,
                dur,
            } => Decision::Reserve {
                vertices: self
                    .rescore(graph, job, now, start, &vertices)
                    .unwrap_or(vertices),
                start,
                dur,
            },
            Decision::Defer => Decision::Defer,
        }
    }
}
