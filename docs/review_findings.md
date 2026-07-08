# Co-Force — Overall Document Review Report & Feasibility Assessment

**Review Date:** 2026-07-08  
**Scope:** `URD.md`, `implementation_instructions.md` (now deleted), `docs/plans/01–04`, `progress.md`, `AGENTS.md`, codebase state in `crates/`  
**Methodology:** Cross-referencing between documents + real-world verification on crates.io and the Ollama registry (no evaluation based on memory).

---

## 1. Executive Summary

| Category | Assessment |
| :--- | :--- |
| URD Quality | 🟢 Extremely detailed, comprehensive use cases, serious risk analysis |
| Inter-document Consistency | 🟡 5 major contradictions resolved (see §2) |
| Overall Feasibility | 🟢 **~85%** (increased from 82% in URD — thanks to `rmcp` reaching 2.x stable) |
| Codebase State | 🔴 Skeleton does not compile (lib.rs declares non-existent modules, `co-force-mcp` lacks `main.rs`) |
| Setup Experience (original design) | 🟡 Too heavy for initial run (requires Ollama + 2 models + wizard) → shifted to "zero-config first run" (see Plan 05) |

**Primary Recommendation:** Align the 1.0 release scope strictly with the "Sweet Spot" identified in URD §11.3 (1 user, 2–3 agents, 1 machine) — ensuring it runs **without complex initial setup**, with a client onboarding time of < 60s. RAG and A2A spawning are critical workstreams, detailed in the Master Plan (`docs/plans/00_roadmap.md`).

---

## 2. Key Findings (resolved prior to coding)

### F-01 🔴 `rmcp` Version & API Drift — Multi-source discrepancy

| Source | Recorded |
| :--- | :--- |
| `URD.md` §3.3 | `rmcp = "0.16"`, features `["server", "transport-sse"]`, macro `#[rmcp::server]` |
| Root `Cargo.toml` | `rmcp = "0.1"`, features `["server", "transport-io"]` |
| **Real crates.io (verified 2026-07-08)** | **`rmcp = "2.1.0"`** (stable, updated 2026-07-02, ~15M downloads) |

Key Consequences:
1. **The `transport-sse` feature no longer exists** in rmcp 2.x. The MCP spec has deprecated SSE transport; replaced by **Streamable HTTP** — correct feature is `transport-streamable-http-server` (+ `transport-streamable-http-server-session` for session binding). Stdio uses `transport-io`.
2. The `#[rmcp::server]` macro is not part of the real API. The rmcp 2.x API utilizes `#[tool_router]` on `impl`, `#[tool(...)]` on each method, implementing the `ServerHandler` trait, and parameters via structs deriving `schemars::JsonSchema`.
3. **Risk R-01 (breaking changes pre-1.0) in URD §13 is obsolete in a positive direction**: rmcp has passed 1.0 → lowered risk from 🔴 to 🟢.

**Action:** Root `Cargo.toml` has been updated to `rmcp = "2"` with correct features. Sample code in Plan 02 has been updated to align with the `tool_router` API (update notes added).

### F-02 🔴 Vector DB `embedvec` — Exists but adoption is too low

Verified: `embedvec 0.8.0` is on crates.io but has only **~1.3k downloads** (compared to `sqlite-vec` ~1.9M, `hnsw_rs` ~620k). Building long-term memory infrastructure on a crate with virtually no adoption is an unnecessary risk.

**Resolution (simplifying both setup and code):**
- **1.0 Release:** Embeddings are stored as a BLOB directly in SQLite (`memory_entries`), searched using **brute-force cosine similarity** in Rust. Given the realistic scale of workspace memory (several thousand entries × 1024d), brute-force calculations take < 10ms — fully sufficient, **requires no additional storage engines**, and eliminates index file corruption risks (removing UC-30).
- **When > ~50k entries:** upgrade to `sqlite-vec` (same SQLite file, no architectural change) or `hnsw_rs`.
- Abstracted behind the `VectorSearch` trait so upgrades do not affect use cases.

### F-03 🟡 Conflicting Vector Store Storage Locations

- URD §5.2: `<project>/.co-force/memory/embedvec-index/`
- URD §6.2: `~/.co-force/vector_store/{workspaceId}/`

