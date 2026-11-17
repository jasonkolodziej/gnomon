//! `admission` world (ADR 0003): accept, reject or rewrite a submitted job,
//! the equivalent of Slurm's `job_submit`. Runs in the submit handler before
//! `JobSubmitted` is built; the (possibly rewritten) job is what the event
//! records.

use super::bindings::admission::AdmissionPre;
use super::bindings::types::{Request, Verdict as WitVerdict};
use super::{plugin_loaders, wit_job, Host, PluginError, Sandbox};
use gnomon_core::{Job, Resources, Walltime};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// A validated rewrite of a job's request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewrite {
    pub partition: String,
    pub request: Resources,
    pub nodes: u32,
    pub walltime_secs: u64,
}

impl Rewrite {
    fn validate(r: Request) -> Result<Self, PluginError> {
        let request = Resources {
            cores: r.cores,
            mem_mb: r.mem_mb,
            gpus: r.gpus,
        };
        let bad = |why: &str| Err(PluginError::Invalid(format!("rewrite: {why}")));
        if r.partition.is_empty() {
            return bad("empty partition");
        }
        if r.nodes == 0 || r.walltime_secs == 0 {
            return bad("nodes and walltime-secs must be at least 1");
        }
        if request.is_zero() {
            return bad("cores, mem-mb and gpus are all 0");
        }
        Ok(Rewrite {
            partition: r.partition,
            request,
            nodes: r.nodes,
            walltime_secs: r.walltime_secs,
        })
    }

    /// Apply to `job`; identity fields (id, user, submit time) are kept.
    pub fn apply(&self, job: &mut Job) {
        job.partition = self.partition.clone();
        job.request = self.request;
        job.nodes = self.nodes;
        job.walltime = Walltime(Duration::from_secs(self.walltime_secs));
    }
}

/// An admission plugin's decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Accept,
    Reject(String),
    Rewrite(Rewrite),
}

/// A loaded admission plugin.
pub struct AdmissionPlugin(Sandbox<AdmissionPre<Host>>);

plugin_loaders!(AdmissionPlugin, AdmissionPre<super::Host>);

impl AdmissionPlugin {
    /// Run the plugin on `job` at time `now`. A rewrite is validated here,
    /// so an invalid one is an error, never a half-applied job.
    pub fn admit(&self, job: &Job, now: u64) -> Result<Verdict, PluginError> {
        let v = self.0.call(|store, pre| {
            pre.instantiate(&mut *store)?
                .call_admit(&mut *store, &wit_job(job), now)
        })?;
        Ok(match v {
            WitVerdict::Accept => Verdict::Accept,
            WitVerdict::Reject(reason) => Verdict::Reject(reason),
            WitVerdict::Rewrite(r) => Verdict::Rewrite(Rewrite::validate(r)?),
        })
    }
}

/// What to do with a job when the admission plugin fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailMode {
    /// Accept it unchanged (the default: a broken plugin cannot stop
    /// submissions).
    Open,
    /// Reject it (for sites where admission enforces a hard rule).
    Closed,
}

impl std::str::FromStr for FailMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "open" => Ok(FailMode::Open),
            "closed" => Ok(FailMode::Closed),
            _ => Err(format!("'{s}' is not open or closed")),
        }
    }
}

/// Admission as the controller runs it, with counters.
pub struct AdmissionHook {
    plugin: AdmissionPlugin,
    pub fail: FailMode,
    pub calls: AtomicU64,
    pub failures: AtomicU64,
    pub rejected: AtomicU64,
    pub rewritten: AtomicU64,
}

impl AdmissionHook {
    pub fn new(plugin: AdmissionPlugin, fail: FailMode) -> Self {
        AdmissionHook {
            plugin,
            fail,
            calls: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            rewritten: AtomicU64::new(0),
        }
    }

    /// Admit `job`, rewriting it in place if the plugin says so. `Err` is
    /// the rejection reason for the submitter. The second value is the
    /// plugin's error when it failed and [`FailMode`] decided.
    pub fn admit(&self, job: &mut Job, now: u64) -> (Result<(), String>, Option<PluginError>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        match self.plugin.admit(job, now) {
            Ok(Verdict::Accept) => (Ok(()), None),
            Ok(Verdict::Reject(reason)) => {
                self.rejected.fetch_add(1, Ordering::Relaxed);
                (Err(reason), None)
            }
            Ok(Verdict::Rewrite(r)) => {
                self.rewritten.fetch_add(1, Ordering::Relaxed);
                r.apply(job);
                (Ok(()), None)
            }
            Err(e) => {
                self.failures.fetch_add(1, Ordering::Relaxed);
                let outcome = match self.fail {
                    FailMode::Open => Ok(()),
                    FailMode::Closed => {
                        self.rejected.fetch_add(1, Ordering::Relaxed);
                        Err("admission policy unavailable".to_string())
                    }
                };
                (outcome, Some(e))
            }
        }
    }
}
