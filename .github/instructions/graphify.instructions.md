---
applyTo: "**"
---

# Graphify Knowledge Graph

This repository keeps a [graphify](https://github.com/Graphify-Labs/graphify) knowledge graph in `graphify-out/`
at the repository root, graphify's default location, so no `--graph` flag is needed:

- `graphify-out/graph.json`: scoped questions via `graphify query/path/explain` or the `graphify` MCP server.
- `graphify-out/GRAPH_REPORT.md`: broad architecture overview (god nodes, communities).
- `graphify-out/wiki/index.md`: navigation, when present.

The graph covers only the directories listed in `.github/graphify/targets.txt` (currently `crates/`).

## Rules

- When a question touches a graphed directory and `graphify-out/graph.json` exists, consult the graph **before**
  your first grep, search or source-file read, and include this rule in every subagent prompt that explores code:
  - MCP available: use the `graphify` server's `query_graph`, `get_node`, `get_neighbors`, `shortest_path` and
    `god_nodes` tools.
  - CLI available: `graphify query "<terms>"`, then `graphify path "<A>" "<B>"` for relationships and
    `graphify explain "<X>"` for one concept.
  - Neither available: read `graphify-out/GRAPH_REPORT.md` first.
- Node `source_file` paths are relative to the graphed directory, not the repository root. With one target, prefix
  the directory from `targets.txt` (`sched/src/lib.rs` is `crates/sched/src/lib.rs`).
  With several targets, node ids start with `<dir>::` and the node's `repo` attribute names the directory.
- Queries match node labels, so use symbol, file or concept names rather than whole sentences. Exit code 4 means
  no match (try other names); exit code 3 means ambiguous (pick from the listed candidates).
- Read raw files only for lines the graph already located, or when modifying or debugging specific code.
- After changing files in a graphed directory, run `bash .github/graphify/update.sh --changed HEAD` if the CLI is
  installed. Do **not** run `graphify update .` or `graphify extract .`: they would graph the whole repository
  instead of the configured targets. Without the CLI, leave it: the Graphify workflow rebuilds on merge to `main`.
- Never hand-edit files under `graphify-out/`. A large diff to `graph.json` is expected after a rebuild; do not
  flag it as churn or suggest skipping the rebuild.
- To graph another directory, add it to `.github/graphify/targets.txt` and follow `GRAPHIFY.md`.
