//! Fluxion-style resource graph (report section 4.2).
//!
//! Resources are typed vertices in a tree/DAG (cluster -> rack -> node ->
//! socket -> core/gpu). Each *schedulable* vertex carries a [`Planner`], a
//! time-axis reservation structure that answers "when can X cores run for D
//! seconds here?" in O(log n)-ish time instead of Slurm's monolithic,
//! lock-held reservation table.
//!
//! The graph is an arena (`Vec<Vertex>`) — indices, not pointers — which is
//! the Rust-idiomatic way to keep the structure cloneable for the immutable
//! snapshots that optimistic scheduling workers consume.

use gnomon_core::Resources;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type VertexId = u32;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum VertexKind {
    Cluster,
    Rack,
    Switch,
    Node,
    Socket,
    Core,
    Gpu,
}

/// A half-open reserved time span `[start, end)` consuming `res` on a vertex.
/// `job` ties the span to its job so completion can release it early and
/// overruns can extend it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Span {
    start: u64,
    end: u64,
    res: Resources,
    #[serde(default)]
    job: Option<u64>,
}

pub use gnomon_core::CapacityProfile;

/// Capacity as seen by a planner query: base capacity plus optional profile.
#[derive(Clone, Copy, Debug)]
pub struct CapView<'a> {
    pub base: Resources,
    pub profile: Option<&'a CapacityProfile>,
}

impl<'a> CapView<'a> {
    pub fn fixed(base: Resources) -> Self {
        CapView {
            base,
            profile: None,
        }
    }
    fn at(&self, t: u64) -> Resources {
        match self.profile {
            Some(p) => p.at(&self.base, t),
            None => self.base,
        }
    }
    /// Largest capacity the profile can ever offer (feasibility bound).
    fn max(&self) -> Resources {
        match self.profile {
            None => self.base,
            Some(p) => p.steps.iter().fold(
                if p.period.is_some() {
                    Resources::default()
                } else {
                    self.base
                },
                |m, s| Resources {
                    cores: m.cores.max(s.1.cores),
                    mem_mb: m.mem_mb.max(s.1.mem_mb),
                    gpus: m.gpus.max(s.1.gpus),
                },
            ),
        }
    }
    fn period(&self) -> u64 {
        self.profile.and_then(|p| p.period).unwrap_or(0)
    }
}

/// Per-vertex temporal planner.
///
/// Queries build a usage step-function over the relevant breakpoints (span
/// edges + capacity-profile steps) and sweep it once, so `earliest_fit` is
/// O(S log S) in the number of spans S (the original skeleton re-summed
/// every span per checkpoint per candidate start: O(S^3)).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Planner {
    spans: Vec<Span>,
}

fn sub(a: Resources, b: Resources) -> Resources {
    a.saturating_sub(&b)
}

impl Planner {
    /// Earliest `start >= from` such that `req` fits against `cap`
    /// continuously for `dur` seconds.
    pub fn earliest_fit(&self, cap: CapView, req: &Resources, from: u64, dur: u64) -> Option<u64> {
        if dur == 0 || !req.fits_in(&cap.max()) {
            return None;
        }
        // Usage at `from`, and deltas after it.
        let mut usage = Resources::default();
        let mut deltas: Vec<(u64, bool, Resources)> = Vec::with_capacity(self.spans.len() * 2);
        let mut last_end = from;
        for s in &self.spans {
            if s.end <= from {
                continue;
            }
            last_end = last_end.max(s.end);
            if s.start <= from {
                usage = usage.add(&s.res);
            } else {
                deltas.push((s.start, true, s.res));
            }
            deltas.push((s.end, false, s.res));
        }
        // Breakpoints: span edges + capacity steps up to a horizon after which
        // usage is zero and the profile has repeated at least once.
        let horizon = last_end + cap.period() + dur + 1;
        let mut points: Vec<u64> = deltas.iter().map(|d| d.0).collect();
        if let Some(p) = cap.profile {
            p.breakpoints(from, horizon, &mut points);
        }
        points.push(from);
        points.sort_unstable();
        points.dedup();
        deltas.sort_unstable_by_key(|d| (d.0, d.1)); // ends (false) before starts at equal t

        let mut di = 0;
        let mut run_start: Option<u64> = None;
        for (i, &p) in points.iter().enumerate() {
            while di < deltas.len() && deltas[di].0 <= p {
                let (_, is_start, r) = deltas[di];
                usage = if is_start {
                    usage.add(&r)
                } else {
                    sub(usage, r)
                };
                di += 1;
            }
            let ok = usage.add(req).fits_in(&cap.at(p));
            if !ok {
                run_start = None;
                continue;
            }
            let rs = *run_start.get_or_insert(p);
            // For periodic profiles every start in [last_end, last_end+period]
            // is fully checked before the horizon; beyond it nothing new can
            // fit, so the tail segment is finite.
            let tail = if cap.period() > 0 { horizon } else { u64::MAX };
            let seg_end = points.get(i + 1).copied().unwrap_or(tail);
            if seg_end.saturating_sub(rs) >= dur {
                return Some(rs);
            }
        }
        None
    }

    /// Resources in use at instant `t`.
    pub fn usage_at(&self, t: u64) -> Resources {
        self.spans
            .iter()
            .filter(|s| s.start <= t && t < s.end)
            .fold(Resources::default(), |u, s| u.add(&s.res))
    }

