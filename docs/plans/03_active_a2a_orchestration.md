# Detailed Implementation Plan: 03 - Active A2A Orchestration Layer

**Status:** Ready for Implementation
**Target:** `crates/co-force-core/src/orchestration/` and Event Bus

> **⚠️ Update 2026-07-08 (v2 — production model):** In production, the server is a remote machine behind a cloudflared tunnel — **the server cannot spawn processes on the client machine**, and the workspace code resides on the client. Therefore, spawning and handover operate under a **3-lane model** (finalized in `architecture.md` §5):
> - **Lane 2 (spawn-by-directive):** `co_force_spawn_agent(placement:"local")` → the server does NOT spawn the process, but returns a `spawn_directive {command, env, cwd}` + scoped token; the requesting agent executes the command locally using its own shell tool. The server-side `ProcessManager` simply validates, generates the directive, and monitors the check-in of the child agent (120s timeout).
> - **Lane 3 (server worker pool):** spawns headless on the server within a **git worktree sandbox** (`/var/lib/co-force/workspaces/{wsId}/jobs/{taskId}`) cloned from the mirror via a deploy key — used for automated reviewer/critic staffing and handover when the client is offline. The sample code under §4 below applies to this lane, adding: fetching mirror + creating worktree before spawn, cgroup/nice limits, token budget, deleting worktree after job completion.
> - Detailed lane selection matrix + handover workflow: `architecture.md` §5.4. Handover priority: commit + push WIP branch → L3 worker resumes.
> - **Solo orchestration (Plan 10):** L2 expands for the scenario where 1 agent bootstraps the entire team on a single machine — spawning receives `taskIds[]`, narrow bootstrap prompt (clean context to prevent hallucination), explicit git rules to prevent race conditions on the same working tree, `use_local_worktrees` option for absolute isolation, stall detector alerting the PM via the inbox.
> - **Doc Generator (§3 below) only writes files where the server has filesystem access:** worker worktrees (L3) and LAN variants. For remote clients, the server **cannot** write `AGENTS.md`/`.cursorrules`/`session_status.json` into the workspace — dynamic state is delivered **in-band** via the response envelope (`workspace_pulse`, `inbox` — architecture.md §5.6); static rules are written by the enrollment script (Plan 05). The dynamic AGENTS.md render is exposed at `GET /api/workspaces/{id}/agents.md` for the dashboard.

## 1. Context & Objectives
This is the core module upgrading Co-Force to an **Active A2A Orchestrator**. Instead of simply responding passively to MCP requests, the server monitors state changes (via the Event Bus), automatically generates orientation documentation (`AGENTS.md`), and proactively spawns new agents using OS Processes (for task division or fallback/handover).

*Reference Documents:*
- `URD.md` (Section 14.2 Event-Driven Architecture)
- `URD.md` (Group I: Active A2A Orchestration - UC-37, UC-38)
- `URD.md` (UC-36: Dynamic AGENTS.md Generation)

---

## 2. In-Memory Event Bus
**Location:** `crates/co-force-core/src/orchestration/bus.rs`

Decouples MCP handlers and Background Tasks.

### 2.1 Event Definition
```rust
#[derive(Debug, Clone)]
pub enum WorkspaceEvent {
    AgentCheckedIn { agent_id: String, workspace_id: String },
    FilesLocked { agent_id: String, files: Vec<String> },
    TaskUpdated { task_id: String, new_status: String },
    ActivityLogged { activity_id: String },
    ContextShared { context_id: String },
    HandoverRequested { old_agent_id: String, task_id: String, next_provider: String },
}
```

### 2.2 Global Broadcaster
Create an instance of `tokio::sync::broadcast::Sender` and share it across all Use Cases via `Arc`.
```rust
// Initialize in main.rs or lib.rs
let (tx, _rx) = tokio::sync::broadcast::channel::<WorkspaceEvent>(1024);
let bus_sender = Arc::new(tx);
```
Each Use Case (like `LockFilesUseCase`) upon completing its DB transaction calls `bus_sender.send(WorkspaceEvent::FilesLocked { ... }).ok();`.

---

## 3. Dynamic AGENTS.md Generator
**Location:** `crates/co-force-core/src/orchestration/doc_generator.rs`

Background task listening to the Event Bus, debouncing events, and automatically overwriting the AGENTS.md file.

### 3.1 Daemon Loop
```rust
pub async fn run_doc_generator(
    mut rx: tokio::sync::broadcast::Receiver<WorkspaceEvent>,
    agent_repo: Arc<dyn AgentRepository>,
    task_repo: Arc<dyn TaskRepository>
) {
    loop {
        // Listen for events
        let Ok(event) = rx.recv().await else { break };
        
        // Debounce: Wait 2-3s for consecutive events to avoid excessive writes
        // ... (tokio::time::sleep logic)
        
        // Query DB for latest state
        let agents = agent_repo.list_active(...).await;
        let tasks = task_repo.find_pending(...).await;
        
        // Format to Markdown
        let md_content = format_to_managed_block(&agents, &tasks);
        
        // Write to .co-force/AGENTS.md (use regex to replace only within BEGIN/END markers)
        write_managed_block(".co-force/AGENTS.md", &md_content).await;
        
        // Also write to .cursorrules and .clauderules if files exist
    }
}
```

