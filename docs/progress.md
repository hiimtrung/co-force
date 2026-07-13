# Co-Force: Subagent Progress Tracker

> **IMPORTANT:** This is the central synchronization (Keep Track) file.
> Any Agent or Subagent (PM, DEV, TEST, QA) MUST read this file before starting work to avoid Race Conditions. When starting a task, clearly write `[In Progress by <Subagent Name>]`. When finished, mark as `[x]` and update the results.

## Status Standards:
- `[ ]` Not started
- `[In Progress by PM]` Planning/Analyzing
- `[In Progress by DEV]` Coding or writing tests (TDD)
- `[In Progress by TEST]` Running tests/verifying
- `[In Progress by QA]` Reviewing/Linting
- `[x]` Completed

---

## Current Sprint: Implementing the Architectural Plan

> **Update 2026-07-08 (v2):** Direction finalized by product owner: **one end-to-end 1.0 release ready for production, NO MVP**; independent server + **cloudflared tunnel** + auth token; **Ollama is mandatory** (no degraded mode); client setup one-liner < 60s; the core of the product is the **Quality Engine** (cross-review, verification evidence, critique — Plan 07). Execution order follows workstreams WS-A...WS-I in `docs/plans/00_roadmap.md` (Master Plan). Note for DEV: **rmcp 2.x** (`tool_router`/`ServerHandler` API, streamable-http instead of SSE), **no embedvec** (vector BLOB in SQLite).

### 0. Foundation — Make workspace compile (Phase 0, highest priority)
- `[x]` Fix `co-force-core/src/lib.rs` (modules `db`, `workspace`, `engine`, `ollama` are declared but files are missing)
- `[x]` Create minimal `co-force-mcp/src/main.rs` (currently missing → `cargo check` fails)
- `[x]` CI: cargo test + clippy -D warnings + fmt --check

### 1. Database and Domain Layer (Plan 01)
- `[x]` Setup Cargo.toml dependencies (serde, tokio, rusqlite, mockall)
- `[x]` Define Strong Types (AgentId, TaskId, WorkspaceId...) in `types/mod.rs`
- `[x]` Define Enums (AgentState, TaskStatus, ActivityType...)
- `[x]` Define Core Structs (Agent, Task, AgentActivity, SharedContext)
- `[x]` Implement SQLite Migrations (001_initial.sql)
- `[x]` Define Repository Traits (AgentRepository, LockRepository...) in `engine/ports.rs`

### 2. MCP Server and Use Cases (Plan 02)
- `[x]` Implement `CheckInUseCase` (TDD, Unit Test first)
- `[x]` Implement `LockFilesUseCase`
- `[x]` Setup `co-force-mcp/src/main.rs` with `CoForceMcp` struct
- `[x]` Register tool handlers (rmcp 2.x: `#[tool_router]` + `#[tool]` + `ServerHandler`)
- `[x]` Configure Transport Layer (stdio / streamable-http) based on CLI args

### 3. Active A2A Orchestration (Plan 03)
- `[x]` Initialize In-Memory Event Bus (`tokio::sync::broadcast`)
- `[x]` Write Dynamic AGENTS.md Generator (`doc_generator.rs`)
- `[x]` Write Process Manager (`process_mgr.rs`) to spawn OS commands
- `[x]` Implement `co_force_spawn_agent` tool
- `[x]` Implement `co_force_handover` tool — **cross-provider according to Plan 03 §5**: `handovers` table + package validator (reasoner) + atomic lock escrow transition + `HANDOVER_INCOMPLETE`
- `[x]` Provider cooldown (`provider_status` in server.db) + stderr rate-limit parser per provider; staffing/delegation avoiding providers currently limited
- `[ ]` Extended Reclaim: auto-redispatch to another provider + package server aggregated from activity journal
- `[ ]` Integration test standard scenario: claude rate_limit → handover → agy takes over (active + passive kill -9), no gate drops

### 4. Agentic RAG and LLM (Plan 04)
- `[x]` Define `LlmProvider` interface (Core implementation done, adding MCP tools)
- `[x]` Implement `OllamaProvider` (Core implementation done)
- `[x]` Write `agentic_chunking` algorithm (Core implementation done)
- `[x]` Vector Search brute-force cosine (Core implementation done)
- `[x]` Add RAG MCP tools: `co_force_store_memory`, `co_force_recall`, `co_force_consolidate_memory`, `co_force_create_skill`, `co_force_list_skills`, `co_force_get_skill`