    /// Reserve `[start, start+dur)` consuming `req`. Returns false on conflict
    /// (used by the plan applier for commit-time revalidation — report 5.2).
    pub fn reserve(&mut self, cap: CapView, req: &Resources, start: u64, dur: u64) -> bool {
        self.reserve_job(cap, req, start, dur, None)
    }

    pub fn reserve_job(
        &mut self,
        cap: CapView,
        req: &Resources,
        start: u64,
        dur: u64,
        job: Option<u64>,
    ) -> bool {
        if self.earliest_fit(cap, req, start, dur) != Some(start) {
            return false;
        }
        self.spans.push(Span {
            start,
            end: start + dur,
            res: *req,
            job,
        });
        true
    }

    /// Unchecked insert — only for plans proven valid against this exact
    /// planner state (version fast path) or for physical facts (a running job
    /// that overran its prediction is on the hardware whether it fits or not).
    fn push_unchecked(&mut self, req: &Resources, start: u64, dur: u64, job: Option<u64>) {
        self.spans.push(Span {
            start,
            end: start + dur,
            res: *req,
            job,
        });
    }

    /// Truncate the job's spans at `at` (early completion / cancellation).
    pub fn release_job(&mut self, job: u64, at: u64) -> bool {
        let before = self.spans.len();
        let mut changed = false;
        for s in self.spans.iter_mut().filter(|s| s.job == Some(job)) {
            if s.end > at {
                s.end = at.max(s.start);
                changed = true;
            }
        }
        self.spans.retain(|s| s.end > s.start);
        changed || self.spans.len() != before
    }

    /// Extend the job's spans to `new_end` (overrun of a predicted runtime).
    pub fn extend_job(&mut self, job: u64, new_end: u64) -> bool {
        let mut changed = false;
        for s in self.spans.iter_mut().filter(|s| s.job == Some(job)) {
            if s.end < new_end {
                s.end = new_end;
                changed = true;
            }
        }
        changed
    }

    /// Remove reservations that start after `now`; returns their job ids.
    pub fn drop_future(&mut self, now: u64) -> Vec<u64> {
        let mut jobs = Vec::new();
        self.spans.retain(|s| {
            if s.start > now {
                if let Some(j) = s.job {
                    jobs.push(j);
                }
                false
            } else {
                true
            }
        });
        jobs
    }

    pub fn job_span(&self, job: u64) -> Option<(u64, u64)> {
        self.spans
            .iter()
            .find(|s| s.job == Some(job))
            .map(|s| (s.start, s.end))
    }

    /// End of the last span (0 when empty).
    pub fn last_end(&self) -> u64 {
        self.spans.iter().map(|s| s.end).max().unwrap_or(0)
    }

    pub fn span_count(&self) -> usize {
        self.spans.len()
    }

    pub fn release_before(&mut self, now: u64) {
        self.spans.retain(|s| s.end > now);
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Least common multiple, saturating at `u64::MAX`.
fn lcm_sat(a: u64, b: u64) -> u64 {
    (a / gcd(a, b)).saturating_mul(b)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vertex {
    pub kind: VertexKind,
    pub name: String,
    /// Total capacity at this vertex (leaf vertices are schedulable).
    pub capacity: Resources,
    pub planner: Planner,
    pub parent: Option<VertexId>,
    pub children: Vec<VertexId>,
    /// Bumped on every planner mutation: the per-vertex optimistic
    /// concurrency token (a plan conflicts only if a vertex it uses moved).
    #[serde(default)]
    pub version: u64,
    /// Optional time-varying capacity (power cap / carbon window).
    #[serde(default)]
    pub profile: Option<CapacityProfile>,
}

impl Vertex {
    pub fn cap(&self) -> CapView<'_> {
        CapView {
            base: self.capacity,
            profile: self.profile.as_ref(),
        }
    }
    pub fn earliest_fit(&self, req: &Resources, from: u64, dur: u64) -> Option<u64> {
        self.planner.earliest_fit(self.cap(), req, from, dur)
    }
}

/// Arena-allocated resource graph.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResourceGraph {
    pub vertices: Vec<Vertex>,
    roots: Vec<VertexId>,
    by_name: BTreeMap<String, VertexId>,
}

impl ResourceGraph {
    pub fn add_vertex(
        &mut self,
        kind: VertexKind,
        name: &str,
        capacity: Resources,
        parent: Option<VertexId>,
    ) -> VertexId {
        let id = self.vertices.len() as VertexId;
        self.vertices.push(Vertex {
            kind,
            name: name.into(),
            capacity,
            planner: Planner::default(),
            parent,
            children: vec![],
            version: 0,
            profile: None,
        });
        match parent {
            Some(p) => self.vertices[p as usize].children.push(id),
            None => self.roots.push(id),
        }
        self.by_name.insert(name.into(), id);
        id
    }

    pub fn vertex(&self, id: VertexId) -> &Vertex {
        &self.vertices[id as usize]
    }
    pub fn vertex_mut(&mut self, id: VertexId) -> &mut Vertex {
        &mut self.vertices[id as usize]
    }
    pub fn by_name(&self, name: &str) -> Option<VertexId> {
        self.by_name.get(name).copied()
    }