---

## 4. OS Process Manager (Spawn & Kill Agents)
**Location:** `crates/co-force-core/src/orchestration/process_mgr.rs`

Responsible for executing system commands to spawn headless sub-agents (Sub-agents).

### 4.1 ProcessManager Structure
```rust
use tokio::process::Command;

pub struct ProcessManager;

impl ProcessManager {
    /// Spawns a CLI agent in background (detached) mode
    /// NOTE (F-05/F-23): The hardcoded match below is strictly for illustration —
    /// the real implementation reads the command template from config.toml [providers] (provider registry),
    /// no hardcoded match in Rust.
    pub async fn spawn_agent(provider: &str, task_id: &str, context: &str) -> anyhow::Result<u32> {
        let mut cmd = match provider {
            "antigravity" => {
                let mut c = Command::new("antigravity-cli");
                c.arg("--task").arg(context);
                c.arg("--auto-approve"); // Critical: background agent must not block for user input
                c
            },
            "claude-code" => {
                let mut c = Command::new("claude");
                c.arg("-p").arg(context);
                c
            },
            _ => return Err(anyhow::anyhow!("Unknown provider")),
        };

        // Spawn detached process
        let child = cmd.spawn()?;
        let pid = child.id().unwrap_or(0);
        
        // Optional: Spawn task to wait() on child to reap zombie processes.
        Ok(pid)
    }
}
```

### 4.2 Use Case: Handover (Task Transition)
> **⚠️ The flow below is the original version, replaced by §5 (cross-provider, escrow locks):** step 2 "release all locks" created a vulnerability — a third agent could claim files during the handover gap. The finalized design utilizes **escrow locks tied to the task** and transfers them atomically to the successor.

Upon receiving the `co_force_handover` MCP request:
1. The Use Case updates the Task Status to `PendingHandover` and saves the `state_summary` in the task description.
2. ~~Call `LockRepository::release_all_for_agent(agent_id)`~~ → locks enter escrow tied to the task (§5.3).
3. Send a `WorkspaceEvent::HandoverRequested` event to the Event Bus.
4. Select the target according to the §5.4 matrix (another online agent → offer; or spawn L2/L3).
5. Return `safe_to_exit: true` to the old Agent, permitting it to exit safely.

---

## 5. Cross-Provider Handover — Standard Scenario: Claude CLI Hits Rate Limit, agy CLI Takes Over

**Fundamental Truth:** Conversation/context CANNOT be ported between two different providers (`claude --resume` is meaningless to `agy`). Therefore, "importing context" = **externalizing the entire state to the server + git**, and the succeeding agent recreates the context from there. This is the core value of Co-Force: the context lives on the server, the agent is just a temporary worker.

### 5.1 Context Transferred via 5 Channels (None dependent on provider)

| Channel | Contents | Author |
| :--- | :--- | :--- |
| Task record (DB) | spec, use cases, verification plan, revision, current gate | create_tasks + update_task |
| **Handover package** (table `handovers`) | §5.2 — done/remaining, decisions, gotchas, next steps | Old agent during handover |
| Activity stream | journal of all tool calls by the old agent (append-only) | Automatic |
| Memory (`recall`) | distilled knowledge/gotchas | store_memory + nightly distillation |
| Code state (git) | WIP branch + commit_sha, or local working tree | Old agent commit/push |

### 5.2 Handover Package (Validated — hand-off quality is also a quality gate)

```json
{
  "reason": "rate_limit",                     // rate_limit | context_exhaustion | session_end | manual
  "provider_cooldown_until": "2026-07-08T21:00:00Z",  // cooldown reset timestamp (if reported by CLI)
  "progress": {"done": ["API skeleton", "3/5 tests"], "remaining": ["wire auth", "2 failing tests"]},
  "decisions": [{"what": "use middleware X", "why": "..."}],
  "gotchas": ["test Y is flaky in parallel run", "do not modify signature Z — old client depends on it"],
  "code_state": {"kind": "pushed_wip", "branch": "co-force/t42", "commit_sha": "a1b2c3",
                  "files_touched": ["src/auth.rs", "tests/auth_test.rs"]},
  "next_steps": ["run cargo test -p core first", "start from the TODO in auth.rs:88"]
}
```

The server uses the **reasoner to validate completion** (missing `remaining`/`next_steps` or ambiguity → triggers a `HANDOVER_INCOMPLETE` error + a recovery action pointing to the missing fields) — sloppy handovers are blocked just like any other gate.

