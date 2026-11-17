//! Example Gnomon policy plugins (ADR 0002, 0003, 0004), one per WIT world
//! in `crates/policy/wit/policy.wit`. Each is selected by a cargo feature
//! and built into its own component; see `README.md`.
//!
//! Every plugin is a pure function of its arguments: it imports nothing (no
//! WASI, no clock: `now` is passed in) and uses only integer arithmetic, so
//! every replica and the offline simulator compute the same value.

// The generated canonical-ABI exports take the flattened job (12+ arguments).
#![allow(clippy::too_many_arguments)]

#[cfg(not(any(
    feature = "priority",
    feature = "admission",
    feature = "estimator",
    feature = "node-score"
)))]
compile_error!("enable one feature: priority, admission, estimator or node-score");

#[cfg(feature = "admission")]
mod admission;
#[cfg(feature = "estimator")]
mod estimator;
#[cfg(feature = "node-score")]
mod node_score;
#[cfg(feature = "priority")]
mod priority;