    pub fn set_profile(&mut self, id: VertexId, profile: Option<CapacityProfile>) {
        let v = self.vertex_mut(id);
        v.profile = profile;
        v.version += 1;
    }

    /// Schedulable leaves: nodes (and later GPU vertices). Matching walks
    /// leaves and checks both capacity *and* the temporal planner.
    pub fn leaves(&self) -> impl Iterator<Item = VertexId> + '_ {
        self.vertices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.kind == VertexKind::Node || v.kind == VertexKind::Gpu)
            .map(|(i, _)| i as VertexId)
    }

    /// Try to place `req` for `dur` starting at or after `from`.
    /// Returns the chosen vertices and the earliest start time.
    /// `seed` rotates the leaf scan order so that concurrent planning
    /// workers (or successive jobs) spread across nodes instead of
    /// collapsing onto the first empty leaf — Borg's "relaxed
    /// randomization" in miniature.
    pub fn match_request(
        &self,
        req: &Resources,
        from: u64,
        dur: u64,
        seed: u64,
    ) -> Option<(Vec<VertexId>, u64)> {
        if req.is_zero() {
            return None;
        }
        let leaves = self.schedulable_nodes();
        if leaves.is_empty() {
            return None;
        }
        let n = leaves.len();
        let off = (seed as usize) % n;
        let mut best: Option<(Vec<VertexId>, u64)> = None;
        for k in 0..n {
            let id = leaves[(off + k) % n];
            if let Some(start) = self.vertex(id).earliest_fit(req, from, dur) {
                let better = match &best {
                    None => true,
                    Some((_, s)) => start < *s,
                };
                if better {
                    let done = start == from;
                    best = Some((vec![id], start));
                    if done {
                        break;
                    } // cannot do better than `from`
                }
            }
        }
        best
    }

    /// Node leaves the parent may place on: `Node` vertices that are not
    /// granted to a child instance, in vertex order.
    pub fn schedulable_nodes(&self) -> Vec<VertexId> {
        self.leaves()
            .filter(|&id| {
                let v = self.vertex(id);
                v.kind == VertexKind::Node && !v.name.starts_with("granted:")
            })
            .collect()
    }

    /// Gang matching (Slurm `-N`): the earliest common start `t >= from` at
    /// which at least `nodes` distinct node leaves can each hold `req` (per
    /// node) for `[t, t+dur)`. Returns the chosen vertices in ascending
    /// vertex order (that order defines the ranks) and `t`. `nodes <= 1`
    /// is exactly [`ResourceGraph::match_request`].
    ///
    /// Algorithm (fixpoint over per-leaf earliest fits). Let `e_l(t)` be
    /// leaf `l`'s `earliest_fit(req, t, dur)`. Starting at `t = from`, let
    /// `t'` be the `nodes`-th smallest `e_l(t)`. No start in `[t, t')` can
    /// work: a leaf feasible at `s` has `e_l(t) <= s`, and fewer than
    /// `nodes` leaves have `e_l(t) < t'`. So if `t' == t` the `nodes`
    /// leaves with `e_l(t) == t` are the answer; otherwise advance to `t'`.
    /// Because `e_l(t') == e_l(t)` whenever `e_l(t) >= t'` (earliest-fit is
    /// constant up to its own answer), only the at most `nodes - 1` leaves
    /// with `e_l(t) < t'` are re-queried per step.
    ///
    /// Complexity with `L` leaves, `S` spans per leaf and `K` advances:
    /// `O(L·S log S)` for the first sweep plus `O(K·(nodes·S log S + L))`
    /// afterwards; `K` is bounded by the number of distinct fit points
    /// (span ends and profile breakpoints) after `from`, and in practice is
    /// a handful. Each candidate is checked exactly, so capacity profiles
    /// and `granted:` exclusions are respected as in the single-node path.
    /// Termination: after every span has ended and every non-periodic
    /// profile step has passed, feasibility is periodic with the LCM of the
    /// leaves' profile periods, so a search that passes that horizon
    /// returns `None` (the gang can never start).
    ///
    /// `seed` rotates the scan order used to pick among more than `nodes`
    /// feasible leaves at `t`, spreading load like `match_request`.
    pub fn match_gang(
        &self,
        req: &Resources,
        nodes: u32,
        from: u64,
        dur: u64,
        seed: u64,
    ) -> Option<(Vec<VertexId>, u64)> {
        let need = nodes.max(1) as usize;
        if need == 1 {
            return self.match_request(req, from, dur, seed);
        }
        if req.is_zero() {
            return None;
        }
        let all = self.schedulable_nodes();
        if all.len() < need {
            return None;
        }
        let off = (seed as usize) % all.len();
        // Leaves in seed-rotated order, with their earliest fit from `t`.
        let mut fits: Vec<(VertexId, u64)> = (0..all.len())
            .map(|k| all[(off + k) % all.len()])
            .filter_map(|id| Some((id, self.vertex(id).earliest_fit(req, from, dur)?)))
            .collect();
        let horizon = self.gang_horizon(&all, from, dur);
        let mut t = from;
        loop {
            if fits.len() < need {
                return None; // fewer than `nodes` leaves can ever hold `req`
            }
            let mut starts: Vec<u64> = fits.iter().map(|f| f.1).collect();
            let (_, &mut t_next, _) = starts.select_nth_unstable(need - 1);
            if t_next == t {
                let mut chosen: Vec<VertexId> = fits
                    .iter()
                    .filter(|f| f.1 == t)
                    .take(need)
                    .map(|f| f.0)
                    .collect();
                chosen.sort_unstable();
                return Some((chosen, t));
            }
            if t_next > horizon {
                return None;
            }
            t = t_next;
            // Only leaves whose fit lies before `t` can have moved.
            fits.retain_mut(|(id, e)| {
                if *e >= t {
                    return true;
                }
                match self.vertex(*id).earliest_fit(req, t, dur) {
                    Some(s) => {
                        *e = s;
                        true
                    }
                    None => false,
                }
            });
        }
    }

    /// Latest start a gang search must examine: beyond it every leaf's
    /// feasibility is periodic (all spans over, every non-periodic profile
    /// step past), so a common start would already have been found.
    fn gang_horizon(&self, leaves: &[VertexId], from: u64, dur: u64) -> u64 {
        let mut settle = from;
        let mut lcm: u64 = 1;
        for &id in leaves {
            let v = self.vertex(id);
            settle = settle.max(v.planner.last_end());
            if let Some(p) = &v.profile {
                match p.period {
                    Some(per) if per > 0 => lcm = lcm_sat(lcm, per),
                    _ => {
                        let last = p.steps.iter().map(|s| s.0).max().unwrap_or(0);
                        settle = settle.max(last);
                    }
                }
            }
        }
        settle.saturating_add(lcm).saturating_add(dur)
    }

    /// Versions of the given vertices (a plan's read set).
    pub fn versions(&self, vertices: &[VertexId]) -> Vec<(VertexId, u64)> {
        vertices
            .iter()
            .map(|&v| (v, self.vertex(v).version))
            .collect()
    }

    /// Commit-time revalidation + reservation (report section 5.2: the
    /// plan applier calls this; a false return means the optimistic plan
    /// conflicted and the job must be re-evaluated).
    pub fn commit(&mut self, vertices: &[VertexId], req: &Resources, start: u64, dur: u64) -> bool {
        self.commit_job(vertices, req, start, dur, None, &[])
    }

    /// Commit with per-vertex optimistic concurrency. If every vertex is still
    /// at the version the plan read (`expect`), the plan was computed against
    /// this exact planner state and is valid by construction: insert without
    /// re-checking (fast path). Otherwise revalidate capacity (slow path) —
    /// the plan is rejected only if it no longer fits. All-or-nothing: every
    /// vertex is validated before any is booked, so a gang whose plan
    /// conflicts on one vertex leaves all of them untouched. Duplicate
    /// vertices are rejected.
    pub fn commit_job(
        &mut self,
        vertices: &[VertexId],
        req: &Resources,
        start: u64,
        dur: u64,
        job: Option<u64>,
        expect: &[(VertexId, u64)],
    ) -> bool {
        // A vertex listed twice would be checked once but booked twice. Gangs
        // are a handful of vertices, and this runs under the FSM write lock:
        // an allocation-free pairwise check beats sorting a copy.
        if vertices
            .iter()
            .enumerate()
            .any(|(i, v)| vertices[i + 1..].contains(v))
        {
            return false;
        }
        let fast = !expect.is_empty()
            && expect.len() == vertices.len()
            && expect.iter().all(|&(v, ver)| self.vertex(v).version == ver);
        if !fast {
            for &id in vertices {
                if self.vertex(id).earliest_fit(req, start, dur) != Some(start) {
                    return false;
                }
            }
        }
        for &id in vertices {
            let v = self.vertex_mut(id);
            v.planner.push_unchecked(req, start, dur, job);
            v.version += 1;
        }
        true
    }

    fn for_each_leaf_mut(&mut self, mut f: impl FnMut(&mut Vertex) -> bool) {
        for v in self.vertices.iter_mut() {
            if f(v) {
                v.version += 1;
            }
        }
    }

    /// Early completion: free the rest of the job's reserved window.
    pub fn release_job(&mut self, job: u64, at: u64) {
        self.for_each_leaf_mut(|v| v.planner.release_job(job, at));
    }

    /// Overrun: stretch the job's spans to `new_end` (physical fact; may
    /// overlap future reservations, which the next re-plan pass resolves).
    pub fn extend_job(&mut self, job: u64, new_end: u64) {
        self.for_each_leaf_mut(|v| v.planner.extend_job(job, new_end));
    }

    /// Drop every reservation starting after `now`; returns affected jobs.
    pub fn drop_future(&mut self, now: u64) -> Vec<u64> {
        let mut all = Vec::new();
        self.for_each_leaf_mut(|v| {
            let j = v.planner.drop_future(now);
            let changed = !j.is_empty();
            all.extend(j);
            changed
        });
        all.sort_unstable();
        all.dedup();
        all
    }

    pub fn job_span(&self, job: u64) -> Option<(u64, u64)> {
        self.vertices.iter().find_map(|v| v.planner.job_span(job))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_node_graph() -> ResourceGraph {
        let mut g = ResourceGraph::default();
        let root = g.add_vertex(VertexKind::Cluster, "cluster", Resources::default(), None);
        for n in 0..2 {
            g.add_vertex(
                VertexKind::Node,
                &format!("n{n}"),
                Resources {
                    cores: 8,
                    mem_mb: 1024,
                    gpus: 0,
                },
                Some(root),
            );
        }
        g
    }

    #[test]
    fn earliest_fit_respects_reservations() {
        let mut g = two_node_graph();
        let req = Resources {
            cores: 8,
            mem_mb: 0,
            gpus: 0,
        };
        // Fill n0 for [100, 200).
        let n0 = g.by_name("n0").unwrap();
        let cap = g.vertex(n0).capacity;
        assert!(g
            .vertex_mut(n0)
            .planner
            .reserve(CapView::fixed(cap), &req, 100, 100));
        // Request [50, 150) overlaps n0's reservation; must land on n1 at t=50.
        let (verts, start) = g.match_request(&req, 50, 100, 0).unwrap();
        assert_eq!(start, 50);
        assert_eq!(verts, vec![g.by_name("n1").unwrap()]);
    }

    #[test]
    fn backfill_finds_future_window() {
        let mut g = two_node_graph();
        let full = Resources {
            cores: 8,
            mem_mb: 0,
            gpus: 0,
        };
        // Occupy both nodes [0, 50).
        for n in ["n0", "n1"] {
            let id = g.by_name(n).unwrap();
            let cap = g.vertex(id).capacity;
            assert!(g
                .vertex_mut(id)
                .planner
                .reserve(CapView::fixed(cap), &full, 0, 50));
        }
        // Request must backfill to t=50.
        let (_, start) = g.match_request(&full, 0, 100, 0).unwrap();
        assert_eq!(start, 50);
    }

    #[test]
    fn commit_revalidation_rejects_conflicts() {
        let mut g = two_node_graph();
        let full = Resources {
            cores: 8,
            mem_mb: 0,
            gpus: 0,
        };
        let n0 = g.by_name("n0").unwrap();
        assert!(g.commit(&[n0], &full, 0, 100));
        // Second conflicting commit at same window must fail.
        assert!(!g.commit(&[n0], &full, 0, 100));
        // Non-overlapping future commit succeeds.
        assert!(g.commit(&[n0], &full, 100, 100));
    }
}

