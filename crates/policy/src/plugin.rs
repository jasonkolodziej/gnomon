//! Sandboxed WASM priority plugins (ADR 0002).
//!
//! A plugin is a WebAssembly *component* implementing the `priority` world
//! in `wit/priority.wit`. The host enforces the ADR's three rules:
//!
//! 1. Plugins run before an event is created (the API's submit handler), and
//!    their result is recorded in the event. Replicas and replay never run
//!    them. Nothing in this module touches the FSM.
//! 2. Plugins are pure: the linker provides no imports (no WASI, no host
//!    functions), and the engine is configured for reproducible results
//!    (NaN canonicalization, no threads, deterministic relaxed SIMD).
//! 3. Every call has a fuel budget and a memory cap. Running out of either,
//!    trapping, or failing to load all surface as [`PluginError`];
//!    [`PriorityHook`] turns them into the declarative fallback.

use crate::PolicyConfig;
use gnomon_core::Job;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "priority",
    });
}

use bindings::gnomon::policy::types::Job as WitJob;
use bindings::PriorityPre;

/// The WIT package this host implements. Breaking interface changes get a
/// new major version (ADR 0002, Consequences).
pub const WIT_PACKAGE: &str = "gnomon:policy@0.1.0";
const TYPES_INTERFACE: &str = "gnomon:policy/types@0.1.0";

/// Per-call resource limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginLimits {
    /// Fuel per call; roughly one unit per executed WASM instruction.
    pub fuel: u64,
    /// Cap on the plugin's linear memory, in bytes.
    pub max_memory_bytes: usize,
}

impl Default for PluginLimits {
    /// 10 M instructions (a few milliseconds) and 16 MiB of memory: far more
    /// than a priority function needs, small enough not to hurt submits.
    fn default() -> Self {
        PluginLimits {
            fuel: 10_000_000,
            max_memory_bytes: 16 << 20,
        }
    }
}

/// Why a plugin produced no value.
#[derive(Debug)]
pub enum PluginError {
    /// The bytes are not a valid component for the `priority` world.
    Load(anyhow::Error),
    /// The call used its whole fuel budget.
    OutOfFuel,
    /// The call trapped, including a memory grow past the cap or an
    /// instantiation the limits refused.
    Trap(anyhow::Error),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::Load(e) => write!(f, "plugin load failed: {e:#}"),
            PluginError::OutOfFuel => f.write_str("plugin ran out of fuel"),
            PluginError::Trap(e) => write!(f, "plugin trapped: {e:#}"),
        }
    }
}

impl std::error::Error for PluginError {}

/// Store data: only the resource limiter. Plugins get no host state.
struct Host {
    limits: StoreLimits,
}

/// The engine configuration of Rules 2 and 3.
fn engine() -> anyhow::Result<Engine> {
    let mut cfg = Config::new();
    cfg.wasm_component_model(true)
        .consume_fuel(true)
        .cranelift_nan_canonicalization(true)
        .wasm_threads(false)
        .wasm_relaxed_simd(false)
        .relaxed_simd_deterministic(true)
        .max_wasm_stack(512 << 10);
    Engine::new(&cfg)
}

/// A loaded, pre-linked priority plugin. Each call gets a fresh store and
/// instance from the pre-linked `InstancePre`, so no state survives between
/// calls and a call that misbehaves cannot affect the next one.
pub struct PriorityPlugin {
    engine: Engine,
    pre: PriorityPre<Host>,
    limits: PluginLimits,
}

impl PriorityPlugin {
    /// Load a component from its binary encoding (or, for tests and quick
    /// experiments, its text format).
    pub fn from_bytes(bytes: &[u8], limits: PluginLimits) -> Result<Self, PluginError> {
        let engine = engine().map_err(PluginError::Load)?;
        let component = Component::new(&engine, bytes).map_err(PluginError::Load)?;
        Self::link(engine, component, limits)
    }

    /// Load a component from a `.wasm` (or `.wat`) file.
    pub fn from_file(path: impl AsRef<Path>, limits: PluginLimits) -> Result<Self, PluginError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .map_err(|e| PluginError::Load(anyhow::anyhow!("{}: {e}", path.display())))?;
        Self::from_bytes(&bytes, limits)
    }

    fn link(
        engine: Engine,
        component: Component,
        limits: PluginLimits,
    ) -> Result<Self, PluginError> {
        let mut linker = Linker::<Host>::new(&engine);
        // `use types.{job}` makes the world import the (types-only) `types`
        // interface. Defining it empty satisfies that import; anything else
        // a plugin imports (WASI, say) stays undefined and fails here.
        linker
            .instance(TYPES_INTERFACE)
            .map_err(PluginError::Load)?;
        let pre = linker
            .instantiate_pre(&component)
            .and_then(PriorityPre::new)
            .map_err(PluginError::Load)?;
        Ok(PriorityPlugin {
            engine,
            pre,
            limits,
        })
    }

    pub fn limits(&self) -> PluginLimits {
        self.limits
    }

    /// Run the plugin on `job` at time `now` (Unix seconds).
    pub fn priority(&self, job: &Job, now: u64) -> Result<u64, PluginError> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.max_memory_bytes)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&self.engine, Host { limits });
        store.limiter(|h| &mut h.limits);
        store
            .set_fuel(self.limits.fuel)
            .map_err(PluginError::Trap)?;
        let result = self
            .pre
            .instantiate(&mut store)
            .and_then(|p| p.call_priority(&mut store, &wit_job(job), now));
        result.map_err(|e| match e.downcast_ref::<wasmtime::Trap>() {
            Some(wasmtime::Trap::OutOfFuel) => PluginError::OutOfFuel,
            _ => PluginError::Trap(e),
        })
    }
}

fn wit_job(job: &Job) -> WitJob {
    WitJob {
        id: job.id.0,
        user: job.user.clone(),
        partition: job.partition.clone(),
        cores: job.request.cores,
        mem_mb: job.request.mem_mb,
        gpus: job.request.gpus,
        nodes: job.node_count(),
        walltime_secs: job.walltime.secs(),
        submit_time: job.submit_time,
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

    fn fallback_priority(&self, job: &Job, now: u64) -> u64 {
        self.fallback.priority(job, now, &self.fallback_qos)
    }
}
