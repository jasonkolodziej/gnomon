# The-AIOS/aios repository profile (as observed 2026-11-20)

Method note: the repository page was fetched via a browser-based fetcher (HTTP OK, no redirect), and the repo was
shallow-cloned read-only (`git clone --depth 200`, HEAD `1b136cbc` on `main`, dated 2026-11-17). The GitHub REST
API (`api.github.com/repos/The-AIOS/aios`) and a direct `curl` of the HTML page were refused by this session's
proxy (403, session policy, not a GitHub answer), so API-only metadata (exact contributor count, issue list,
release objects, language byte percentages) was not retrieved. File-level claims below come from the clone and
are cited to the matching `blob/main` URL. Nothing from the clone was executed.

## Is The-AIOS/aios the same codebase as agiresearch/AIOS, or a distinct project?

### Takeaway
It is an entirely different project that only shares the "AIOS" name. The-AIOS/aios is a Claude Code
"personal/team operating framework" (Markdown vault + slash commands + agent prompt files + hooks + bundled MCP
servers), maintained by Chuy Cepeda and a small contributor group; agiresearch/AIOS is the Rutgers-originated
Python "LLM Agent Operating System" kernel documented at docs.aios.foundation. There is no fork, mirror, rename
or shared code relationship.

### Cited Findings
- The repo exists, is public, and does not redirect; GitHub title: "The-AIOS/aios: Amplify yourself and your team – with AI co-workers." — [The-AIOS/aios](https://github.com/The-AIOS/aios)
- README headline: "The AIOS — AI as a Team, Not a Tool" and "Everyone is building an AIOS. We built **The AIOS**."; setup is by telling Claude Code "Set up my AI-OS from https://github.com/The-AIOS/aios" — [README.md](https://github.com/The-AIOS/aios/blob/main/README.md)
- README positions it against "second-brain wrappers with a CLAUDE.md and a few prompts" and describes itself as "governed (the INTENT.md trust contract), multiplayer (personal × team × company topologies), and substrate-agnostic"; architecture basis is "filesystem as context, plain Markdown, no RAG" — [README.md](https://github.com/The-AIOS/aios/blob/main/README.md)
- A case-insensitive search of the whole working tree for `agiresearch`, `aios.foundation` and `LLM Agent Operating System` returned zero files — [The-AIOS/aios tree](https://github.com/The-AIOS/aios)
- The plugin marketplace manifest names the owner/author as "Chuy Cepeda" with a sovra.io address, plugin `aios` v0.8.19, category "productivity", keywords `claude-code, obsidian, personal-os, knowledge-management, agentic-workflow, daily-rituals, second-brain, ai-coworker` — [.claude-plugin/marketplace.json](https://github.com/The-AIOS/aios/blob/main/.claude-plugin/marketplace.json)
- Commit authors in the fetched history (863 commits): chuycepeda 684, then "Matias"/"matiasmacera"/"Matías Macera" ~85 combined, jatres3 19, fernandezdiegoh 18, Luigi Matrone 17, and a handful of others — [commit history](https://github.com/The-AIOS/aios/commits/main)
- The org profile describes "a team of agents that works for you, with everyone, within your rules", a site at the-aios.com (EN/ES/PT) with an "Operating Manual", and lists 7 repositories including `aios` ("the framework, and the place to start"), `aios-app` (desktop app), `aios-glass` (editor layer), `company-template` and `site`; it states a business model of "always free... You pay only for what we run for you: servers, people, or verification" — [The-AIOS org](https://github.com/The-AIOS)
- By contrast agiresearch/AIOS describes itself as "AIOS: AI Agent Operating System", which "embeds large language model (LLM) into the operating system" and addresses "scheduling, context switch, memory management, storage management, tool management"; it consists of an AIOS kernel plus the Cerebrum SDK, launched as a uvicorn server (`runtime.launch:app`, port 8000), with LLM backends including OpenAI, Anthropic, Gemini, Groq, HuggingFace, ollama and vLLM, and cites "mei2025aios, AIOS: LLM Agent Operating System" — [agiresearch/AIOS](https://github.com/agiresearch/AIOS)
- agiresearch/AIOS shows 6.5k stars, 932 forks, 859 commits, and contains an "Experimental Rust Rewrite (aios-rs)" described as "NOT feature-parity yet" with trait definitions and placeholders including a `NoopScheduler` — [agiresearch/AIOS](https://github.com/agiresearch/AIOS)

### Inferences
- If the question that prompted this research was about the AIOS Foundation architecture at docs.aios.foundation, the intended repository is almost certainly `agiresearch/AIOS` (kernel) plus `agiresearch/Cerebrum` (SDK), not `The-AIOS/aios`. This is inference from name collision; nothing in either repo links to the other.
- The README line "Everyone is building an AIOS" suggests the maintainers are aware the name is crowded, but they do not name agiresearch.

### Gaps
- Exact contributor count and the GitHub "About" topics were not retrievable (API blocked; sidebar lists rendered empty in the fetch).
- The full text of the org README's repository list was only partially captured; the seventh repo names beyond `aios`, `aios-app`, `aios-glass`, `company-template`, `site` and the `.github` profile repo were not confirmed.

## What is its architecture, languages and dependencies?

### Takeaway
There is no kernel, daemon, runtime or model-serving layer. It is a content-and-scripts framework that rides
entirely on Claude Code: Markdown is the dominant "language", with Bash/PowerShell and stdlib-leaning Python
helper scripts and a few Node MCP servers. No Rust, no C, no gRPC, no protobuf.

### Cited Findings
- Tracked files by extension at HEAD (870 files): 411 `.md`, 180 `.sh`, 108 `.py`, 24 `.json`, 23 `.js`, 9 `.ps1`, 8 `.html`, 6 `.yml`; no `.rs`, `.c`, `.proto` or `Cargo.toml` — [The-AIOS/aios tree](https://github.com/The-AIOS/aios)
- Top-level layout: `agents/`, `hooks/`, `mcps/`, `plugins/`, `skills/`, `templates/`, `tests/`, `vault/`, plus large root docs (`CLAUDE.md` ~95 KB behavioural contract, `SETUP.md` ~92 KB, `CHANGELOG.md` ~650 KB, `FORTRESS.md`, `MODEL-ROUTING.md`, `SECURITY.md`, `INTENT.md`, `USER.md`, `TOOLS.md`) — [The-AIOS/aios tree](https://github.com/The-AIOS/aios)
- Agents are Markdown prompt files with frontmatter, grouped in 7 bundles (commerce, communication, engineering, finance-legal, personal, sales, strategy), e.g. `accountant.md`, `lawyer.md`, `code-reviewer.md`, `market-researcher.md`; operator and company agents live in `agents/custom/` and `agents/<company>/` — [agents/](https://github.com/The-AIOS/aios/tree/main/agents), [TOOLS.md](https://github.com/The-AIOS/aios/blob/main/TOOLS.md)
- The framework ships as a Claude Code plugin (`plugins/aios/commands`, 26 command files such as `/aios:today`, `/aios:close-day`, `/aios:close-session`, `/aios:company`, `/aios:collaborate`) and 34 skill folders under `skills/aios`, plus vendored `skills/anthropic` (Apache-2.0), `skills/superpowers` (MIT) and `skills/cloudflare` — [plugins/aios](https://github.com/The-AIOS/aios/tree/main/plugins/aios), [NOTICE](https://github.com/The-AIOS/aios/blob/main/NOTICE)
- "Memory" is a Markdown vault with three context layers (declared, observed, intent); observed context is written by Claude itself and routed by the daily ritual commands — [README.md](https://github.com/The-AIOS/aios/blob/main/README.md)
- "Tools" are bundled MCP servers: Google Workspace, Slack, Atlassian, Stitch, NotebookLM, Playwright (capability layer, not a server), Nano Banana (Gemini image), PDF generator, Spotify; Python MCPs require Python 3.10+ and versions are pinned by policy — [mcps/_index.md](https://github.com/The-AIOS/aios/blob/main/mcps/_index.md)
- Hooks are scripts wired into Claude Code events (`UserPromptSubmit`, `SessionStart`, `PostModelSwitch`, `statusLine`) or called from commands; grouped as `context/`, `vault/`, `pipeline/`, `agents/`, `media/`, `security/`, `install/`, `claude-identity/`, `git/` — [hooks/_index.md](https://github.com/The-AIOS/aios/blob/main/hooks/_index.md)
- `hooks/pipeline/pipeline-executor.py` (1,343 lines) is despite its name a data pre-loader: it batch-fetches Google Calendar, Tasks and Slack with a `ThreadPoolExecutor` before `/today` and `/close-day`, run via `uv run` with google-api-python-client and requests — [pipeline-executor.py](https://github.com/The-AIOS/aios/blob/main/hooks/pipeline/pipeline-executor.py)
- Runtime dependencies named in docs: Claude Code CLI (required), Python 3.10/3.11+, `uv`/`uvx`, Node (`npx`) for some MCPs, git, optional ffmpeg and mlx-whisper (macOS), headless Chrome/Chromium, pandoc; platforms macOS, Linux and Windows (PowerShell twins of the shell scripts) — [hooks/_index.md](https://github.com/The-AIOS/aios/blob/main/hooks/_index.md), [mcps/_index.md](https://github.com/The-AIOS/aios/blob/main/mcps/_index.md)

### Inferences
- The "operating system" in the name is metaphorical (an operating model for a person or team using Claude Code), not a systems-software claim. The real execution engine, tool loop, context management and subagent machinery are Claude Code's.

### Gaps
- `aios-app` and `aios-glass` (the desktop app and editor layer that "fulfil" spawn requests) are separate repos and were not inspected; their languages and internals are unknown here.

## Does it have a scheduler or task/agent orchestrator, and how does it work?

### Takeaway
No scheduler in the systems sense: no run queue, priorities, preemption, slot accounting or concurrency limits.
Orchestration is (a) prose guidance to the LLM about when to use one agent, parallel agents or a workflow, and
(b) a file-drop "agent bus" (`~/.aios/spawn-inbox/*.json`) that asks a running UI surface to open, message,
kill or resume a named Claude Code session. Timed work is delegated to the OS (launchd, systemd timers, Task
Scheduler, cron).

### Cited Findings
- The agent bus: "Agents coordinate across machines by writing files, not by opening ports." A JSON file in `~/.aios/spawn-inbox/` supports four verbs: `spawn` (`{"name","task"}`), `send`, `kill`, `resume`; the receiving machine "opens no port and runs no listener — it watches a directory it already owns" — [FORTRESS.md](https://github.com/The-AIOS/aios/blob/main/FORTRESS.md)
- Stated design invariant: "Zero inbound surface. If a future design needs a listening port on the agent host, find another way." — [FORTRESS.md](https://github.com/The-AIOS/aios/blob/main/FORTRESS.md)
- Stated limitation: "A request file carries no authentication and no requester identity... Treat the current bus as a single-operator convenience, not a multi-party protocol." — [FORTRESS.md](https://github.com/The-AIOS/aios/blob/main/FORTRESS.md)
- Spawn requests may carry an optional `"model"` id or `"tier"` rung; sessions are told to write a request file rather than run `spawn` directly; liveness and addressing come from Claude Code's session registry `~/.claude/sessions/*.json`; the fulfiller stamps a contract version (`aios-spawn-inbox: contract N`, currently 3) in `~/.aios/spawn-inbox/README.md` — [CLAUDE.md](https://github.com/The-AIOS/aios/blob/main/CLAUDE.md)
- The request is fulfilled "by whichever surface is running — AIOS Glass or the AIOS App"; with no live surface nothing consumes the inbox — [skills/aios/orchestration-ladder/SKILL.md](https://github.com/The-AIOS/aios/blob/main/skills/aios/orchestration-ladder/SKILL.md), [CLAUDE.md](https://github.com/The-AIOS/aios/blob/main/CLAUDE.md)
- The "orchestration ladder" skill is decision guidance for the model: single agent → parallel agents → dynamic workflow, plus a spawn-vs-subagent axis and model tier choice. It is prose, not code — [orchestration-ladder/SKILL.md](https://github.com/The-AIOS/aios/blob/main/skills/aios/orchestration-ladder/SKILL.md)
- `hooks/agents/team-plan.py` (287 lines) validates a project note's `## Team` plan (agents exist, known tiers, one chief of staff, reporting lines) and with `--approved` writes one spawn-inbox request per member; members report back with a `send` request — [team-plan.py](https://github.com/The-AIOS/aios/blob/main/hooks/agents/team-plan.py)
- The only concurrency control found is a spawn mutex: a `spawn.lock.d` directory lock with a noclobber pid file, polled every 0.5 s and reclaimed as stale after ~30 s, serialising terminal/tab creation rather than limiting how many workers run — [install-wrappers.sh](https://github.com/The-AIOS/aios/blob/main/hooks/claude-identity/install-wrappers.sh)
- Supporting observability: `bus-dead-letters.py` (169 lines) and `bus_log.py` (121 lines) for the bus; `aios-health.py` writes one health file per scheduled job in `~/.aios/health/` with a `valid_until` deadline so failed or stopped routines surface in `/today` — [hooks/_index.md](https://github.com/The-AIOS/aios/blob/main/hooks/_index.md)
- Scheduled work runs through OS schedulers: the quota watcher installs as launchd (macOS), systemd user timer (Linux) or Task Scheduler (Windows); FORTRESS describes "overnight shifts" and "scheduled cron agents" on an always-on second machine — [hooks/_index.md](https://github.com/The-AIOS/aios/blob/main/hooks/_index.md), [FORTRESS.md](https://github.com/The-AIOS/aios/blob/main/FORTRESS.md)

### Inferences
- The spawn-inbox is the only machine-readable seam where an external system could inject or control work: an external scheduler could act as a writer of request files (deciding when and with which tier a worker starts) or, in principle, as an alternative fulfiller watching the directory. Admission control, queueing and limits would all have to live in that external component, since the repo has none.
- Because the bus is unauthenticated, single-operator and explicitly forbids listeners, a gRPC or network scheduler integration would run against the project's stated invariant; a file-based adapter is the only fit consistent with its design.
- There is no notion of a schedulable unit with declared resource demand (no CPU/GPU/memory request, no duration estimate, no priority field); a request is just a name plus a prompt plus an optional model tier.

### Gaps
- The fulfiller side (how AIOS App / Glass consume the inbox, ordering, retries, limits) lives in other repos and in a README generated at runtime (`~/.aios/spawn-inbox/README.md`); not inspected.
- `bus-dead-letters.py` and `bus_log.py` were sized but not read line by line.

## How does it run LLM inference, and how are resources arbitrated?

### Takeaway
Inference is remote only, through the operator's Claude Code subscription/accounts (Anthropic models), with an
optional helper to call non-Claude models via OpenRouter or Gemini. There is no local model serving, no
GPU/VRAM/CPU management. The only "resources" managed are subscription rate-limit quota (by rotating between
Anthropic accounts) and cost (by a four-rung model tier ladder).

### Cited Findings
- "Every AIOS session runs on a Claude model, and every spawned worker inherits one"; the tier ladder maps `spawn --tier` values `frontier`, `judgment` (default), `scale`, `fast` to specific Claude models, with effort as a second knob; top and bottom of the ladder differ "roughly 22× in cost" — [MODEL-ROUTING.md](https://github.com/The-AIOS/aios/blob/main/MODEL-ROUTING.md)
- Tier resolution is a script (`hooks/pipeline/resolve-tier`) and literals in `install-wrappers.sh`/`.ps1`; a test (`tests/model-routing.test.sh`) fails the build if the doc table and wrappers disagree — [MODEL-ROUTING.md](https://github.com/The-AIOS/aios/blob/main/MODEL-ROUTING.md)
- Non-Claude models are reachable only via `hooks/pipeline/openrouter.py` (402 lines, stdlib-only, text in/text out), resolving `OPENROUTER_API_KEY` then `GEMINI_API_KEY`; used as a cheap text lane or independent judge — [hooks/_index.md](https://github.com/The-AIOS/aios/blob/main/hooks/_index.md), [MODEL-ROUTING.md](https://github.com/The-AIOS/aios/blob/main/MODEL-ROUTING.md)
- Quota management: `claude-identity/` is a "multi-account quota autopilot" that caches Claude Code's `rate_limits` via the statusLine process, watches it (every status refresh, rate-limited to once per 30 s, plus a 30-minute launchd safety net) and rotates Anthropic accounts when the 5h/7d limit nears a threshold; requires two or more Anthropic accounts; "One reading does not rotate" (two samples at least 30 s apart) — [claude-identity/README.md](https://github.com/The-AIOS/aios/blob/main/hooks/claude-identity/README.md), [hooks/_index.md](https://github.com/The-AIOS/aios/blob/main/hooks/_index.md)
- Model-switch auditing: a `PostModelSwitch` hook appends to `~/.aios/model-switch.jsonl` so a worker that finished on a different model is reported at session close — [hooks/_index.md](https://github.com/The-AIOS/aios/blob/main/hooks/_index.md)
- The only on-device model use found is optional macOS transcription (`ffmpeg → mlx-whisper large-v3-turbo`) and Apple Vision OCR for ingest, not agent inference — [hooks/_index.md](https://github.com/The-AIOS/aios/blob/main/hooks/_index.md)
- Searches of `README.md`, `FORTRESS.md`, `SETUP.md` and `MODEL-ROUTING.md` for `ollama`, `llama.cpp`, `gpu` and `vram` returned no hits — [The-AIOS/aios tree](https://github.com/The-AIOS/aios)

### Inferences
- Arbitration between concurrent agents is implicit: all workers draw on the same account's rate limit until the autopilot swaps accounts. There is no per-agent token budget, fair-share or priority mechanism in the repo.
- Multi-account rotation to stay under subscription rate limits is a design choice an adopter should check against the provider's terms; the repo presents it as a feature and this note makes no judgement on it.

### Gaps
- Exact rotation thresholds were not read (a "Threshold tuning" section exists in the claude-identity README).

## What roadmap items, issues or docs mention scheduling, resource management, multi-node or distributed operation?

### Takeaway
No roadmap for scheduling or cluster operation was found. The only multi-machine story is FORTRESS.md's
two-machine containment model (laptop plus an always-on, firewalled agent host) joined by the file bus.

### Cited Findings
- FORTRESS.md defines a "Two-Machine AI Agent Architecture": a main MacBook for interactive sessions and a 24/7 Mac mini hosting "remote-controlled Claude Code workers, overnight shifts, scheduled cron agents", isolated with pf firewall rules, SSH hardening and Tailscale for recovery, with one-way data flow (results leave via git push) — [FORTRESS.md](https://github.com/The-AIOS/aios/blob/main/FORTRESS.md)
- It explains why it keeps a file bus instead of Claude Code's native cross-session messaging: no cloud transit, no plan gate, pluggable transport (scp, synced folder, git repo, object store), no inbound surface — [FORTRESS.md](https://github.com/The-AIOS/aios/blob/main/FORTRESS.md)
- Signing of bus requests is named as missing and a precondition for sharing an inbox — [FORTRESS.md](https://github.com/The-AIOS/aios/blob/main/FORTRESS.md)
- "Multiplayer" in this project means sharing context and artifacts between people (`/aios:collaborate` over Drive, GitHub or local folders; `/aios:company` mounting company-distributed agents, skills, hooks and MCPs), not distributed compute — [README.md](https://github.com/The-AIOS/aios/blob/main/README.md)
- Searching `README.md`, `FORTRESS.md`, `CONTRIBUTING.md` and `TOOLS.md` for `roadmap`, `multi-node`, `cluster` and `distributed` produced only "company-distributed" (content distribution) hits and one CONTRIBUTING note that private roadmap keys must not appear in issues — [CONTRIBUTING.md](https://github.com/The-AIOS/aios/blob/main/CONTRIBUTING.md)

### Inferences
- The project's direction is governance, context and UX for human operators, not resource scheduling. A scheduler integration would be an external add-on, not something on its visible roadmap.

### Gaps
- GitHub issues and pull requests could not be listed (API blocked), so issue-tracker mentions of scheduling or resource management are unverified.
- The 650 KB CHANGELOG and the the-aios.com Operating Manual (21 sections) were not read in full.

## How active and mature is it?

### Takeaway
Small but very actively developed and young: about 120 stars and 28 forks, version 0.8.19, with several
hundred commits per month dominated by one author. It is pre-1.0 and changes almost daily.

### Cited Findings
- 120 stars, 4 watchers, 28 forks on 2026-11-20 — [The-AIOS/aios](https://github.com/The-AIOS/aios)
- Most recent commit on `main`: 2026-11-17 (author chuycepeda); latest CHANGELOG entries dated 2026-11-17, 11-14, 11-13, 11-12, 11-11 — [commit history](https://github.com/The-AIOS/aios/commits/main), [CHANGELOG.md](https://github.com/The-AIOS/aios/blob/main/CHANGELOG.md)
- Commits per month within the fetched window: 139 (Aug 2026, partial), 272 (Sep), 287 (Oct), 165 (Nov to the 17th) — [commit history](https://github.com/The-AIOS/aios/commits/main)
- 26 tags reachable in the shallow clone, from v0.5.0 (2026-08-12) to v0.8.19 (2026-11-17), i.e. roughly two tagged versions per week — [tags](https://github.com/The-AIOS/aios/tags)
- Licence: GPL-2.0-or-later for the framework, with vendored subtrees keeping upstream licences (Apache-2.0, MIT); a `LICENSE-AUDIT.md` documents the boundary — [NOTICE](https://github.com/The-AIOS/aios/blob/main/NOTICE), [LICENSE](https://github.com/The-AIOS/aios/blob/main/LICENSE)
- Engineering hygiene signals: a `tests/` directory of shell and Python tests, CI workflows `validate.yml` and `triage.yml`, issue templates, a pinned-dependency lint, `SECURITY.md` — [.github/workflows](https://github.com/The-AIOS/aios/tree/main/.github/workflows), [mcps/_index.md](https://github.com/The-AIOS/aios/blob/main/mcps/_index.md)
- Community channel is a Discord server linked from the README — [README.md](https://github.com/The-AIOS/aios/blob/main/README.md)

### Inferences
- The project predates the fetched window: `hooks/_index.md` frontmatter says `created: '2026-03-28'` and the earliest fetched commit is 2026-08-11, which is the shallow-clone boundary, so total commit count is higher than 863 and the true start date is earlier (probably early 2026).
- Bus-factor risk is high: about 79% of fetched commits are from one author.
- For comparison, agiresearch/AIOS is roughly 50 times larger by stars (6.5k) with a similar raw commit count (859).

### Gaps
- Total commit count, total tag/release count, whether GitHub Releases (as opposed to tags) are published, open issue/PR counts and contributor count were not obtained.

## Integration points where an external scheduler could plug in

### Takeaway
There are no APIs, plugin interfaces for execution, IPC sockets or gRPC services. The integration surfaces are
files and Claude Code extension points: the spawn-inbox JSON bus, the tier resolver, Claude Code hooks, and
MCP servers.

### Cited Findings
- Spawn-inbox request files (`~/.aios/spawn-inbox/<name>.json`, verbs spawn/send/kill/resume, optional `model`/`tier`, inbox path overridable via `AIOS_INBOX_DIR` in `team-plan.py`) — [FORTRESS.md](https://github.com/The-AIOS/aios/blob/main/FORTRESS.md), [team-plan.py](https://github.com/The-AIOS/aios/blob/main/hooks/agents/team-plan.py)
- State an external tool can read: Claude Code session registry `~/.claude/sessions/*.json` (name, pid, status, sessionId), `~/.claude/rate-limit-cache.json`, `~/.aios/health/*`, `~/.aios/model-switch.jsonl`, `~/.aios/accounts.json` — [CLAUDE.md](https://github.com/The-AIOS/aios/blob/main/CLAUDE.md), [claude-identity/README.md](https://github.com/The-AIOS/aios/blob/main/hooks/claude-identity/README.md), [hooks/_index.md](https://github.com/The-AIOS/aios/blob/main/hooks/_index.md)
- Extension namespaces that survive updates: `hooks/custom/`, `agents/custom/`, `skills/custom/`, `plugins/custom/`, `mcps/custom/`, and per-company namespaces — [hooks/_index.md](https://github.com/The-AIOS/aios/blob/main/hooks/_index.md), [TOOLS.md](https://github.com/The-AIOS/aios/blob/main/TOOLS.md)
- Claude Code event hooks are installed into `~/.claude/settings.json` by `install/install-settings.py` — [hooks/_index.md](https://github.com/The-AIOS/aios/blob/main/hooks/_index.md)

### Inferences
- A cluster/HPC-style scheduler has little to schedule here: the workload is interactive or cron-launched Claude Code sessions whose compute is remote API inference. The schedulable quantities are session count, account rate-limit headroom and model tier, none of which the framework exposes as a formal resource model.
- The agiresearch/AIOS kernel (with its explicit scheduler module and syscall-style LLM/memory/storage/tool queues) is the project where a scheduler-fit analysis is meaningful; The-AIOS/aios is relevant mainly as a name-collision to rule out.

### Gaps
- No documented stable contract for the spawn-inbox exists inside this repo beyond the prose in FORTRESS.md and CLAUDE.md; the authoritative protocol README is generated at runtime by the App/Glass.
