//! `priority` world (ADR 0002): a job's scheduling priority, computed at
//! submit time and recorded in `JobSubmitted`.

use super::bindings::priority::PriorityPre;
use super::{plugin_loaders, wit_job, Host, PluginError, Sandbox};
use crate::PolicyConfig;
use gnomon_core::Job;
use std::sync::atomic::{AtomicU64, Ordering};

/// A loaded priority plugin.
pub struct PriorityPlugin(Sandbox<PriorityPre<Host>>);

plugin_loaders!(PriorityPlugin, PriorityPre<super::Host>);

impl PriorityPlugin {
    /// Run the plugin on `job` at time `now` (Unix seconds).
    pub fn priority(&self, job: &Job, now: u64) -> Result<u64, PluginError> {
        self.0.call(|store, pre| {
            pre.instantiate(&mut *store)?
                .call_priority(&mut *store, &wit_job(job), now)
        })
    }
}

/// Priority as the controller computes it: the plugin when one is loaded,
/// otherwise (or when the plugin fails) the declarative [`PolicyConfig`].
/// A plugin can make priorities worse; it cannot stop submissions.
pub struct PriorityHook {
    plugin: Option<PriorityPlugin>,
    fallback: PolicyConfig,
    /// QoS tier the fallback uses (jobs carry no QoS yet).
    pub fallback_qos: String,
    /// Plugin calls attempted.
    pub calls: AtomicU64,
    /// Plugin calls that fell back (fuel, memory, trap).
    pub failures: AtomicU64,
}

impl PriorityHook {
    pub fn new(plugin: Option<PriorityPlugin>, fallback: PolicyConfig) -> Self {
        PriorityHook {
            plugin,
            fallback,
            fallback_qos: "normal".into(),
            calls: AtomicU64::new(0),
            failures: AtomicU64::new(0),
        }
    }

    pub fn has_plugin(&self) -> bool {
        self.plugin.is_some()
    }

    /// The priority to record in `JobSubmitted`, with the plugin's error if
    /// it failed and the fallback was used.
    pub fn priority(&self, job: &Job, now: u64) -> (u64, Option<PluginError>) {
        if let Some(p) = &self.plugin {
            self.calls.fetch_add(1, Ordering::Relaxed);
            match p.priority(job, now) {
                Ok(v) => return (v, None),
                Err(e) => {
                    self.failures.fetch_add(1, Ordering::Relaxed);
                    return (self.fallback_priority(job, now), Some(e));
                }
            }
        }
        (self.fallback_priority(job, now), None)
    }

    /// The declarative priority alone.
    pub fn fallback_priority(&self, job: &Job, now: u64) -> u64 {
        self.fallback.priority(job, now, &self.fallback_qos)
    }
}

/// A candidate priority plugin run beside the live priority on real
/// submissions, applying nothing (the priority counterpart of
/// `gnomon_sched::ShadowPolicy`). It counts how often the candidate would
/// have given a different priority.
pub struct ShadowPriority {
    plugin: PriorityPlugin,
    pub compared: AtomicU64,
    pub differed: AtomicU64,
    pub failures: AtomicU64,
}

/// One shadow comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShadowOutcome {
    Same,
    /// The candidate's priority, where it differs from the live one.
    Differs(u64),
    Failed(String),
}

impl ShadowPriority {
    pub fn new(plugin: PriorityPlugin) -> Self {
        ShadowPriority {
            plugin,
            compared: AtomicU64::new(0),
            differed: AtomicU64::new(0),
            failures: AtomicU64::new(0),
        }
    }

    /// Compare the candidate with `job.priority` (the live value).
    pub fn compare(&self, job: &Job, now: u64) -> ShadowOutcome {
        self.compared.fetch_add(1, Ordering::Relaxed);
        match self.plugin.priority(job, now) {
            Ok(p) if p == job.priority => ShadowOutcome::Same,
            Ok(p) => {
                self.differed.fetch_add(1, Ordering::Relaxed);
                ShadowOutcome::Differs(p)
            }
            Err(e) => {
                self.failures.fetch_add(1, Ordering::Relaxed);
                ShadowOutcome::Failed(e.to_string())
            }
        }
    }

    /// Fraction of compared submissions where the candidate returned the
    /// live priority. A failed call is not agreement, so a candidate that
    /// always traps scores 0, not 1.
    pub fn agreement(&self) -> f64 {
        let n = self.compared.load(Ordering::Relaxed);
        let disagreed =
            self.differed.load(Ordering::Relaxed) + self.failures.load(Ordering::Relaxed);
        n.saturating_sub(disagreed) as f64 / n.max(1) as f64
    }
}
