# Detailed Implementation Plan: 09 - Agent Operating Protocol (Onboarding & Uniform Behavior)

**Status:** Ready for Implementation (supports WS-B/C/G — finalized 2026-07-08)
**Target:** `crates/co-force-core/src/workspace/protocol_templates/` (rules template + guide renderer), tool descriptions in `co-force-mcp/src/tools/`
**Target Question:** *After client setup is complete, how does Claude Code (or any CLI agent) know which tool to use for what, where to start, and how to behave uniformly?*

Replaces URD §9.3 (the old template contained incorrect instructions: threats of "OS Permission Denied" / chmod bans no longer exist since Layer 4 was changed to go in-band — architecture §5.6; the old flow where `update_task(completed)` was called now returns `GATE_VIOLATION`).

---

## 1. Discovery Sequence for a Cold Agent (Cold Start) — 4 Touchpoints

A new agent starting a session knows nothing about Co-Force. It "learns" the protocol through exactly 4 touchpoints, in chronological order:

| # | Touchpoint | When | What the Agent Receives |
| :- | :--- | :--- | :--- |
| 1 | **Rules File** (`AGENTS.md`/`CLAUDE.md`/`.cursorrules` — managed block §2, injected by enrollment script) | The client automatically loads it into its system context when opening the project | Check-in starting point, task lifecycle, uniform behavior rules, tool map §2.4 |
| 2 | **Tool Descriptions** (Layer 2 — §3) | When the client loads the list of 39 tools from the server | Each tool explicitly states when it MUST be used ("MANDATORY: call first...") |
| 3 | **`co_force_check_in` Response** | First tool call of the session | Pending tasks + team online + unread inbox + `protocol_next_step` + `onboarding: true` on first run → directed to `co_force_guide()` |
| 4 | **Subsequent Tool Responses** (envelope §6.2 architecture) | Throughout the session | `protocol_next_step` indicating the next action, `inbox` delivering team work, error + `recovery_action` for self-correction |

Design Principle: **the agent does not need to memorize the protocol** — the protocol finds the agent at every step. The rules file only needs to be sufficient for step 3 to occur; from there, the server guides the agent in-band.

---

## 2. Finalized Rules Template (Managed Block — Injected by Enrollment Script, Plan 05 §3 step 5)

Writing Principles: (a) **English** (maximizes compliance across all CLI agents); (b) **only promise what the server actually enforces** — no empty threats like "the OS will block you" (agents detecting false rules lose trust in the entire protocol); (c) short enough to fit in the context of every session, routing details to `co_force_guide()`; (d) versioned to allow updates via re-enrollment.

