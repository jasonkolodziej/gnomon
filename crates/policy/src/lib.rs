//! Policy-as-data (report section 4.6 phase 5).
//!
//! Scheduling policy is configuration, not code: fairshare weights, QoS
//! tiers, per-user/partition limits are declarative JSON evaluated by pure
//! functions. Priority computation can optionally be delegated to a
//! WASM-sandboxed plugin (wasmtime) — untrusted policy code gets no host
//! memory access, replacing Slurm's in-process C plugin hazards.

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
    pub fn priority(&self, job: &Job, now: u64, qos: &str) -> u64 {
        let age = now.saturating_sub(job.submit_time) / 60; // minutes waited
        let size = (job.request.cores as u64) + 4 * (job.request.gpus as u64);
        let qos_boost = self
            .qos_tiers
            .iter()
            .find(|t| t.name == qos)
            .map(|t| t.priority_boost)
            .unwrap_or(0);
        self.weight_age * age + self.weight_jobsize * size + self.weight_qos * qos_boost / 1000
    }

    /// Admission check against per-partition limits.
    pub fn admits(&self, job: &Job, user_running: u32, user_usage: &Resources) -> bool {
        match self.partitions.iter().find(|p| p.name == job.partition) {
            Some(lim) => {
                user_running < lim.max_running_jobs_per_user
                    && user_usage.cores + job.request.cores <= lim.max_cores_per_user
                    && user_usage.gpus + job.request.gpus <= lim.max_gpus_per_user
            }
            None => true,
        }
    }
}

/// WASM-sandboxed priority plugin host (phase 5 differentiator).
/// Plugin ABI: exports `memory` and `priority(job_ptr, job_len) -> u64`;
/// the job is passed as serialized JSON in linear memory.
pub struct WasmPriorityPlugin {
    engine: wasmtime::Engine,
    module: wasmtime::Module,
}

impl WasmPriorityPlugin {
    pub fn from_wat(wat: &str) -> anyhow::Result<Self> {
        let engine = wasmtime::Engine::default();
        let module = wasmtime::Module::new(&engine, wat)?;
        Ok(Self { engine, module })
    }

    pub fn priority(&self, job_json: &str) -> anyhow::Result<u64> {
        let mut store = wasmtime::Store::new(&self.engine, ());
        let instance = wasmtime::Instance::new(&mut store, &self.module, &[])?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| anyhow::anyhow!("plugin must export memory"))?;
        let alloc = instance.get_typed_func::<u32, u32>(&mut store, "alloc")?;
        let prio = instance.get_typed_func::<(u32, u32), u64>(&mut store, "priority")?;
        let ptr = alloc.call(&mut store, job_json.len() as u32)?;
        memory.write(&mut store, ptr as usize, job_json.as_bytes())?;
        let v = prio.call(&mut store, (ptr, job_json.len() as u32))?;
        Ok(v)
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
    fn wasm_priority_plugin_runs_sandboxed() {
        // WAT plugin: priority = job JSON byte length (trivial but proves the
        // sandbox round-trip: alloc -> write -> call -> read result).
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "alloc") (param i32) (result i32) (i32.const 0))
            (func (export "priority") (param i32 i32) (result i64)
                (i64.extend_i32_u (local.get 1))))"#;
        let plugin = WasmPriorityPlugin::from_wat(wat).unwrap();
        let prio = plugin.priority("{\"cores\":8}").unwrap();
        assert_eq!(prio, "{\"cores\":8}".len() as u64);
    }
}