#[cfg(test)]
mod planner_tests {
    use super::*;

    fn r(c: u32) -> Resources {
        Resources {
            cores: c,
            mem_mb: 0,
            gpus: 0,
        }
    }

    /// Brute-force oracle on an integer grid: earliest start in [from, limit)
    /// where every unit second of the run fits.
    fn oracle(
        spans: &[(u64, u64, u32)],
        cap: &dyn Fn(u64) -> u32,
        req: u32,
        from: u64,
        dur: u64,
        limit: u64,
    ) -> Option<u64> {
        'outer: for t in from..limit {
            for x in t..t + dur {
                let used: u32 = spans
                    .iter()
                    .filter(|s| s.0 <= x && x < s.1)
                    .map(|s| s.2)
                    .sum();
                if used + req > cap(x) {
                    continue 'outer;
                }
            }
            return Some(t);
        }
        None
    }

    #[test]
    fn sweep_matches_bruteforce_oracle() {
        // Deterministic pseudo-random cases (splitmix64).
        let mut x: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            x = x.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        for case in 0..400 {
            let mut p = Planner::default();
            let mut spans = vec![];
            for _ in 0..(next() % 8) {
                let s = next() % 40;
                let d = 1 + next() % 15;
                let c = 1 + (next() % 4) as u32;
                p.spans.push(Span {
                    start: s,
                    end: s + d,
                    res: r(c),
                    job: None,
                });
                spans.push((s, s + d, c));
            }
            let base = 6u32;
            let periodic = case % 2 == 0;
            let prof = CapacityProfile {
                period: Some(20),
                steps: vec![(0, r(base)), (12, r(3))],
            };
            let capv = if periodic {
                CapView {
                    base: r(base),
                    profile: Some(&prof),
                }
            } else {
                CapView::fixed(r(base))
            };
            let capf = |t: u64| {
                if periodic {
                    if t % 20 >= 12 {
                        3
                    } else {
                        base
                    }
                } else {
                    base
                }
            };
            let req = 1 + (next() % 4) as u32;
            let from = next() % 30;
            let dur = 1 + next() % (if periodic { 8 } else { 20 });
            let got = p.earliest_fit(capv, &r(req), from, dur);
            let want = oracle(&spans, &capf, req, from, dur, 200);
            assert_eq!(
                got, want,
                "case {case}: spans={spans:?} req={req} from={from} dur={dur} periodic={periodic}"
            );
        }
    }

    /// One leaf for the gang oracle: its spans and its capacity over time.
    type LeafModel<'a> = (Vec<(u64, u64, u32)>, &'a dyn Fn(u64) -> u32);

    /// Brute-force gang oracle: earliest `t` in `[from, limit)` at which at
    /// least `n` leaves individually fit (per the single-leaf oracle).
    fn gang_oracle(
        leaves: &[LeafModel],
        req: u32,
        n: usize,
        from: u64,
        dur: u64,
        limit: u64,
    ) -> Option<u64> {
        (from..limit).find(|&t| {
            leaves
                .iter()
                .filter(|(spans, cap)| oracle(spans, *cap, req, t, dur, t + 1) == Some(t))
                .count()
                >= n
        })
    }

    #[test]
    fn gang_matches_bruteforce_oracle() {
        let mut x: u64 = 0x1234_5678_9ABC_DEF1;
        let mut next = || {
            x = x.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        let base = 6u32;
        let periodic_cap = |t: u64| if t % 20 >= 12 { 3 } else { base };
        let fixed_cap = |_t: u64| base;
        let prof = CapacityProfile {
            period: Some(20),
            steps: vec![(0, r(base)), (12, r(3))],
        };
        let mut delayed = 0;
        let mut infeasible = 0;
        for case in 0..500 {
            let mut g = ResourceGraph::default();
            let root = g.add_vertex(VertexKind::Cluster, "c", Resources::default(), None);
            let n_leaves = 2 + (next() % 4) as usize;
            let mut model: Vec<LeafModel> = vec![];
            for l in 0..n_leaves {
                let id = g.add_vertex(VertexKind::Node, &format!("n{l}"), r(base), Some(root));
                let periodic = next() % 3 == 0;
                if periodic {
                    g.set_profile(id, Some(prof.clone()));
                }
                let mut spans = vec![];
                for _ in 0..(next() % 6) {
                    let s = next() % 40;
                    let d = 1 + next() % 15;
                    let c = 1 + (next() % 4) as u32;
                    g.vertex_mut(id).planner.spans.push(Span {
                        start: s,
                        end: s + d,
                        res: r(c),
                        job: None,
                    });
                    spans.push((s, s + d, c));
                }
                let cap: &dyn Fn(u64) -> u32 = if periodic { &periodic_cap } else { &fixed_cap };
                model.push((spans, cap));
            }
            let need = 1 + (next() as usize) % n_leaves;
            let req = 1 + (next() % 4) as u32;
            let from = next() % 30;
            let dur = 1 + next() % 15;
            let got = g.match_gang(&r(req), need as u32, from, dur, next());
            let want = gang_oracle(&model, req, need, from, dur, 300);
            assert_eq!(
                got.as_ref().map(|g| g.1),
                want,
                "case {case}: need={need} req={req} from={from} dur={dur} model={:?}",
                model.iter().map(|m| &m.0).collect::<Vec<_>>()
            );
            match got {
                None => infeasible += 1,
                Some((verts, t)) => {
                    if t > from {
                        delayed += 1;
                    }
                    assert_eq!(verts.len(), need);
                    assert!(verts.windows(2).all(|w| w[0] < w[1]), "sorted, distinct");
                    for v in &verts {
                        assert_eq!(g.vertex(*v).earliest_fit(&r(req), t, dur), Some(t));
                    }
                }
            }
        }
        assert!(delayed > 50, "cases exercise delayed starts: {delayed}");
        assert!(
            infeasible > 5,
            "cases exercise infeasible gangs: {infeasible}"
        );
    }

    fn nodes_graph(n: usize, cores: u32) -> (ResourceGraph, Vec<VertexId>) {
        let mut g = ResourceGraph::default();
        let root = g.add_vertex(VertexKind::Cluster, "c", Resources::default(), None);
        let ids = (0..n)
            .map(|i| g.add_vertex(VertexKind::Node, &format!("n{i}"), r(cores), Some(root)))
            .collect();
        (g, ids)
    }

    #[test]
    fn gang_waits_for_enough_simultaneously_free_nodes() {
        let (mut g, ids) = nodes_graph(4, 4);
        // n0 busy [0,100), n1 busy [50,150), n2 busy [0,30), n3 free.
        assert!(g.commit_job(&[ids[0]], &r(4), 0, 100, Some(1), &[]));
        assert!(g.commit_job(&[ids[1]], &r(4), 50, 100, Some(2), &[]));
        assert!(g.commit_job(&[ids[2]], &r(4), 0, 30, Some(3), &[]));
        // Two nodes for 20s: n3 now, and n1 ([0,20) ends before its span).
        assert_eq!(
            g.match_gang(&r(4), 2, 0, 20, 0),
            Some((vec![ids[1], ids[3]], 0))
        );
        // Three nodes for 40s: n2 frees at 30 but n1 is busy from 50 and n0
        // until 100, so the earliest common window is [100,140) on n0,n2,n3.
        assert_eq!(
            g.match_gang(&r(4), 3, 0, 40, 0),
            Some((vec![ids[0], ids[2], ids[3]], 100))
        );
        // All four for 10s: only after n1 frees at 150.
        assert_eq!(g.match_gang(&r(4), 4, 0, 10, 7).unwrap().1, 150);
        // More nodes than exist, or a per-node request larger than any node.
        assert_eq!(g.match_gang(&r(4), 5, 0, 10, 0), None);
        assert_eq!(g.match_gang(&r(5), 2, 0, 10, 0), None);
        // nodes 0 and 1 are the single-node path.
        assert_eq!(
            g.match_gang(&r(4), 0, 0, 20, 3),
            g.match_request(&r(4), 0, 20, 3)
        );
        assert_eq!(
            g.match_gang(&r(4), 1, 0, 20, 3),
            g.match_request(&r(4), 0, 20, 3)
        );
    }

    #[test]
    fn gang_seed_rotates_choice_among_free_nodes() {
        let (g, ids) = nodes_graph(4, 4);
        let a = g.match_gang(&r(1), 2, 0, 10, 0).unwrap().0;
        let b = g.match_gang(&r(1), 2, 0, 10, 2).unwrap().0;
        assert_eq!(a, vec![ids[0], ids[1]]);
        assert_eq!(b, vec![ids[2], ids[3]]);
    }

    #[test]
    fn gang_skips_granted_nodes_and_honours_profiles() {
        let (mut g, ids) = nodes_graph(3, 8);
        let _grant = g.grant_subgraph(&ids[0..1]);
        // Only n1, n2 are visible to the parent: a 3-node gang never fits.
        assert_eq!(g.match_gang(&r(1), 3, 0, 10, 0), None);
        // n2 is capped to 2 cores during [0, 3600) each day.
        g.set_profile(
            ids[2],
            Some(CapacityProfile::daily(vec![(0, r(2)), (3600, r(8))])),
        );
        assert_eq!(
            g.match_gang(&r(4), 2, 0, 600, 0),
            Some((vec![ids[1], ids[2]], 3600))
        );
        // n1 now only has capacity in [0,100) each day, which never overlaps
        // n2's window: the gang can never start, and the horizon makes the
        // search terminate.
        g.set_profile(
            ids[1],
            Some(CapacityProfile::daily(vec![(0, r(8)), (100, r(0))])),
        );
        assert_eq!(g.match_gang(&r(4), 2, 0, 60, 0), None);
    }

    #[test]
    fn gang_commit_is_all_or_nothing() {
        let (mut g, ids) = nodes_graph(3, 4);
        let plan = g.match_gang(&r(4), 3, 0, 100, 0).unwrap();
        let read = g.versions(&plan.0);
        // A concurrent commit takes n2 inside the gang's window.
        assert!(g.commit_job(&[ids[2]], &r(4), 50, 10, Some(7), &[]));
        assert!(!g.commit_job(&plan.0, &r(4), plan.1, 100, Some(1), &read));
        for &v in &ids[0..2] {
            assert_eq!(g.vertex(v).planner.span_count(), 0, "no partial booking");
        }
        assert_eq!(g.versions(&ids[0..2]), read[0..2].to_vec());
        // A re-plan lands after the conflict and books every vertex.
        let plan = g.match_gang(&r(4), 3, 0, 100, 0).unwrap();
        assert_eq!(plan.1, 60);
        assert!(g.commit_job(&plan.0, &r(4), plan.1, 100, Some(1), &[]));
        assert!(ids
            .iter()
            .all(|&v| g.vertex(v).planner.job_span(1) == Some((60, 160))));
        // Listing a vertex twice is rejected outright.
        assert!(!g.commit_job(&[ids[0], ids[0]], &r(1), 500, 10, Some(2), &[]));
    }

    #[test]
    fn release_extend_and_drop_future() {
        let mut g = ResourceGraph::default();
        let n = g.add_vertex(VertexKind::Node, "n", r(4), None);
        assert!(g.commit_job(&[n], &r(4), 0, 100, Some(1), &[]));
        assert!(g.commit_job(&[n], &r(4), 100, 50, Some(2), &[]));
        // Early completion of job 1 at t=40 frees [40,100): job 2 can't move by
        // itself, but a new 4-core 60s job now fits at 40.
        g.release_job(1, 40);
        assert_eq!(g.vertex(n).earliest_fit(&r(4), 0, 60), Some(40));
        // Drop future reservations (job 2 starts at 100 > now=40).
        assert_eq!(g.drop_future(40), vec![2]);
        assert_eq!(g.vertex(n).earliest_fit(&r(4), 40, 500), Some(40));
        // Overrun: job 3 predicted [40,60) runs long -> extend to 90.
        assert!(g.commit_job(&[n], &r(2), 40, 20, Some(3), &[]));
        g.extend_job(3, 90);
        assert_eq!(g.job_span(3), Some((40, 90)));
        assert_eq!(g.vertex(n).earliest_fit(&r(3), 40, 10), Some(90));
    }

    #[test]
    fn version_fast_path_and_slow_path_revalidation() {
        let mut g = ResourceGraph::default();
        let a = g.add_vertex(VertexKind::Node, "a", r(4), None);
        let b = g.add_vertex(VertexKind::Node, "b", r(4), None);
        let read_a = g.versions(&[a]);
        // An unrelated commit on b must not invalidate a plan that read a.
        assert!(g.commit_job(&[b], &r(4), 0, 10, Some(9), &[]));
        assert!(g.commit_job(&[a], &r(2), 0, 10, Some(1), &read_a)); // fast path
                                                                     // A second plan that read a's *old* version still commits if it fits
                                                                     // (slow path revalidation)...
        assert!(g.commit_job(&[a], &r(2), 0, 10, Some(2), &read_a));
        // ...and is rejected only when it truly no longer fits.
        assert!(!g.commit_job(&[a], &r(1), 0, 10, Some(3), &read_a));
    }

    #[test]
    fn capacity_profile_shapes_placement() {
        let mut g = ResourceGraph::default();
        let n = g.add_vertex(VertexKind::Node, "n", r(8), None);
        // Daily cap: 8 cores, but only 2 between 16:00 and 20:00.
        g.set_profile(
            n,
            Some(CapacityProfile::daily(vec![
                (0, r(8)),
                (16 * 3600, r(2)),
                (20 * 3600, r(8)),
            ])),
        );
        // A 4-core 2h job asked for at 15:00 cannot run through the cap window;
        // earliest is 20:00.
        let t15 = 15 * 3600;
        assert_eq!(g.vertex(n).earliest_fit(&r(4), t15, 7200), Some(20 * 3600));
        // A 2-core job fits immediately.
        assert_eq!(g.vertex(n).earliest_fit(&r(2), t15, 7200), Some(t15));
        // Requests above the max capacity are infeasible.
        assert_eq!(g.vertex(n).earliest_fit(&r(9), 0, 10), None);
    }
}

