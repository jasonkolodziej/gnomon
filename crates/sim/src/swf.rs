//! Standard Workload Format (SWF) traces — the Parallel Workloads Archive
//! format (Feitelson et al.). Fields used: 1 job id, 2 submit, 4 run time,
//! 5 allocated procs, 8 requested procs, 9 requested time, 12 user.

use crate::SimJob;
use gnomon_core::{Job, JobId, Resources, Walltime};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Trace {
    pub name: String,
    pub max_procs: u32,
    pub unix_start: u64,
    pub jobs: Vec<TraceJob>,
}

#[derive(Clone, Copy, Debug)]
pub struct TraceJob {
    pub id: u64,
    pub submit: u64,
    pub run: u64,
    pub procs: u32,
    pub req_time: u64,
    pub user: i64,
}

fn header_num(line: &str, key: &str) -> Option<u64> {
    let rest = line.strip_prefix(';')?.trim();
    let rest = rest.strip_prefix(key)?.trim_start().strip_prefix(':')?.trim();
    rest.split_whitespace().next()?.parse().ok()
}

/// Parse an SWF trace. Keeps jobs with a positive runtime, processor count
/// and requested time, and that fit the machine.
pub fn parse(name: &str, text: &str) -> Trace {
    let (mut max_procs, mut max_nodes, mut unix_start) = (None, None, 0u64);
    let mut jobs = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() { continue; }
        if l.starts_with(';') {
            if let Some(v) = header_num(l, "MaxProcs") { max_procs = Some(v as u32); }
            if let Some(v) = header_num(l, "MaxNodes") { max_nodes = Some(v as u32); }
            if let Some(v) = header_num(l, "UnixStartTime") { unix_start = v; }
            continue;
        }
        let f: Vec<f64> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        if f.len() < 12 { continue; }
        let procs = if f[7] > 0.0 { f[7] } else { f[4] };
        let (run, req) = (f[3], f[8]);
        if run <= 0.0 || procs <= 0.0 || req <= 0.0 { continue; }
        jobs.push(TraceJob { id: f[0] as u64, submit: f[1].max(0.0) as u64, run: run as u64,
            procs: procs as u32, req_time: req as u64, user: f[11] as i64 });
    }
    let max_procs = max_procs.or(max_nodes).unwrap_or_else(|| jobs.iter().map(|j| j.procs).max().unwrap_or(1));
    jobs.retain(|j| j.procs <= max_procs);
    jobs.sort_by_key(|j| (j.submit, j.id));
    Trace { name: name.into(), max_procs, unix_start, jobs }
}

impl Trace {
    /// `n` consecutive jobs starting at `offset`, rebased so the window starts
    /// at its original time-of-day (UTC) — keeps daily cycles aligned for
    /// power/carbon profiles. Runtimes are clipped to the requested time
    /// (the walltime is the kill limit).
    pub fn window(&self, offset: usize, n: usize) -> Vec<SimJob> {
        let slice = &self.jobs[offset.min(self.jobs.len())..(offset + n).min(self.jobs.len())];
        let Some(first) = slice.first() else { return vec![] };
        let base = first.submit;
        let tod = (self.unix_start + base) % 86_400;
        slice.iter().enumerate().map(|(i, t)| {
            let mut job = Job::new(JobId(i as u64 + 1), "swf", &format!("u{}", t.user),
                Resources { cores: t.procs, mem_mb: 0, gpus: 0 },
                Walltime(Duration::from_secs(t.req_time)));
            job.submit_time = t.submit - base + tod;
            job.priority = 0; // FCFS: ties broken by id = submit order
            SimJob { job, actual: t.run.min(t.req_time).max(1) }
        }).collect()
    }

    /// The `n` jobs before `offset` — history used to warm up learned models
    /// (a production scheduler starts with its accounting history).
    pub fn history(&self, offset: usize, n: usize) -> Vec<SimJob> {
        let s = offset.saturating_sub(n);
        self.window(s, offset - s)
    }
}