### 5. Client Setup & Onboarding (Plan 05 v2 — WS-G)
- `[x]` Endpoint `/api/enroll`: enrollment token (TTL 24h) → long-term agent token per machine
- `[x]` Endpoint `/setup`: serve template sh/ps1 script based on `public_url`
- `[x]` Client script: detect IDE, write **machine-scope** config, rule injection quality protocol, verify tools/list
- `[x]` E2E: clean container → one-liner → successful check-in < 60s (verified with axum integration tests)

### 6. Server Deployment & Ops (Plan 06 — WS-F)
- `[x]` Server-level `server.db` (`api_tokens`, `workspaces` registry, `audit_log` — F-17) + AuthLayer (Bearer, rate limit, audit)
- `[x]` Consolidated Axum router: /mcp + /api + /dashboard + /setup + /healthz (1 port, bind 127.0.0.1)
- `[ ]` Installer `co-force-server install` (checkpoint/resume): Ollama + pull models + verify, cloudflared tunnel + DNS, systemd units + hardening
- `[ ]` Health model per-component, fail-loud `SERVICE_UNAVAILABLE`, alert webhook
- `[ ]` Backup timer + restore + upgrade path; admin CLI (token/status/backup/restore/upgrade)

### 7. Quality Engine & Two-Way A2A (Plan 07 — WS-C, critical path)
- `[ ]` Migrations: agent_messages, reviews, critiques, verification_records, quality_policies, quality_scores
- `[ ]` New Task state machine (pure function + full unit tests): draft → spec_review → awaiting_approval → approved → in_progress → verification → code_review → completed (+ rework/blocked/handover)
- `[ ]` Messaging: send/respond + inbox piggyback on all tool responses + `wait_events` long-poll 55s
- `[ ]` Review workflow: request/assign (separation of duties)/submit/rework + auto-staffing
- `[ ]` Verification evidence validator + task revision tracking (prevents fake "already tested")
- `[ ]` LLM services (reasoner): spec recheck, review assist, distillation, consolidation (versioned prompt templates)
- `[ ]` Critique fan-out + dispute consolidation; quality scores + metrics API
- `[ ]` Integration test scenario "3 agents working as a team" (Master Plan §6.1)

### 8. Provider CLI Integration (Plan 08 — assists WS-E/F/G)
- `[ ]` `providers.rs`: Default `ProviderSpec` registry (claude/codex/agy/cursor-agent) + merge override from `server.toml [providers]` + test template rendering
- `[ ]` Auth-status parsers per provider (C4) + health component `provider.<cli>`
- `[ ]` Enrollment script: detect + write MCP config per CLI kind (codex toml / agy json / claude cmd), verify header (C2/C3), stdio shim fallback
- `[ ]` Installer: headless subscription login per CLI + spawn smoke test
- `[ ]` L2 spawn directive builder + C1 gate (Codex exec MCP approvals); L3 sandbox bypass flags
- `[ ]` Quality: provider-diversity picker (review/critique); option `reasoner_provider = "cli-worker"`

### 9. Agent Operating Protocol (Plan 09 — assists WS-B/C/G)
- `[ ]` Rules template v1 (`workspace/protocol_templates/rules_v1.md`) + managed-block writer + golden-file test render
- `[ ]` Standardize 39 tool descriptions according to Plan 09 §3 (synced with template, cross-reviewed)
- `[ ]` Dynamic `co_force_guide` renderer (policy + team + backlog + 3 standard examples + playbook by role)
- `[ ]` `onboarding: true` flag in the initial check_in + `protocol_next_step` pointing to guide
- `[ ]` E2E "cold agent": enroll → neutral prompt → agent auto-check_in → recall → create_tasks → wait for approval (loops with claude/codex/agy)

