---
applyTo: "**"
---

# Graphify Knowledge Graph

This repository keeps a [graphify](https://github.com/Graphify-Labs/graphify) knowledge graph in `graphify-out/`
at the repository root, graphify's default location, so no `--graph` flag is needed:

- `graphify-out/graph.json`: scoped questions via `graphify query/path/explain` or the `graphify` MCP server.
- `graphify-out/GRAPH_REPORT.md`: broad architecture overview (god nodes, communities).
- `graphify-out/wiki/index.md`: navigation, when present.

The only target in `.github/graphify/targets.txt` is `.`: the whole repository minus the paths in `.graphifyignore`.
In practice that is the Gnomon Rust workspace (`crates/*/src`, including `build.rs` and `src/bin/*`) and the
`testkit/` shell and Python scripts. Not graphed: `docs/` (Markdown is skipped in code-only mode), `proto/`
(graphify has no `.proto` extractor; read `proto/gnomon.proto` directly), `results/`, `assets/`, `.github/`,
`.agents/` and `target/`.

## Rules

- When a question touches a graphed directory and `graphify-out/graph.json` exists, consult the graph **before**
  your first grep, search or source-file read, and include this rule in every subagent prompt that explores code:
  - MCP available: use the `graphify` server's `query_graph`, `get_node`, `get_neighbors`, `shortest_path` and
    `god_nodes` tools.
  - CLI available: `graphify query "<terms>"`, then `graphify path "<A>" "<B>"` for relationships and
    `graphify explain "<X>"` for one concept.
  - Neither available: read `graphify-out/GRAPH_REPORT.md` first.
- Node `source_file` paths are relative to the graphed directory. With the `.` target they are repository-root
  paths such as `crates/sched/src/lib.rs`. If more targets are added, node ids start with `<dir>::`, paths become
  relative to that directory, and the node's `repo` attribute names it.
- Queries match node labels, so use symbol, file or concept names rather than whole sentences. Exit code 4 means
  no match (try other names); exit code 3 means ambiguous (pick from the listed candidates).
- Rust specifics:
  - Crate directory `crates/<name>` is package `gnomon-<name>` and is imported as `gnomon_<name>`
    (`crates/fsm` → `gnomon-fsm` → `gnomon_fsm::Fsm`).
  - Types, traits and free functions are labelled by name (`ResourceGraph`, `backfill_pass()`); methods are
    labelled `.name()` (`.earliest_fit()`), so look a method up via its type with `get_neighbors` or
    `graphify explain "<Type>"` when the bare name is ambiguous.
  - Nodes without a `source_file` are external types (`std`, `tokio`, `openraft`, `wasmtime`, ...), not project code.
  - The gRPC types in `gnomon_api::pb` are generated from `proto/gnomon.proto` at build time and are not in the
    graph.
- Read raw files only for lines the graph already located, or when modifying or debugging specific code.
- After changing files in a graphed directory, run `bash .github/graphify/update.sh --changed HEAD` if the CLI is
  installed. Do **not** run `graphify update .` or `graphify extract .`: they would graph the whole repository
  while bypassing `update.sh` (which applies the target modes, the force guard and the report rebuild). Without the CLI, leave it: the Graphify workflow rebuilds on merge to `main`.
- Never hand-edit files under `graphify-out/`. A large diff to `graph.json` is expected after a rebuild; do not
  flag it as churn or suggest skipping the rebuild.
- To graph another directory, add it to `.github/graphify/targets.txt` and follow `GRAPHIFY.md`.
