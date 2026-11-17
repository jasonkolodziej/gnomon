//! Sandboxed WASM policy plugins (ADR 0002, 0003, 0004).
//!
//! A plugin is a WebAssembly *component* implementing one of the worlds in
//! `wit/policy.wit`: [`priority`], [`admission`], [`estimator`] or
//! [`node_score`]. Every world shares the sandbox in this module, which
//! enforces ADR 0002's three rules:
//!
//! 1. Plugins run where a write is prepared (the submit handler, or the
//!    leader building a plan), and their result is recorded in the event.
//!    Replicas and replay never run them. Nothing here touches the FSM.
//! 2. Plugins are pure: the linker provides no imports (no WASI, no host
//!    functions), and the engine is configured for reproducible results
//!    (NaN canonicalization, no threads, deterministic relaxed SIMD).
//! 3. Every call has a fuel budget and a memory cap. Running out of either,
//!    trapping, failing to load or returning an invalid value all surface as
//!    [`PluginError`], and each world's hook falls back to declarative
//!    behaviour.

pub mod admission;
pub mod estimator;
pub mod node_score;
pub mod priority;

use std::path::Path;
use wasmtime::component::{Component, InstancePre, Linker};
use wasmtime::{Config, Engine, ResourceLimiter, Store};

mod bindings {
    pub mod priority {
        wasmtime::component::bindgen!({ path: "wit", world: "priority" });
    }
    pub mod admission {
        wasmtime::component::bindgen!({
            path: "wit",
            world: "admission",
            with: { "gnomon:policy/types": super::priority::gnomon::policy::types },
        });
    }
    pub mod estimator {
        wasmtime::component::bindgen!({
            path: "wit",
            world: "estimator",
            with: { "gnomon:policy/types": super::priority::gnomon::policy::types },
        });
    }
    pub mod node_score {
        wasmtime::component::bindgen!({
            path: "wit",
            world: "node-score",
            with: { "gnomon:policy/types": super::priority::gnomon::policy::types },
        });
    }
    pub use priority::gnomon::policy::types;
}

/// The WIT package this host implements. Breaking interface changes get a
/// new major version (ADR 0002, Consequences).
pub const WIT_PACKAGE: &str = "gnomon:policy@0.1.0";
const TYPES_INTERFACE: &str = "gnomon:policy/types@0.1.0";

/// Per-call resource limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginLimits {
    /// Fuel per call; roughly one unit per executed WASM instruction.
    pub fuel: u64,
    /// Cap on the plugin's linear memory, in bytes, summed over every memory
    /// the component instantiates.
    pub max_memory_bytes: usize,
}

impl Default for PluginLimits {
    /// 10 M instructions (a few milliseconds) and 16 MiB of memory: far more
    /// than a policy function needs, small enough not to hurt submits.
    fn default() -> Self {
        PluginLimits {
            fuel: 10_000_000,
            max_memory_bytes: 16 << 20,
        }
    }
}

/// Why a plugin produced no usable value.
#[derive(Debug)]
pub enum PluginError {
    /// The bytes are not a valid component for the expected world.
    Load(anyhow::Error),
    /// The call used its whole fuel budget.
    OutOfFuel,
    /// The call trapped, including a memory grow past the cap or an
    /// instantiation the limits refused.
    Trap(anyhow::Error),
    /// The call returned a value the host cannot use (an invalid rewrite,
    /// a score list of the wrong length).
    Invalid(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::Load(e) => write!(f, "plugin load failed: {e:#}"),
            PluginError::OutOfFuel => f.write_str("plugin ran out of fuel"),
            PluginError::Trap(e) => write!(f, "plugin trapped: {e:#}"),
            PluginError::Invalid(e) => write!(f, "plugin returned an invalid value: {e}"),
        }
    }
}

impl std::error::Error for PluginError {}

/// Store data: only the resource limiter. Plugins get no host state.
struct Host {
    budget: Budget,
}

/// Table elements a call may allocate across all its tables. A `funcref`
/// table needs a few for indirect calls; nothing legitimate needs more.
const MAX_TABLE_ELEMENTS: usize = 64 * 1024;
/// Caps on how many instances, memories and tables one call may create.
/// Compiled plugins use one memory and a handful of core instances.
const MAX_INSTANCES: usize = 32;
const MAX_MEMORIES: usize = 4;
const MAX_TABLES: usize = 8;

/// The store's resource limiter. Wasmtime's `StoreLimits` caps each memory
/// separately, so a component with several memories (or a huge table) could
/// still allocate far more than `max_memory_bytes`. This budget is shared by
/// every memory and table in the store, and a growth past it traps.
struct Budget {
    memory_left: usize,
    table_elements_left: usize,
}

