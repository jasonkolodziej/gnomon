//! Policy-as-data (report section 4.6 phase 5).
//!
//! Scheduling policy is configuration, not code: fairshare weights, QoS
//! tiers, per-user/partition limits are declarative JSON evaluated by pure
//! functions. Priority computation can optionally be delegated to a
//! WASM-sandboxed plugin ([`plugin`], ADR 0002): a pure, fuel- and
//! memory-bounded wasmtime component that falls back to [`PolicyConfig`] when
//! it fails, replacing Slurm's in-process C plugin hazards.

pub mod fixtures;
pub mod plugin;

pub use plugin::{PluginError, PluginLimits, PriorityHook, PriorityPlugin};

use gnomon_core::{Job, Resources};
use serde::{Deserialize, Serialize};

/// Declarative policy configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Multifactor weights (Slurm-compatible semantics).
    pub weight_age: u64,
    pub weight_jobsize: u64,
    pub weight_qos: u64,
    /// QoS tiers: name -> base priority boost.
    pub qos_tiers: Vec<QosTier>,
    /// Per-partition limits.
    pub partitions: Vec<PartitionLimit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QosTier {
    pub name: String,
    pub priority_boost: u64,
    pub max_walltime_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartitionLimit {
    pub name: String,
    pub max_cores_per_user: u32,
    pub max_gpus_per_user: u32,
    pub max_running_jobs_per_user: u32,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        PolicyConfig {
            weight_age: 1000,
            weight_jobsize: 100,
            weight_qos: 10000,
            qos_tiers: vec![
                QosTier {
                    name: "normal".into(),
                    priority_boost: 0,
                    max_walltime_secs: 86400,
                },
                QosTier {
                    name: "high".into(),
                    priority_boost: 5000,
                    max_walltime_secs: 43200,
                },
            ],
            partitions: vec![PartitionLimit {
                name: "normal".into(),
                max_cores_per_user: 512,
                max_gpus_per_user: 64,
                max_running_jobs_per_user: 128,
            }],
        }
    }
}

impl PolicyConfig {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Pure priority function: age + size + QoS (a Slurm multifactor subset).
    /// Size counts the whole job: a gang's `request` is per node.
    pub fn priority(&self, job: &Job, now: u64, qos: &str) -> u64 {
        let age = now.saturating_sub(job.submit_time) / 60; // minutes waited
        let n = u64::from(job.node_count());
        let size = n * (u64::from(job.request.cores) + 4 * u64::from(job.request.gpus));
        let qos_boost = self
            .qos_tiers
            .iter()
            .find(|t| t.name == qos)
            .map(|t| t.priority_boost)
            .unwrap_or(0);
        self.weight_age * age + self.weight_jobsize * size + self.weight_qos * qos_boost / 1000
    }

    /// Admission check against per-partition limits. A gang uses `request` on
    /// each of its nodes, so it counts `nodes x request` against the limits.
    pub fn admits(&self, job: &Job, user_running: u32, user_usage: &Resources) -> bool {
        let n = job.node_count();
        let (cores, gpus) = (
            job.request.cores.saturating_mul(n),
            job.request.gpus.saturating_mul(n),
        );
        match self.partitions.iter().find(|p| p.name == job.partition) {
            Some(lim) => {
                user_running < lim.max_running_jobs_per_user
                    && user_usage.cores.saturating_add(cores) <= lim.max_cores_per_user
                    && user_usage.gpus.saturating_add(gpus) <= lim.max_gpus_per_user
            }
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnomon_core::{Job, JobId, Resources, Walltime};
    use std::time::Duration;

    #[test]
    fn declarative_priority_and_limits() {
        let cfg = PolicyConfig::default();
        let mut j = Job::new(
            JobId(1),
            "a",
            "u",
            Resources {
                cores: 8,
                mem_mb: 0,
                gpus: 1,
            },
            Walltime(Duration::from_secs(3600)),
        );
        j.submit_time = 0;
        let p_high = cfg.priority(&j, 3600, "high");
        let p_norm = cfg.priority(&j, 3600, "normal");
        assert!(p_high > p_norm);
        assert!(cfg.admits(
            &j,
            10,
            &Resources {
                cores: 100,
                mem_mb: 0,
                gpus: 10
            }
        ));
        assert!(!cfg.admits(&j, 200, &Resources::default())); // running-jobs limit
    }

    #[test]
    fn gangs_count_every_node_in_size_and_limits() {
        let cfg = PolicyConfig::default();
        let mut j = Job::new(
            JobId(1),
            "mpi",
            "u",
            Resources {
                cores: 64,
                mem_mb: 0,
                gpus: 0,
            },
            Walltime(Duration::from_secs(3600)),
        );
        j.submit_time = 0;
        let single = cfg.priority(&j, 0, "normal");
        j.nodes = 4;
        assert_eq!(cfg.priority(&j, 0, "normal"), 4 * single);
        // 4 x 64 = 256 cores: admitted with 256 in use (512 limit), refused with 257.
        let used = |cores| Resources {
            cores,
            mem_mb: 0,
            gpus: 0,
        };
        assert!(cfg.admits(&j, 0, &used(256)));
        assert!(!cfg.admits(&j, 0, &used(257)));
    }
}