### 5.3 Active Flow (Claude detects rate limit — can still call tools)

1. **Rules teach early handover** (Plan 09): upon the first rate limit warning → DO NOT start new work; commit + push WIP → call `co_force_handover(taskId, reason="rate_limit", resetAt, package)`.
2. The server validates the package → sets task status to `pending_handover`; **locks are placed in escrow tied to the task** (no loose releases — transferred atomically to the successor, preventing a third agent from claiming them).
3. Record **provider cooldown**: `provider_status[machine][claude-code].rate_limited_until = resetAt` → plan_team/staffing/delegation avoids assigning tasks to this provider until the cooldown expires (Plan 08).
4. Select target according to §5.4 → the successor agent receives a `handover_offer` via the inbox (an online agy agent working on the same feature receives it IMMEDIATELY if it is in `wait_events`).
5. agy accepts → the server transfers **assignee + locks atomically**, task returns to `in_progress`; response contains the package + `protocol_next_step: "Read package.next_steps, checkout branch co-force/t42, run co_force_recall on the feature topic before coding."`
6. Claude receives `safe_to_exit: true`. Once the cooldown expires → Claude checks in again and accepts new tasks normally.

### 5.4 Target Selection Matrix (expands on architecture §5.4)

| Condition | Target |
| :--- | :--- |
| Another provider agent is **online** in the workspace, with capacity (agy in this use case) | **Offer via inbox** — fastest, agy already has feature context from previous pulses/inbox updates |
| No one is online, code **has been pushed** to WIP | **L3 worker** of a different provider (reads from mirror @ commit_sha) |
| Code is **local and not pushed** (no remote / not pushed yet) | **L2 spawn directive for agy on the SAME MACHINE** — reads the working tree directly; directive must be executed by Claude BEFORE it runs out of limits entirely (why rules teach early handover) |
| No feasible options | Task stands at `pending_handover` + alerts user + cooldown recorded; timeout → returns to backlog (Plan 07 §3) |

### 5.5 Passive Flow (Claude dies suddenly mid-way — no time to handover)

1. Session drops → 2-minute grace period → reclaim daemon runs (architecture §9). New feature: reclaim **does not just return tasks to the backlog** — if there is an online agent of a different provider → automatically sends a `handover_offer` with a **server-synthesized package**: task record + recent activity stream + git state (lacks the clean summary from the old agent).
2. This is why rules mandate writing progress notes via `update_task` frequently — **every update_task is handover insurance**; if the agent dies suddenly, the journal is the handover.
3. Detect cause: L2/L3 worker exits with stderr containing the provider's rate-limit pattern (one parser per provider — Plan 08 C4 extended) → server records cooldown identical to the active flow.

### 5.6 Additional Schema (WS-A)

```sql
CREATE TABLE IF NOT EXISTS handovers (
    handover_id  TEXT PRIMARY KEY,
    task_id      TEXT NOT NULL,
    from_agent_id TEXT NOT NULL,
    to_agent_id  TEXT,              -- NULL until accepted
    reason       TEXT NOT NULL,     -- rate_limit | context_exhaustion | session_end | manual
    package      TEXT NOT NULL,     -- JSON §5.2 (validated)
    provider_cooldown_until TIMESTAMP,
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    accepted_at  TIMESTAMP,
    FOREIGN KEY (task_id) REFERENCES tasks(task_id)
);
-- provider_status (in server.db): machine_id, provider, rate_limited_until, last_error
```

---

## 6. Steps to Implement (Step-by-Step)
1. Add `tokio::sync::broadcast` to Core, defining the `WorkspaceEvent` enum.
2. Pass the `Sender` into the Use Cases, appending a `send()` call at the end of each Use Case execution.
3. Write `doc_generator.rs`, implementing the `recv()` loop and string replacement using Regex (respecting user-made code blocks).
4. Write the `process_mgr.rs` module — command templates are retrieved from the **provider registry in config** (F-05 decision, no hardcoded providers in Rust). Verified specs for each CLI (Claude Code `claude -p`, Codex `codex exec`, Antigravity `agy -p`, with caveats C1–C4): **Plan 08 §3**.
5. Handover use case according to §5: `handovers` + `provider_status` (server.db) tables, lock escrow + atomic transfer (unit test: a third agent cannot claim locks during pending_handover), package validator via reasoner (mock), target matrix §5.4, cooldown tracking.
6. Extended reclaim (§5.5): auto-redispatch to another provider agent + server-synthesized package; stderr rate-limit parser per provider.
7. Write Integration Tests: (a) Mock OS Command to ensure Handover triggers `spawn`; (b) **cross-provider standard scenario**: mock 2 sessions (claude + agy), claude handover(reason="rate_limit") → agy receives the offer, locks transfer atomically, task continues without dropping gates; (c) claude kill -9 → after grace period, agy receives the offer with the server-synthesized package.