impl Budget {
    fn new(limits: &PluginLimits) -> Self {
        Budget {
            memory_left: limits.max_memory_bytes,
            table_elements_left: MAX_TABLE_ELEMENTS,
        }
    }

    fn take(left: &mut usize, current: usize, desired: usize, what: &str) -> anyhow::Result<bool> {
        let extra = desired.saturating_sub(current);
        match left.checked_sub(extra) {
            Some(rest) => {
                *left = rest;
                Ok(true)
            }
            None => anyhow::bail!("plugin {what} budget exceeded"),
        }
    }
}

impl ResourceLimiter for Budget {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        Self::take(&mut self.memory_left, current, desired, "memory")
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        Self::take(&mut self.table_elements_left, current, desired, "table")
    }

    fn instances(&self) -> usize {
        MAX_INSTANCES
    }

    fn memories(&self) -> usize {
        MAX_MEMORIES
    }

    fn tables(&self) -> usize {
        MAX_TABLES
    }
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

/// A loaded, pre-linked component for one world (`P` is that world's
/// generated `*Pre`). Each call gets a fresh store and instance, so no state
/// survives between calls and a call that misbehaves cannot affect the next.
struct Sandbox<P> {
    engine: Engine,
    pre: P,
    limits: PluginLimits,
}

impl<P> Sandbox<P> {
    fn from_bytes(
        bytes: &[u8],
        limits: PluginLimits,
        pre: impl FnOnce(InstancePre<Host>) -> anyhow::Result<P>,
    ) -> Result<Self, PluginError> {
        let engine = engine().map_err(PluginError::Load)?;
        let component = Component::new(&engine, bytes).map_err(PluginError::Load)?;
        let mut linker = Linker::<Host>::new(&engine);
        // `use types.{...}` makes each world import the (types-only) `types`
        // interface. Defining it empty satisfies that import; anything else
        // a plugin imports (WASI, say) stays undefined and fails here.
        linker
            .instance(TYPES_INTERFACE)
            .map_err(PluginError::Load)?;
        let pre = linker
            .instantiate_pre(&component)
            .and_then(pre)
            .map_err(PluginError::Load)?;
        Ok(Sandbox {
            engine,
            pre,
            limits,
        })
    }

    fn from_file(
        path: &Path,
        limits: PluginLimits,
        pre: impl FnOnce(InstancePre<Host>) -> anyhow::Result<P>,
    ) -> Result<Self, PluginError> {
        let bytes = std::fs::read(path)
            .map_err(|e| PluginError::Load(anyhow::anyhow!("{}: {e}", path.display())))?;
        Self::from_bytes(&bytes, limits, pre)
    }

    /// Run `f` in a fresh store with this plugin's fuel and memory limits.
    fn call<R>(
        &self,
        f: impl FnOnce(&mut Store<Host>, &P) -> anyhow::Result<R>,
    ) -> Result<R, PluginError> {
        let budget = Budget::new(&self.limits);
        let mut store = Store::new(&self.engine, Host { budget });
        store.limiter(|h| &mut h.budget);
        store
            .set_fuel(self.limits.fuel)
            .map_err(PluginError::Trap)?;
        f(&mut store, &self.pre).map_err(|e| match e.downcast_ref::<wasmtime::Trap>() {
            Some(wasmtime::Trap::OutOfFuel) => PluginError::OutOfFuel,
            _ => PluginError::Trap(e),
        })
    }
}

fn wit_job(job: &gnomon_core::Job) -> bindings::types::Job {
    bindings::types::Job {
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

/// Defines `from_bytes`, `from_file` and `limits` for a world's plugin type.
macro_rules! plugin_loaders {
    ($ty:ident, $pre:path) => {
        impl $ty {
            /// Load a component from its binary encoding (or, for tests and
            /// quick experiments, its text format).
            pub fn from_bytes(
                bytes: &[u8],
                limits: $crate::plugin::PluginLimits,
            ) -> Result<Self, $crate::plugin::PluginError> {
                $crate::plugin::Sandbox::from_bytes(bytes, limits, <$pre>::new).map(|s| $ty(s))
            }

            /// Load a component from a `.wasm` (or `.wat`) file.
            pub fn from_file(
                path: impl AsRef<std::path::Path>,
                limits: $crate::plugin::PluginLimits,
            ) -> Result<Self, $crate::plugin::PluginError> {
                $crate::plugin::Sandbox::from_file(path.as_ref(), limits, <$pre>::new)
                    .map(|s| $ty(s))
            }

            pub fn limits(&self) -> $crate::plugin::PluginLimits {
                self.0.limits
            }
        }
    };
}
use plugin_loaders;