// ---------------------------------------------------------------------------
// Phase 6: ownership-typed nested delegation (report section 4.4 / 5.2).
//
// A parent instance *moves* a `SubgraphGrant` to a child scheduler. The grant
// owns its vertex set by value; Rust's ownership rules make "child allocates
// outside its granted subgraph" a compile-time impossibility rather than a
// runtime convention (Flux enforces this by discipline in C++).
// ---------------------------------------------------------------------------

/// Capability a child scheduler receives. `Drop` returns the vertices to the
/// parent via the returned closure stored at grant time (in-process skeleton;
/// the distributed version sends a ReleaseSubgraph Raft event instead).
pub struct SubgraphGrant {
    vertices: Vec<VertexId>,
    /// Cloned graph restricted to the granted vertices for planning.
    local: ResourceGraph,
}

impl SubgraphGrant {
    pub fn vertices(&self) -> &[VertexId] {
        &self.vertices
    }

    /// Child-side scheduling: match within the grant only.
    pub fn match_within(
        &self,
        req: &Resources,
        from: u64,
        dur: u64,
        seed: u64,
    ) -> Option<(Vec<VertexId>, u64)> {
        let mut best: Option<(Vec<VertexId>, u64)> = None;
        let n = self.vertices.len();
        if n == 0 {
            return None;
        }
        for k in 0..n {
            let id = self.vertices[(seed as usize + k) % n];
            let v = self.local.vertex(id);
            if v.kind != VertexKind::Node {
                continue;
            }
            if let Some(start) = v.earliest_fit(req, from, dur) {
                if best.as_ref().is_none_or(|(_, s)| start < *s) {
                    best = Some((vec![id], start));
                }
            }
        }
        best
    }
}