```markdown
<!-- CO-FORCE:BEGIN v1 (managed block — do not edit; re-run enrollment one-liner to update) -->
# Co-Force Team Protocol — {{workspace_name}}

You are ONE agent in a coordinated multi-agent team. The Co-Force MCP server
(tools `co_force_*`) is the single source of truth for tasks, file claims,
messages, reviews, and shared memory. Work done outside it conflicts with
teammates and WILL be rejected at server-enforced quality gates.

## Session start — always, before anything else
1. Call `co_force_check_in(workspacePath, agentName, role)`.
   Every other tool returns CHECK_IN_REQUIRED until you do.
2. Read the response: your pending tasks, teammates online, unread inbox.
   If it contains `onboarding: true`, call `co_force_guide()` once.
3. Before planning any work: `co_force_recall(<topic>)` to load the team's
   memory/knowledge/skills. Cite what you reuse.

## Task lifecycle (server-enforced gates — skipping returns GATE_VIOLATION)
draft → spec_review (auto LLM recheck) → awaiting_approval (USER approves)
→ approved → in_progress → verification → code_review → completed
- Turn any non-trivial work into tasks: `co_force_create_tasks` with
  objective, use cases, and a verification plan.
- NEVER edit files before: task approved AND `co_force_lock_files` succeeded
  on the paths you will touch. On LOCK_CONFLICT: `co_force_check_conflicts`,
  then coordinate (`co_force_send_message`) or `co_force_delegate_task` —
  do not edit anyway.
- You CANNOT set status=completed. The only path: `co_force_submit_verification`
  with REAL evidence (actual test/lint commands, outputs, exit codes, and
  `commit_sha` — push first when the repo has a remote) → cross review by a
  DIFFERENT agent → their approval completes it.
- Evidence is bound to the current task revision. Changed anything after
  submitting? Re-run tests and submit again (stale evidence → EVIDENCE_STALE).
- Review returned findings? Task is in `rework`: fix, re-verify, resubmit.

## Uniform behavior — every agent, every turn
- Every tool response carries `inbox`, `protocol_next_step`, `workspace_pulse`.
  OBEY `protocol_next_step`. Handle inbox items with `requires_response`
  (review requests, questions) BEFORE continuing your own task.
- Never review, approve, or critique your own work (server enforces this).
- `co_force_unlock_files` the moment you stop working on files.
- On ANY tool error: perform its `recovery_action` verbatim.
  SERVICE_UNAVAILABLE → wait `retry_after_secs`, retry; ops is already
  alerted. Do NOT work around the server or lower quality to proceed.
- Running low on context, OR you see the FIRST rate-limit warning from your
  harness? Do not start anything new: commit + push WIP, then
  `co_force_handover` with a complete package (done/remaining, decisions,
  gotchas, next steps) — another agent (possibly a different provider)
  continues from it. Never silently abandon work.
- Write short progress notes via `co_force_update_task` as you work — if you
  die unexpectedly (hard rate limit, crash), that journal IS the handover.
- Waiting on the team (e.g., you are a reviewer on duty)? Loop
  `co_force_wait_events` instead of ending your session.
- Store durable, non-obvious learnings with `co_force_store_memory` when a
  task completes.
- SOLO RULE: if check_in shows you are the ONLY agent online
  (`team_context.solo: true`) and the work spans more than ~3 tasks, do NOT
  do everything yourself — a bloated context degrades your quality. Register
  role `pm` (`co_force_register_role`), call `co_force_plan_team`, confirm
  the estimate with the user, then spawn the recommended subagents. While
  the team runs, you coordinate — you do not code.

## Which tool, when (quick map)
| Situation | Call |
| :--- | :--- |
| New session | `check_in` → `recall` |
| Plan a feature/fix | `create_tasks` (recheck runs automatically) |
| Start an approved task | `lock_files` → `update_task(in_progress)` |
| Someone holds my files | `check_conflicts` → `send_message` / `delegate_task` |
| Coding done | `submit_verification` (evidence + commit_sha) |
| Asked to review | read the code, run tests yourself, `submit_review` with findings |
| Big design decision | `request_critique` BEFORE coding |
| Need help / spawn worker | `delegate_task` / `co_force_spawn_agent` |
| Solo with a big backlog | `register_role(pm)` → `plan_team` → spawn team |
| Context nearly full / rate-limit warning | push WIP → `handover` (package contents) |
| Learned something durable | `store_memory` |
| Who is doing what | `list_agents` / `workspace_status` / `whoami` |

Server: {{server_url}} · All tool names are prefixed `co_force_`.
<!-- CO-FORCE:END -->
```

Template variables: `{{workspace_name}}`, `{{server_url}}` (+ `{{role_hint}}` if the machine is enrolled with a fixed role). The same template applies to all clients — the only difference is the target file (`AGENTS.md` shared by Claude Code/Codex/agy; `.cursorrules` for Cursor — Plan 08 §3 rules column).

---

## 3. Layer 2 — Standard for Writing Tool Descriptions (Aligned with Rules)

Tool descriptions are "rules at the point of consumption" — the agent reads them when selecting tools. Mandatory standard for implementation (`co-force-mcp/src/tools/`):

1. **Entry tool explicitly declares mandatory nature:** `check_in` = "MANDATORY first call of every session. All other tools fail with CHECK_IN_REQUIRED until called."
2. **Tool with preconditions explicitly states them:** `lock_files` = "MANDATORY before editing any file. Requires an approved task."; `submit_verification` = "The ONLY way to move a task toward completed. Requires real test evidence + commit_sha."
3. **High-risk tools explicitly state consequences:** `update_task` = "Cannot set completed (GATE_VIOLATION) — use submit_verification."
4. **Never promise what the server does not enforce** (aligning with §2b).
5. The description is a contract — modifying a description changes the protocol → the same PR must update the template §2 and guide §4 to keep them aligned.

