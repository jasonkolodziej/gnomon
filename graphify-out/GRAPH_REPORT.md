# Graph Report - git-repo-template  (2026-11-12)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 114 nodes · 156 edges · 13 communities (8 shown, 5 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `4b5c2452`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4
- Community 5
- Community 6
- Community 7
- Community 8
- Community 9
- Community 10
- Community 11
- Community 12

## God Nodes (most connected - your core abstractions)
1. `compilerOptions` - 19 edges
2. `scripts` - 10 edges
3. `handleSiteverify()` - 7 edges
4. `vitest` - 7 edges
5. `SiteverifyError` - 6 edges
6. `Env` - 6 edges
7. `ErrorCode` - 5 edges
8. `fetch()` - 5 edges
9. `jsonResponse()` - 5 edges
10. `validate()` - 5 edges

## Surprising Connections (you probably didn't know these)
- `LogContext` --references--> `ErrorCode`  [EXTRACTED]
  turnstile-spin/templates/worker/src/observability.ts → turnstile-spin/templates/worker/src/types.ts
- `parseBody()` --calls--> `SiteverifyError`  [EXTRACTED]
  turnstile-spin/templates/worker/src/index.ts → turnstile-spin/templates/worker/src/errors.ts
- `validate()` --calls--> `SiteverifyError`  [EXTRACTED]
  turnstile-spin/templates/worker/src/validate.ts → turnstile-spin/templates/worker/src/errors.ts
- `ValidateResult` --references--> `SiteverifyResponse`  [EXTRACTED]
  turnstile-spin/templates/worker/src/validate.ts → turnstile-spin/templates/worker/src/types.ts
- `handleSiteverify()` --calls--> `errorResponse()`  [EXTRACTED]
  turnstile-spin/templates/worker/src/index.ts → turnstile-spin/templates/worker/src/errors.ts

## Import Cycles
- None detected.

## Communities (13 total, 5 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.10
Nodes (18): @cloudflare/workers-types, typescript, vitest, wrangler, description, devDependencies, @cloudflare/workers-types@^4.20240502.0, typescript@^5.4.0 (+10 more)

### Community 1 - "Community 1"
Cohesion: 0.09
Nodes (21): compilerOptions, allowJs, esModuleInterop, forceConsistentCasingInFileNames, isolatedModules, lib, module, moduleResolution (+13 more)

### Community 2 - "Community 2"
Cohesion: 0.25
Nodes (15): errorResponse(), SiteverifyError, corsHeaders(), fetch(), handleHealth(), handleSiteverify(), jsonResponse(), parseBody() (+7 more)

### Community 3 - "Community 3"
Cohesion: 0.17
Nodes (8): WORKER_VERSION, LogContext, Env, ErrorCode, SiteverifyLogEntry, WorkerMetadata, TEST_ENV, ENV

### Community 4 - "Community 4"
Cohesion: 0.20
Nodes (10): scripts, deploy, dev, lint, test, test:deploy, test:integration, test:validation (+2 more)

### Community 5 - "Community 5"
Cohesion: 0.33
Nodes (5): interface, display_name, icon_large, icon_small, short_description

### Community 6 - "Community 6"
Cohesion: 0.83
Nodes (3): deploy(), set_secret(), worker-deploy.sh script

### Community 7 - "Community 7"
Cohesion: 0.50
Nodes (4): repository, directory, type, url

## Knowledge Gaps
- **5 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `vitest` connect `Community 0` to `Community 3`?**
  _High betweenness centrality (0.221) - this node is a cross-community bridge._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.10276679841897234 - nodes in this community are weakly interconnected._
- **Why does `scripts` connect `Community 4` to `Community 0`?**
  _High betweenness centrality (0.092) - this node is a cross-community bridge._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.09090909090909091 - nodes in this community are weakly interconnected._