**Resolution (per central server model):** All server states (SQLite, vectors) reside on the **server host**: `~/.co-force/data/{workspaceId}/co-force.db` (vectors stored in the same DB file per F-02). `<project>/.co-force/` only contains client-side files: `agent.json`, `session_status.json`, auto-generated `AGENTS.md` — all git-ignored.

### F-04 🟡 OS-level `chmod -w` Workspace Enforcement (Layer 4 / UC-04 Reclaim) — High UX risk

Issues with the "set all files to Read-only, open only when locked" design:
1. Breaks build tools, git (`.git/index`), formatters, and IDE watchers — legitimate processes that do not pass through MCP.
2. The agent runs as the **same OS user** → can run `chmod +w` itself; the barrier acts as an advisory warning, not true security.
3. Complex cross-platform behavior (Windows ACL ≠ chmod), with a risk of leaving the workspace permanently read-only if the server crashes.

**Resolution:** Demoted to a **strict mode opt-in** (`config: enforcement = "advisory" | "strict"`, default `advisory`), applied only to source files (excluding `.git/`, `target/`, `node_modules/`). 1.0 relies on Layers 1–3 (rule injection + tool description + interlocking errors) — which is a practical resolution since Layer 3 (`CHECK_IN_REQUIRED`) is sufficient to create a self-correction loop for LLMs.

### F-05 🟡 UC-37/38 (Spawn / Handover) — Conditional Feasibility, Phase 3

- `claude -p "<prompt>"` is real; but `antigravity-cli --task --auto-approve` must be verified per provider — CLI flags evolve rapidly.
- Missing in design: **max spawn depth** (prevents sub-agents from spawning further sub-agents recursively), cost/token budgets for background agents, reaping zombie processes, and how users monitor/kill background agents.
- **Resolution:** Provider registry via configuration (command templates declared in `config.toml`, not hardcoded in Rust), default `max_spawn_depth = 1`, all spawns log to `agent_activities` + display on the dashboard.

---

## 3. Minor Findings (fixed during file edits)

| # | Finding | Resolution |
| :- | :--- | :--- |
| F-06 | UC summary table (URD §7.1) missing UC-37, UC-38 (present in Group I) | Added 2 rows to the table, Priority P2 |
| F-07 | Version drift: implementation doc recorded `rusqlite 0.31`/`tokio-rusqlite 0.5`, Cargo.toml used `0.32`/`0.6` | Aligned to Cargo.toml (sole source of truth for versions) |
| F-08 | Incorrect Claude config path: `~/.claude/claude_desktop_config.json` | Claude Code uses `.mcp.json` (project scope) or the `claude mcp add` command; `claude_desktop_config.json` belongs to Claude **Desktop** (macOS: `~/Library/Application Support/Claude/`) |
| F-09 | "Implicit Session Binding" (UC-01) described using SSE query params | Rewritten based on actual transport: stdio = 1 process/1 session (trivial); Streamable HTTP = binds via `Mcp-Session-Id` header (rmcp feature `transport-streamable-http-server-session`) |
| F-10 | Typo `.windsufrules` (URD §9.2) | Fixed to `.windsurfrules` |
| F-11 | `lib.rs` declares `db`, `workspace`, `engine`, `ollama` but files do not exist; `co-force-mcp` declares `[[bin]]` but lacks `src/main.rs` → `cargo check` fails | Highest priority of Phase 0: make workspace compile + CI passes |
| F-12 | Model names: `gemma4:e2b`, `mxbai-embed-large` — **verified present on ollama.com** ✅ | No action needed |
| F-13 | Dashboard proposed 2 separate ports (3847 HTTP + 3848 WS) | Consolidated to 1 port: axum serves static files + upgrades WebSockets on the same listener — removing one firewall rule and one config key |
| F-14 | Missing plans for Setup Wizard, `co-force init`, distribution — despite being critical for adoption | Added `docs/plans/05_setup_ux_and_onboarding.md` |
| F-15 | Missing overall roadmap/phasing — original plans had no execution order or MVP definitions | Added `docs/plans/00_roadmap.md` |

---

## 4. Re-evaluated Feasibility (updated with verified data)

