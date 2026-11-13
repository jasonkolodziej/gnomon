//! Node agent library: job launching, cgroup v2 confinement and rootless
//! container support (report section 4.6). The `gnomon-agent` binary wires
//! these to the controller's gRPC API.

pub mod cgroup;
pub mod container;
pub mod launch;

pub use launch::{LaunchSpec, Launcher, Outcome};