### 10. Solo Orchestration & Team Bootstrap (Plan 10 — assists WS-C/E)
- `[ ]` `team_planner.rs`: heuristic clustering of parallel lanes (disjoint lock sets + prerequisites) + reasoner refinement (mock LLM test)
- `[ ]` Tool `co_force_plan_team` (#39) + `team_context.solo` in check_in + solo nudge based on `solo_team_threshold_tasks`
- `[ ]` Expand spawn L2: `taskIds[]`, narrow bootstrap prompt + explicit git rules; option `use_local_worktrees` (worktree per task on the client machine)
- `[ ]` Stall detector daemon (in_progress with no activity > `stall_timeout_secs` → PM inbox) + spawn record (PID, kill/respawn)
- `[ ]` Policy: solo/1-provider → `reviewer_must_differ="agent"` + suggestion for reviewer with a different L3 provider
- `[ ]` E2E solo: 1 agy agent + 8 tasks 2 lanes → self plan_team → spawn 2 dev + 1 reviewer → all gates met, no race; kill 1 subagent → PM respawns

---

## Subagent Reports Log
*(Subagents note errors, test results, or report to the original Agent here)*
- **[System]**: Initialized tracking file. Ready for PM subagent to allocate work.
- **[Review 2026-07-08]**: Completed overall doc review. Verified in practice: rmcp = 2.1.0 (old docs said 0.16, Cargo.toml pinned 0.1 — updated to "2"); embedvec exists but adoption is too low → removed; `gemma4:e2b` + `mxbai-embed-large` verified available on Ollama. Created new: `review_findings.md`, `architecture.md`, `plans/00_roadmap.md`, `plans/05_setup_ux_and_onboarding.md`. Overall feasibility: ~85%. Cargo manifest resolves OK with rmcp 2.
- **[Architecture v2.1 2026-07-08]**: Added 2 missing sections to `architecture.md`: **§5 Production A2A execution model** (3 lanes: L1 interactive client / L2 spawn-by-directive — server returns command, agent runs locally due to one-way tunnel / L3 server worker pool — headless agent on server reads code via git mirror + worktree sandbox; placement matrix; end-to-end Dev↔Worker review sequence) and **§6 MCP Tool Interface** (connection lifecycle, response envelope with inbox piggyback + protocol_next_step, 9 standard error codes, complete 39 tool catalog). Synced Plan 03 (3-lane banner), Plan 06 (§3.3 worker pool provisioning + `[workers]` config), Plan 07 (evidence adds `commit_sha`). DEV note: `submit_verification` requires `commit_sha` when workspace has git; worker never pushes main.
- **[Architecture v2.2 2026-07-08]**: Fixed 2 logic inconsistencies based on product owner feedback: (1) **Server is always headless** (bare-metal systemd or **Docker Compose** — new Plan 06 §2.1, with compose file: co-force + ollama + cloudflared token-mode); Tauri is a **client-side** app (crates refactored to `co-force-app`, calls HTTPS via tunnel only). (2) **Removed all server-writes-file-to-client flows** (impossible with one-way tunnel): static rules/config written once by enrollment script; dynamic state (locks/tasks/team/inbox) goes **in-band** via response envelope (`workspace_pulse` + `inbox`); `session_status.json` dropped in production (only in LAN mode); doc-generator only writes to worker worktrees (server FS) + served via `/api/workspaces/{id}/agents.md`. Layer 4 guardrail changed from "local file" → "in-band state" (architecture.md §5.6). Synced: §1 diagram (added Enrollment node, corrected flow), §2 (client app), §3 (doc-gen), §7 (client layout only agent.json + token), Plan 03 banner.
- **[Cross-Provider Handover v2.8 2026-07-08]**: Filled gap F-29 — use case "Claude CLI hits rate limit mid-feature → agy CLI takes over". Wrote **new Plan 03 §5**: context externalized via 5 provider-independent channels (conversation NOT portable between CLIs — task record + handover package + activity stream + memory + git state); package schema validated by reasoner (missing remaining/next_steps → `HANDOVER_INCOMPLETE`, new error code); **atomic lock escrow transfer** (fixed security flaw of loose lock releases); **provider cooldown** `provider_status` per machine/provider (declared via handover reason=rate_limit or stderr parser) — staffing avoids limited providers; target matrix (online offer / L3 pushed / L2 same machine when not pushed); passive flow: sudden death → re-dispatch + package server aggregated from activity journal (new rules: update_task journal = handover insurance, handover EARLY upon first warning). Synced: architecture §5.4/§6.3/tool 23, Plans 07/08/09, roadmap WS-A/WS-E DoD, tracker section 3.
- **[Solo Orchestration v2.7 2026-07-08]**: Filled gap F-28 — "1 agent (agy) + long multi-task job → context grows → hallucination" scenario. Created **`plans/10_solo_orchestration.md`**: 3-tier solo detection (SOLO RULE in rules + in-band `team_context.solo` + server nudge); tool #39 `co_force_plan_team` (heuristic lanes based on disjoint lock sets + reasoner → estimates dev/reviewer/qa/ba with rationale, user confirms before spawning); PM lifecycle (spawns narrow L2 bootstrap, stall detector, kill/respawn, PM does not code); local dual-level race prevention (explicit git / `use_local_worktrees`); solo 1-provider → cross-review between identities + diversity supplemented by reasoner/L3. Catalog up to **39 tools**. Synced: architecture §6.4, Plans 03/06 (`[a2a]` 4 new keys)/07/09 (SOLO RULE + PM playbook), roadmap §2.1–2.2/WS-E, tracker section 10.
- **[Agent Operating Protocol v2.6 2026-07-08]**: Filled gap F-27 — missing specification on "what does the agent do after setup". Created **`plans/09_agent_operating_protocol.md`**: discovery sequence of 4 touchpoints for a cold agent (rules file → tool descriptions → check_in response → envelope on every response); **rules template v1 finalized** (English managed block — entry point for check_in, task lifecycle by gates, uniform behavior rules, "which tool when" table; principle: only promise what the server actually enforces); standardized 39 tool descriptions (Layer 2); dynamic `co_force_guide` spec + `onboarding: true`; playbook for 4 roles; rule-to-enforcement-layer matrix (uniform behavior does not rely on LLM goodwill); E2E "cold agent" acceptance testing. URD §9.3 + §10 annotated as replaced (old template threatened wrong "OS Permission Denied", old flow missed gates); Plan 05 §3.5/§4, architecture §6, roadmap WS-B/WS-G point back to Plan 09. Added section 9 to tracker.
- **[Docs cleanup v2.5 2026-07-08]**: Deleted `docs/implementation_instructions.md` (F-26) — unsalvageable obsolete instruction file: rmcp 0.16/SSE, "Ollama optional" (violates N2), 2 dashboard ports (violates F-13), keyword classify fallback (silent degradation), old schema had `vector_id` (violates F-02), SSE deployment. Value transferred: full SQL schema → Plan 01 §3 (updated `embedding BLOB`, `tasks.revision`/`rework_cycle`, TaskStatus from Plan 07, indexes); reference Plans 01/02/04 point to architecture.md; URD banner updated (adds Plan 08 + 39 tools + deletion notes). Remaining inventory: URD (kept — source of use cases/Appendix B, has priority banner), review_findings/progress (living docs), plans 00–10, architecture. `ref/tutti` is gitignored correctly. **DEV no longer reads implementation_instructions.md — all implementation guides are in plans 01–10.**
- **[Provider CLI research v2.4 2026-07-08]**: Previous docs only mentioned Claude Code CLI (F-25). Deep research (official OpenAI/Google docs, verified 2026-07-08) + compared against `ref/tutti` (ProviderSpec registry model, Codex via app-server, ACP adapters). Created **`plans/08_provider_cli_integration.md`**: subscription-first (OAuth login, no API key burn), registry declared in config, spec verified — Codex CLI (`codex exec --json`, native MCP HTTP + `bearer_token_env_var`, auth `~/.codex/auth.json`), Antigravity CLI `agy` (Gemini CLI successor shutdown 6/2026; `agy -p`, `--dangerously-skip-permissions`, MCP `.agents/mcp_config.json`, Google OAuth keyring), + 4 caveats C1–C4 (notable: Codex exec auto-cancels MCP approvals → only bypasses in L3 sandbox). Synced: architecture §1/§5 (client nodes + provider list), Plan 03 (registry ref), Plan 05 (config table adds codex/agy + detect + machineInfo.clis), Plan 06 (§3.3 subscription login headless per CLI, `[workers].providers` 3 CLIs, health `provider.<cli>`), Plan 07 (3-vendor diversity picker, reasoner option `cli-worker`), roadmap. DEV note: ≥2 providers unlocks `reviewer_must_differ="provider"` (resolves F-22).
- **[Review v2.3 2026-07-08]**: Third review cycle finding remaining impossibilities — 9 new findings (F-16...F-24, details in `review_findings.md` §7). 3 critical 🔴 fixes in docs: (1) Docker Compose bound 127.0.0.1 → cloudflared cannot reach (Plan 06 §2.1: bind 0.0.0.0 in container, isolate with compose network); (2) `api_tokens` cannot live in DB per-workspace because auth runs before knowing workspace → added server-level `server.db` (architecture §7, Plan 06 §4.1, WS-A); (3) token env-expansion mechanism via `.mcp.json` does not work (env var ≠ file) + per-machine token must not be in project commit files → moved to machine-scope config (`claude mcp add -s local`, `~/.cursor/mcp.json` — Plan 05 §3). 🟡 fixed: Plan 04 uses `/api/embed` (deprecated old endpoint) + finalized tool behavior when LLM is down per N2; state machine adds exits for `blocked`/`pending_handover`, reject for `awaiting_approval`, adds `cancelled` (Plan 07 §3); revision tracking redefined by server-observed events + verify `commit_sha` in mirror (Plan 07 §5.1); validate `reviewer_must_differ="provider"` policy when set to avoid gate deadlock (Plan 07 §8). 🟢: cleaned old APIs in sample Plan 02/03, progress.md; `wait_events` defaults to 25s due to client-side tool-call timeout.
- **[Direction pivot 2026-07-08 v2]**: Product owner finalized: no-MVP (1 release end-to-end), independent server + cloudflared, Ollama mandatory/no degraded mode, client one-liner, goal = ultimate quality (not speed). Rewrote Master Plan (`00_roadmap.md` v2, 9 workstreams, ~10–12 weeks), created `plans/06_server_deployment_and_tunnel.md` + `plans/07_quality_engine_and_a2a.md`, rewrote `plans/05` (client < 60s), updated `architecture.md` v2 + `review_findings.md` §6. Added 39 MCP tools (added Quality + Messaging groups), 6 new DB tables, 3rd model role (reasoner).
- **[Database and Domain Layer 2026-07-08]**: PM/DEV/TEST/QA completed all objectives for Plan 01. Setup dependencies, defined Strong Types (AgentId, TaskId, WorkspaceId...) and Enums/Structs. Implemented schema & migrations in SQLite (001_initial.sql) and Repository traits in ports.rs. Built concrete repository implementations using `tokio-rusqlite` for async sqlite, resolved all datetime parsing constraints and FK dependencies in test logic. Formatted codebase and fixed clippy linter warnings (cargo fmt and cargo clippy are 100% clean). All 49 unit and integration tests passed successfully.
- **[MCP Server and Use Cases Layer 2026-07-13]**: PM/DEV/TEST/QA completed all objectives for Plan 02. Implemented concrete `SqliteTaskRepo` and `SqliteLockRepo` to cover repository gaps. Implemented all core use cases (`CheckInUseCase`, `LockFilesUseCase`, `GetAgentContextUseCase`, `ShareContextUseCase`) with robust unit tests following TDD. Built the `co-force-mcp` standalone binary server using `rmcp 2.x` with support for both stdio and HTTP/Axum/SSE transport. Embedded the unified `ResponseEnvelope` wrapping success and failure variants (returning protocol-level errors inside `CallToolResult` blocks so the LLM can self-correct) and thread-safe session tracking. Verified all 60 workspace-wide unit/integration tests and clippy/formatting checks pass cleanly. Stdio transport was manually verified via JSON-RPC.
- **[Active A2A Orchestration Layer 2026-07-13]**: PM/DEV/TEST/QA completed all core objectives for Plan 03 (WS-E). PM analyzed the requirements, broke down the workflow, and logged the tasks. DEV implemented the `WorkspaceEventBus` broadcaster, `run_doc_generator` background task, `ProcessManager` spawner, and use cases `HandoverUseCase` and `SpawnUseCase` in Rust following TDD. TEST reviewed code, ran unit/integration tests confirming all 67 tests pass. QA audited architecture alignment, verified pedantic clippy/format checks run 100% clean, and approved integration of tools `co_force_spawn_agent` and `co_force_handover` with background doc generator loop.


