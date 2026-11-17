# Plans & Architecture Decision Records

This directory contains planning documents and Architecture Decision Records (ADRs) for this repository.

## What is an ADR?

An **Architecture Decision Record** (ADR) documents a significant decision made about the architecture or design of the project — including the context, the options considered, the decision taken, and the consequences.

## File Naming Convention

```
NNNN-short-title-of-decision.md
```

Where `NNNN` is a zero-padded sequential number (e.g., `0001`, `0002`).

## ADR Template

Create a new file following the template below:

```markdown
# NNNN — Title of Decision

**Status**: Proposed | Accepted | Deprecated | Superseded by [NNNN](./NNNN-title.md)
**Date**: YYYY-MM-DD

## Context

What is the issue or requirement motivating this decision?

## Options Considered

1. **Option A** — brief description, pros/cons
2. **Option B** — brief description, pros/cons

## Decision

What was decided and why?

## Consequences

What are the resulting trade-offs, follow-up tasks, or risks?
```

## Index

| # | Title | Status | Date |
|---|-------|--------|------|
| [0001](./0001-replicated-controller-writes.md) | Route controller writes through Raft with a Proposer hook on the FSM | Accepted | 2026-11-17 |
| [0002](./0002-wasm-policy-plugins.md) | Sandboxed WASM plugins for scheduling policy | Accepted | 2026-11-17 |
| [0003](./0003-admission-plugins.md) | Admission plugins that can reject or rewrite jobs | Accepted | 2026-11-17 |
| [0004](./0004-node-score-plugins.md) | Node-score plugins choose where, never when | Accepted | 2026-11-17 |