## 4. `co_force_guide()` — Dynamic Onboarding (Details omitted in static rules)

The server renders this based on the real workspace state (not static markdown):
- Active quality policy (reviews_required, reviewer_must_differ, evidence kinds) — "why your task requires X".
- Current team: agents + roles + who holds which lock; task backlog awaiting claim.
- **3 examples of standard tool calls** matching the active policy (create_tasks with all fields, submit_verification with complete evidence, submit_review with findings schema).
- Common errors → recovery actions (abridged error codes from architecture §6.3).
- Trigger: the check_in response of a new agent contains `onboarding: true` + `protocol_next_step: "Call co_force_guide() once before taking any task."`

## 5. Playbook by Role (Sent by Server via Guide + review_request Payload)

| Role | Standard Loop |
| :--- | :--- |
| `developer` | check_in → recall → (claim approved task or create_tasks) → lock → code → submit_verification → handle findings → store_memory |
| `reviewer` (including L3 workers) | check_in(role=reviewer) → `wait_events` loop → receives review_request (with assist checklist) → reads actual code (worktree/workspace) → runs tests independently → submit_review(findings with file/line/severity) → returns to wait_events |
| `critic` | receives critique_request → submit_critique(position, arguments, risks, alternatives) — honest critiques, no polite pleasantries |
| `pm`/`architect` | create_tasks + request_critique before big decisions; does not self-approve. **PM solo-bootstrap (Plan 10):** plan_team → presents estimate to user → spawns subagents → monitoring loop `wait_events` (handles stalls/respawns, aggregates questions for user) — **does not code while the team is running** |

## 6. "Uniform Behavior" Matrix — Enforcement Layers

Uniform behavior does not rely on LLM goodwill — every rule in §2 has at least one enforcement layer server-side:

| Rule | Layer 1 (Rules) | Layer 2 (Descriptions) | Layer 3 (Interlocking Server) | Layer 4 (In-band State) |
| :--- | :---: | :---: | :---: | :---: |
| Check-in first | ✓ | ✓ | ✓ `CHECK_IN_REQUIRED` blocks remaining 37 tools | |
| Lock before edit | ✓ | ✓ | ✓ `LOCK_CONFLICT` on duplicate claims | ✓ pulse shows team locks |
| Do not self-set completed | ✓ | ✓ | ✓ `GATE_VIOLATION` | |
| Real evidence on correct revision | ✓ | ✓ | ✓ validator + `EVIDENCE_STALE` (F-21) | |
| Do not self-review | ✓ | | ✓ server-side separation of duties | |
| Process inbox first | ✓ | | | ✓ inbox + `protocol_next_step` on all responses |
| Self-correct via recovery_action | ✓ | | ✓ every error returns a recovery_action | |
| Handover instead of abandoning | ✓ | ✓ | ✓ reclaim after 2-minute grace period | ✓ pulse warning |

→ Even the most stubborn agent converges to the correct flow because **incorrect paths are blocked and returned with guiding instructions to the correct path** (self-correction loop). Layer 1 rules only accelerate convergence (fewer error cycles), they are not the sole barrier.

---

## 7. Steps to Implement (Step-by-Step)

1. Save the §2 template as a file in `workspace/protocol_templates/rules_v1.md` (with a version constant); the managed-block writer is shared with Plan 03/05; golden-file test rendering with sample variables.
2. Standardize the 39 tool descriptions according to §3 (descriptions reside next to their handlers; cross-review with the template to keep them aligned).
3. `co_force_guide` renderer (§4): input = quality policy + team snapshot + backlog; outputs markdown; unit tests with different policies output different examples.
4. Include `onboarding: true` flag in the check_in response (if the agent has not checked into this workspace before) + point `protocol_next_step` to the guide.
5. Playbook §5: embed into the guide based on role; review_request payload includes checklist + prompts reviewer process.
6. **E2E "cold agent"** (acceptance criteria for this plan): clean container → enroll → open real Claude Code with a neutral prompt ("add a hello endpoint") → assert agent automatically: check_in → recall → create_tasks → halts waiting for approval (no file modification before locking) — repeated for Codex + agy (Plan 08).
