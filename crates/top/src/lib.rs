//! `gntop`: a `top`-like live Gantt chart of the cluster plan.
//!
//! Gnomon schedules on the time axis — every resource vertex carries a
//! planner of running spans and future backfill reservations (report
//! sections 4.2 and 5.2). `gntop` makes that axis visible: one row per node,
//! solid bars for running spans, hatched bars for reservations, a "now"
//! cursor, and the pending queue underneath.
//!
//! The crate is split so that everything but the poll loop is pure:
//! [`model`] (wire → client model), [`view`] (zoom/pan state machine),
//! [`layout`] (time→column mapping, lane stacking, labels, styles),
//! [`app`] (UI state and keys) and [`render`] (buffer drawing). [`poll`]
//! is the only module that touches the network; the binary owns the
//! terminal.

pub mod app;
pub mod layout;
pub mod model;
pub mod poll;
pub mod render;
pub mod view;