| Component | Old URD | Re-evaluated | Reason |
| :--- | :--- | :--- | :--- |
| MCP Server (rmcp) | 🟢 90% | 🟢 **97%** | rmcp is now 2.x stable, rich docs/examples; eliminated pre-1.0 risks |
| SQLite (rusqlite) | 🟢 95% | 🟢 95% | No change |
| Ollama Integration | 🟢 95% | 🟢 95% | Model names verified; retains fallback queue |
| Vector Search | 🟢 85% (embedvec) | 🟢 **95%** (brute-force → sqlite-vec) | Removed risky dependency, removed UC-30 |
| Agentic Chunking | 🟡 70% | 🟡 70% | No change — Phase 2, can ship structural-only first |
| LLM Classification | 🟡 75% | 🟡 78% | Few-shot + confidence threshold + manual override design is solid |
| Skill Auto-Detection | 🟡 65% | 🟡 65% | Phase 3+, not on the critical path |
| Cross-Machine Sync | 🟢 85% | 🟢 85% | Streamable HTTP replaces SSE — equivalent complexity |
| 4-Layer Guardrails | — | 🟢 88% (Layers 1–3) / 🔴 50% (Layer 4) | Layer 4 downgraded to opt-in (F-04) |
| A2A Spawn/Handover | — | 🟡 70% | Feasible with provider registry + depth limit (F-05) |
| Tauri App | 🟢 90% | 🟢 90% | Pushed to future backlog — 1.0 uses embedded web dashboard |

**Overall: 🟢 ~85%** on the condition of following the phasing in `00_roadmap.md`. The largest remaining risk is **scope**: 26 MCP tools + RAG + spawning + dashboard + Tauri was too much for one release; by executing according to the roadmap, each phase is independently feasible and delivers immediate value.

---

## 5. Architectural Decisions Summary (post-review)

1. **rmcp 2.x**, transports: `stdio` (default, solo) + `streamable-http` (team/LAN). No SSE.
2. **Single SQLite file per workspace** containing both metadata and vectors (brute-force cosine, `VectorSearch` trait for future upgrades). No `embedvec`.
3. **Server state at `~/.co-force/data/{workspaceId}/`** on the server machine; `<project>/.co-force/` is strictly client-side cache (git-ignored).
4. **Default Guardrails = Layers 1–3**; chmod enforcement is strict-mode opt-in.
5. **Ollama is optional** — the server coordinates normally when Ollama is unavailable; RAG runs in degraded mode (saves without vector, recalls via keyword LIKE) + retry queue flushes when active. (Note: overridden by product owner in v2).
6. **Single binary `co-force`** with subcommands (`serve`, `init`, `doctor`, `config`, `dashboard`) — detailed in Plan 05.
7. **Dashboard 1.0 = web dashboard embedded in the binary** (axum, 1 port); Tauri is future backlog.
8. **A2A spawn via provider registry in config.toml**, `max_spawn_depth = 1`.

The synthesized architecture diagram reflecting these decisions can be found in `docs/architecture.md`.

---

## 6. Direction Pivot (2026-07-08, v2 — Product Owner Decision)

Following the presentation of the review report, the product owner finalized the direction, **replacing some decisions in §5**:

| Old §5 Decision | Status | Replaced By |
| :--- | :--- | :--- |
| #5 — Ollama optional, RAG degraded (keyword) on failure | ❌ **Cancelled** | Ollama **is mandatory** on the server. No degraded mode cutting features — failures return a clear `SERVICE_UNAVAILABLE` + auto-heals + alerts (Master Plan §5) |
| Phased MVP roadmap (first version of 00_roadmap) | ❌ **Cancelled** | **One end-to-end 1.0 release ready for production**, no intermediate MVPs (Master Plan v2) |
| Solo zero-config is the default | 🔄 **Adjusted** | Standard deployment = **independent server + cloudflared tunnel + public domain**; server init can be heavy, **client setup < 60s requiring no binaries** (Plan 05 v2, Plan 06). LAN/local remains supported with full features |
| #7 — Minimalist MVP Dashboard | 🔄 **Expanded** | Comprehensive dashboard in 1.0 (review queue, quality metrics, admin token/enrollment management) |

