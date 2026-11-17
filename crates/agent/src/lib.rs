//! Node agent library: job launching, cgroup v2 confinement, privilege
//! separation, the gang release barrier and rootless container support
//! (report section 4.6), the kill channel that stops a rank when the
//! controller says so ([`kill`]), and controller failover ([`controllers`]).
//! The `gnomon-agent` binary wires these to the controller's gRPC API.

pub mod barrier;
pub mod cgroup;
pub mod container;
pub mod controllers;
pub mod identity;
pub mod kill;
pub mod launch;

pub use launch::{LaunchSpec, Launcher, Outcome};