impl ResourceGraph {
    /// Grant a set of node vertices to a child instance. The parent keeps its
    /// graph; the child receives a *view clone* plus the capability. Only the
    /// child's `SubgraphGrant` can be used for child-side matching, and the
    /// granted vertices are marked in the parent (name prefix) so parent-side
    /// matching skips them — the double-allocation invariant, in types.
    pub fn grant_subgraph(&mut self, vertices: &[VertexId]) -> SubgraphGrant {
        for &v in vertices {
            let old = self.vertex(v).name.clone();
            if !old.starts_with("granted:") {
                self.by_name.remove(&old);
                let new = format!("granted:{old}");
                self.by_name.insert(new.clone(), v);
                self.vertex_mut(v).name = new;
            }
        }
        SubgraphGrant {
            vertices: vertices.to_vec(),
            local: self.clone(),
        }
    }

    /// Parent-side leaves exclude granted vertices (name marker).
    pub fn ungranted_leaves(&self) -> impl Iterator<Item = VertexId> + '_ {
        self.leaves().filter(|&id| {
            let v = self.vertex(id);
            !v.name.starts_with("granted:")
        })
    }

    /// Reclaim a grant (child job finished / instance shutting down).
    pub fn reclaim(&mut self, grant: SubgraphGrant) {
        for &v in &grant.vertices {
            let old = self.vertex(v).name.clone();
            if let Some(stripped) = old.strip_prefix("granted:") {
                self.by_name.remove(&old);
                self.by_name.insert(stripped.to_string(), v);
                self.vertex_mut(v).name = stripped.to_string();
            }
        }
    }
}