New decisions emerging from this direction:
- **N-01:** The ultimate goal of the product is **agent output quality, not execution speed** → added the **Quality Engine** as a critical-path workstream: role separation, mandatory cross-reviews, verification evidence, critique fan-outs, server-side LLM-driven spec rechecks (Plan 07).
- **N-02:** Auth via Bearer tokens per machine (independent issue/revoke), enrollment token TTL 24h exchanged for a long-term agent token (Plan 06 §4).
- **N-03:** Added a 3rd model role — **reasoner** (default local `qwen3:14b`, or cloud API) for advanced quality services server-side.
- **N-04:** `wait_events` long-poll timeout ≤ 55s to comply with Cloudflare proxy timeouts.

The remaining decisions in §5 (#1 rmcp 2.x + streamable HTTP, #2 vector BLOB in SQLite, #3 server-side storage layout, #4 Layers 1–3 guardrails + strict mode opt-in, #6 single binary, #8 provider registry for spawn) **remain in effect**.

---

## 7. Third Review Cycle (2026-07-08, v2.3) — Remaining impossibilities/inconsistencies resolved

### F-16 🔴 Docker Compose (Plan 06 §2.1) Network Failure — `bind 127.0.0.1` inside container

Binding to 127.0.0.1 is correct for bare-metal but **impossible in Docker**: each container has its own network namespace — if `co-force` binds to loopback, the `cloudflared` container (and `ollama` vice-versa) **can never reach it** over the bridge network. The compose file as written would fail to run.

**Resolution:** Inside containers, services bind to `0.0.0.0` (overridden via env `CO_FORCE_BIND`); isolation is handled by **the compose network not publishing ports to the host** — securing the container similarly to localhost binding on bare-metal. Cloudflared token-mode: public hostname on the CF dashboard points to `http://co-force:3846` (service name). Corrected in Plan 06 §2.1.

### F-17 🔴 Storing `api_tokens` in the DB per-workspace is impossible

Architecture §7 originally placed `api_tokens` in `data/{workspaceId}/co-force.db`, but: (1) admin/enrollment tokens have `workspace_scope = "*"` — belonging to no specific workspace; (2) the AuthLayer runs **before** the request is matched to a workspace (enrollment happens before a workspace exists) — the server cannot know which DB file to open to verify the token.

**Resolution:** Added a **server-level DB** `/var/lib/co-force/server.db` storing: `api_tokens`, `workspaces` registry (mapping id ↔ name ↔ data path), `audit_log`, and install/ops states. The DB per-workspace retains all other business data. Corrected in architecture §7, Plan 06 §4.1, Master Plan WS-A.

### F-18 🔴 The agent token delivery mechanism to clients (Plan 05 §3) does not work as described

Two overlapping bugs:
1. `.mcp.json` env expansion (`${CO_FORCE_TOKEN}`) reads the **environment variables of the client process** — there is no mechanism to automatically read the `.co-force/token` file. The script writing the token to a file and referencing `${VAR}` results in the client sending an empty header.
2. `.mcp.json`/`.cursor/mcp.json` are **project-scope files, committed to git by default to share with the team** — whereas agent tokens are **per-machine**. Placing machine-scope tokens in a shared file is a design contradiction (forcing git-ignoring of a file meant to be committed).

**Resolution (corrected in Plan 05 §3):** Tokens go into **user/machine-scope configurations outside the repository**: Claude Code uses `claude mcp add -s local -t http ... --header` (writes to `~/.claude.json`, per-project-per-machine, outside the repo); Cursor/Windsurf use global configs `~/.cursor/mcp.json` / `~/.codeium/windsurf/mcp_config.json`. Only the fallback direct-write token to project files (CI/generic) requires the gitignore + `git check-ignore` gate. Alongside this: the setup script **verifies that headers are actually being sent** (calling `tools/list`); if the client does not support custom headers → prints manual instructions instead of leaving a broken config.

### F-19 🟡 Plan 04 conflicts with N2 policy + uses deprecated Ollama APIs

1. §3.1 "store memory with `vector_id = null`, cron 5-minute re-embed" — written under the old silent fallback spirit (violates N2). Finalized per-tool behavior: if `embed` fails during `store_memory` → **still save** (no data loss) but response explicitly states `index_status: "pending"`; if `recall` cannot embed the query → return `SERVICE_UNAVAILABLE` (no fallback search); the re-embed queue flushes when the LLM recovers. Consistent with `PARTIAL_INDEX` (architecture §6.3) and Master Plan §5.
2. The `/api/embeddings` endpoint is **deprecated** by Ollama — use `/api/embed` (receives batch `input`, returns `embeddings: [[f32]]`).
3. §1 "relying completely on Local LLMs for security" is obsolete: embedding/classifier are always local, but **the reasoner is permitted to go to the cloud** (N-03) — documentation must explicitly warn of data leaving the machine when cloud options are selected.
4. Chunking pseudo-code: the `Chunk` struct lacked an `id` but children referenced `parent_id`; if the parent is pushed with `parent_id: None` first, children can never resolve it. Fixed.

### F-20 🟡 Task State Machine (Plan 07 §3) contains dead ends and missing exit paths

- `blocked` and `pending_handover` had **no outgoing transitions** — tasks entering these states were permanently locked.
- `awaiting_approval` lacked a user **rejection** path (only support approval).
- Missing a `cancelled` state — users could not cancel tasks without breaking the state machine.

**Resolution:** Added `blocked → in_progress` (dependency resolved), `pending_handover → in_progress` (new agent claims via L2/L3), `pending_handover → approved` (timeout with no takers → back to backlog), `awaiting_approval → draft` (user reject with reason), and `cancelled` reachable from all non-completed states (triggered by user/admin only). Corrected in Plan 07 §3.

### F-21 🟡 "Detect code changes post-submit" Revision Tracking is impossible as described

Plan 07 step 4 stated "revision increments when lock/unlock records **file changes** post-submit" — the server **cannot see the client filesystem**, it has no way to know if files changed. Redefined based on server-observed events: revision increments when (a) task returns to `in_progress` (rework), (b) agent locks additional files or re-locks files for the task after evidence is submitted, (c) a new `submit_verification` is received, or (d) a new `commit_sha` is received. When git remote is active + L3 is enabled: the server **verifies the `commit_sha` exists in the mirror** during `submit_verification` (fetches first) — if missing → returns `EVIDENCE_STALE {reason: "commit_not_found"}` requiring a push. Without a remote: trusts local execution limits, and the primary gate remains the reviewer running tests independently. Corrected in Plan 07 §5.1 and architecture §6.3.

### F-22 🟡 Deadlock of `reviewer_must_differ = "provider"` policy in single-provider setups

If a workspace only has Claude Code agents + worker pool `providers = ["claude-code"]` → can never satisfy the policy → **every task freezes at code_review indefinitely** (auto-staffing is powerless). Freezing at gates + alerting fits N2, but exposing configuration deadlocks to users at runtime is unacceptable. **Resolution:** Validate when **setting policy / enrolling / installing**: if `reviewer_must_differ="provider"` but < 2 providers are available → rejects the change with instructions (add providers or lower setting to `"agent"`), dashboard alerts if available providers drop to 1 (e.g. revoking the last Cursor machine). Corrected in Plan 07 §8.

### F-23 🟢 Documents still reference deprecated APIs/models (fixed to avoid DEV copying mistakes)

| Location | Issue | Resolution |
| :--- | :--- | :--- |
| `progress.md` §2 | Still mentions "`#[rmcp::server]`", "Transport (stdio / sse)" | → `tool_router`/`ServerHandler`, streamable-http |
| Plan 02 §3.2 sample | Sample code still has `#[rmcp::server]` | → `#[tool_router]` + notes on real API |
| Plan 03 §4.1 + step 4 | Hardcoded `match provider` (antigravity/claude) in Rust — violates F-05 (provider registry in config) | Annotated as structural illustration; command templates loaded from `config.toml [providers]` |
| Plan 04 §2 | Hardcoded comment "Returns 1024 dimension vector" | Dimensions depend on model config (change model → re-embed, Plan 06 §7) |
| Architecture §6.4 | Tool 1 records "only tool not requiring session" but tool 3 (`guide`) also does not require one | Corrected wording |

### F-24 🟢 `wait_events` 55s vs MCP client tool-call timeouts

55s is below Cloudflare's timeout (100s) but **some MCP clients have shorter tool-call timeouts** — if the client cuts off early, the long-poll turns into an error loop. Resolution: default `timeoutSecs = 25`, max 55; the enrollment script executes one real `wait_events` call to measure and print recommendations if the client terminates early. Recorded in Plan 07 §4.2.

---

## 8. Fourth Review Cycle (2026-07-08, v2.4) — Provider CLI coverage

### F-25 🟡 Documentation only mentioned Claude Code CLI — lacking Codex CLI & Antigravity CLI (fixed)

Previous architecture hardcoded `claude`/`antigravity-cli` in examples; no integration specs existed for **Codex CLI** (OpenAI) and **Antigravity CLI `agy`** (Google — Gemini CLI successor, Gemini CLI shutdown 2026-06-18). This directly conflicted with the Quality Engine: `reviewer_must_differ = "provider"` and multi-model critiques (Plan 07) only make sense when ≥ 2 true providers participate — related to F-22.

**Deep-researched (official OpenAI/Google docs) + compared against Tutti's provider architecture** (multi-agent shared workspace running on existing subscriptions — the `ProviderSpec` registry with binary names, adapter commands, auth markers, and login specs per provider is an excellent model; Tutti controls Codex via `codex app-server` and other CLIs via ACP). Result: **new Plan 08** (`docs/plans/08_provider_cli_integration.md`) — subscription-first, declared registry, verified specs for claude/codex/agy/cursor-agent, and 4 caveats that must be handled in code (C1: Codex `exec` auto-cancels MCP approvals — only bypassed in L3 sandbox; C2: `bearer_token_env_var` requires real env vars; C3: agy MCP header support unconfirmed — verified during enrollment + stdio shim fallback; C4: auth-status parser per provider). Synced: architecture §1/§5, Plans 03/05/06/07, roadmap.

### F-26 🟢 Document cleanup (2026-07-08): deleting `implementation_instructions.md`

The file contained too many incorrect instructions to save with a banner: pinned `rmcp 0.16` + `transport-sse` in Cargo.toml; its own banner claimed "Ollama is optional" (violating the finalized N2 principle); sample config had 2 dashboard ports (violating F-13); classify "fallback keyword heuristic if Ollama is unreachable" (silent degradation — prohibited); schema missing 7 new tables + retained the `vector_id` column (violating F-02); SSE deployment section `http://<ip>:3846/sse` (deprecated transport); `co-force-tauri` crate (refactored to `co-force-app` client-side).

**Remaining valuable content was transferred before deletion:** the complete 6-table SQL schema → **Plan 01 §3** (updated: `embedding BLOB` replaces `vector_id`, `tasks.status` aligns with Plan 07, `revision`/`rework_cycle` columns added for F-21, hot-path indexes created); the TaskStatus enum in Plan 01 §2.2 matches the Plan 07 state machine. References in Plans 01/02/04 now point to `architecture.md` + matching plans; the URD banner notes the deletion. Frequently used developer commands (cargo test/clippy pedantic/fmt) are documented in AGENTS.md §2.3–2.4 and the DoD of each workstream.

### F-27 🟡 Missing specification on "what does the agent do after setup" — onboarding contract & uniform behavior (fixed)

Following Plan 05 (client setup), no documentation finalized: how the agent (Claude Code or any CLI agent) knows **which tool to use for what**, where the **starting point** is, and what mechanism ensures **all agents behave uniformly**. Scattered content: Plan 05 §3.5 only mentioned "rule injection" in one line; the original URD §9.3 template contained incorrect instructions (threats of "OS Permission Denied / chmod bans" — no longer true after Layer 4 went in-band; the old `update_task(completed)` flow now returns `GATE_VIOLATION`); URD §10 Phase 5–6 lacked quality gates.

**Fixed — created `docs/plans/09_agent_operating_protocol.md`:** (1) the discovery sequence of 4 touchpoints for a cold agent (rules file → tool descriptions → check_in response → envelope on every response — the agent doesn't need to *remember* the protocol, the protocol finds the agent); (2) **finalized rules template v1** (managed block, English, only promises what the server actually enforces, with a "which tool when" table); (3) standardized 39 tool descriptions (Layer 2); (4) dynamic `co_force_guide` spec + `onboarding: true` flag; (5) playbooks by role (developer/reviewer/critic/pm); (6) **uniform behavior ↔ enforcement layer matrix** — every rule has at least one server-side enforcement layer, not relying on LLM goodwill; (7) E2E "cold agent" acceptance testing criteria. URD §9.3/§10 annotated as replaced; Plan 05, architecture §6, and roadmap point to Plan 09.

### F-28 🟡 Missing mechanism for the "single agent + long job" scenario (solo → context bloat → hallucination) — fixed

Realistic scenario: the workspace only has 1 agent (e.g. Antigravity), a long job with many subtasks → the agent takes everything, context window bloats → hallucinations, quality collapse. The docs had server-side auto-staffing (Plan 07 §2.2) but lacked: (1) how the agent **automatically knows it is solo** and must partition work; (2) promoting the original agent to **PM** + estimating how many dev/test/ba/qa agents are needed before spawning; (3) handling race conditions when multiple subagents run on the **same machine, in the same working tree** (lock logic does not prevent `git add -A`, repository-wide formatters, or build artifact clashes).

**Fixed — created `docs/plans/10_solo_orchestration.md`:** 3-tier solo detection (SOLO RULE in the rules template + in-band `team_context.solo` + server nudge based on `solo_team_threshold_tasks`); new **`co_force_plan_team` (#39)** tool — heuristic clustering of parallel lanes based on disjoint lock sets + reasoner refinement → estimates the team with rationales (dev/reviewer/qa/ba), presented to the user before spawning; PM lifecycle managing subagents (spawns L2 with `taskIds[]` + narrow bootstrap prompt for clean context, stall detector alerting the PM via the inbox, kill/respawn, PM does not code while the team runs, aggregates questions for the user once); 2-tier local race prevention (default: explicit git paths in bootstrap prompt / `use_local_worktrees = true`: private worktrees per task, absolute isolation); solo + 1 provider → pins `reviewer_must_differ="agent"` between **distinct identities** + model diversity supplemented by server-side reasoner and L3 reviewer from a different provider. Synced: architecture §6.4 (39-tool catalog), Plans 03/06/07/09, roadmap §2.1/WS-E (+solo scenario DoD).

### F-29 🟡 Cross-provider handover on rate limit — old flow contained 4 flaws (fixed)

Standard use case: Claude CLI + agy CLI working on the same feature, Claude hits subscription rate limits → must transfer context + task to agy. The old handover flow (Plan 03 §4.2) failed because: (1) it did not distinguish **rate limits** (resetting after N hours — requires cooldown tracking, unlike context exhaustion); (2) it assumed context was portable — in reality, conversations cannot be ported between different providers (`--resume` is per-provider); (3) the "release all locks" step created a gap for a third agent to intercept files mid-handover; (4) it lacked re-dispatching when an agent died suddenly due to a hard rate limit.

**Fixed — new Plan 03 §5 (Cross-Provider Handover):** context is externalized via **5 provider-independent channels** (task record, handover package, activity stream, memory, git state); the **handover package has a schema** (done/remaining/decisions/gotchas/next_steps/code_state) **validated by the reasoner** — missing/ambiguous fields → returns the new `HANDOVER_INCOMPLETE` error (sloppy handovers are blocked like other gates); locks are placed in **escrow tied to the task**, transferring atomically to the successor; **provider cooldown** (`provider_status` in server.db) — staffing/delegation avoids the limited provider until it recovers, returning automatically afterward; target matrix (§5.4: online agent → inbox offer / pushed to remote → L3 / local not pushed → L2 same machine); **passive flow**: sudden death → reclaim + automatic offer to another provider with a server-synthesized package from the activity journal (new rules: update_task progress notes = handover insurance). Synced: architecture §5.4/§6.3/§6.4 tool 23, Plans 07 (validation service)/08 (cooldown awareness)/09 (early-handover rules + progress notes), roadmap WS-A (`handovers` and `provider_status` tables) + WS-E standard scenario DoD.
