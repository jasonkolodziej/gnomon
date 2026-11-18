# 0005 — A Rune web UI over a server-side bridge

**Status**: Accepted
**Date**: 2026-11-18

## Context

`gntop` shows the plan in a terminal, but most of what Gnomon now does has no view at all: the Raft
controller group (ADR 0001), the WASM policy plugins and their counters (ADR 0002-0004), the gang
kill channel, the event stream, and the policy comparisons in `gnomon-sim`. The request was a
polished web UI written in Rust, using [Rune](https://github.com/anomalyco/rune), in a terminal
style and the brand palette.

Rune is a GPU-rendered Rust UI framework. Its web target compiles the app to WebAssembly and draws
one canvas through WebGPU. It pins Rust 1.99.1 (edition 2024), asks apps to repeat its
`[patch.crates-io]` entries, and has no networking layer.

## Options Considered

1. **Browser speaks gRPC to the controllers (gRPC-web via `tonic-web`).** No extra process, but each
   tab is a gRPC client, the UI would need protobuf code under the Rune toolchain, and failover
   across replicas would live in the browser.
2. **A small server, `gnomon-web`, bridging gRPC to JSON and Server-Sent Events.** One gRPC client
   however many tabs are open. Failover, the long-poll and the lab simulations run in Rust on the
   server. The UI shares only serde types with it.
3. **Server-rendered HTML** instead of Rune. Simplest, but not what was asked for, and it does not
   draw a live Gantt chart as well as a GPU canvas.

## Decision

Option 2.

- **Read-only status RPCs.** Three were added to the API, all read-only:
  - `ClusterStatus`, from openraft's metrics.
  - `PolicyStatus`, the plugin hooks' existing atomic counters, now exported instead of only
    logged.
  - `ListEvents`, a long-poll over an opt-in, in-memory tail of applied events. Only the controller
    turns the tail on, so the simulator, replay and the throughput benchmark pay nothing.
- **Kill list.** `ListNodes` reports each node's ranks on the kill channel.
- **Change signal.** `gnomon-web` long-polls `ListEvents` as its change signal, refreshes a full
  snapshot when something changed (at most every 300 ms, at least every 2 s), and pushes both to
  browsers over SSE.
- **Separate workspace.** The UI lives in `ui/` as a workspace of its own on Rune's toolchain. The
  scheduler crates stay on stable and never see Rune's patches. The shared JSON types are
  `gnomon-web-model` (serde only).

## Consequences

- **Browser support.** WebGPU is required: current Chrome, Edge, Safari, or Firefox behind a flag.
  Rune's `webgl` fallback does not build at the pinned revision. The page says so when WebGPU is
  missing, and the JSON endpoints work regardless.
- **Build.** The UI build needs Rust 1.99.1, `wasm-bindgen-cli` 0.2.129 and Rune's build script
  (fetched at the pinned revision by `ui/build.sh`). CI builds it in its own job.
- **Security.** The UI can submit and cancel jobs, and the gRPC API has no authentication, so
  `gnomon-web` binds to localhost by default and has `--read-only`. Authentication is follow-up
  work, for the API and the UI together.
- **Bumping Rune.** Upgrading Rune means updating the revision in `ui/Cargo.toml` (and the patch
  list, if Rune's changes).