#[cfg(test)]
mod delegation_tests {
    use super::*;

    #[test]
    fn subgraph_grant_excludes_parent_matching_and_reclaims() {
        let mut g = ResourceGraph::default();
        let root = g.add_vertex(VertexKind::Cluster, "cluster", Resources::default(), None);
        let mut ids = vec![];
        for n in 0..4 {
            ids.push(g.add_vertex(
                VertexKind::Node,
                &format!("n{n}"),
                Resources {
                    cores: 8,
                    mem_mb: 0,
                    gpus: 0,
                },
                Some(root),
            ));
        }
        // Grant n0,n1 to a child instance.
        let grant = g.grant_subgraph(&ids[0..2]);
        // Parent matching now only sees n2,n3.
        let req = Resources {
            cores: 8,
            mem_mb: 0,
            gpus: 0,
        };
        let (verts, _) = g.match_request(&req, 0, 100, 0).unwrap();
        assert!(verts.iter().all(|v| !ids[0..2].contains(v)));
        // Child matches within its grant.
        let (cverts, _) = grant.match_within(&req, 0, 100, 0).unwrap();
        assert!(cverts.iter().all(|v| ids[0..2].contains(v)));
        // Reclaim restores full parent visibility.
        g.reclaim(grant);
        assert_eq!(g.ungranted_leaves().count(), 4);
        assert!(g.by_name("n0").is_some());
    }
}
