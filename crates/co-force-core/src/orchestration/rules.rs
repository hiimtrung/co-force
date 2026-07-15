//! Managed rules block writer and templates.
//!
//! Implements Plan 09 §2.

/// The versioned Co-Force Team Protocol rules template.
pub const RULES_TEMPLATE: &str = r#"<!-- CO-FORCE:BEGIN v1 (managed block — do not edit; re-run enrollment one-liner to update) -->
# Co-Force Team Protocol — {workspace_name}

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

Server: {server_url} · All tool names are prefixed `co_force_`.
<!-- CO-FORCE:END -->"#;

/// Injects or updates the Co-Force managed rules block in the given file content.
pub fn inject_rules_block(original: &str, workspace_name: &str, server_url: &str) -> String {
    let rendered = RULES_TEMPLATE
        .replace("{workspace_name}", workspace_name)
        .replace("{server_url}", server_url);

    let start_tag = "<!-- CO-FORCE:BEGIN";
    let end_tag = "<!-- CO-FORCE:END -->";

    if let Some(start_idx) = original.find(start_tag) {
        if let Some(end_idx) = original[start_idx..].find(end_tag) {
            let mut result = original[..start_idx].to_string();
            result.push_str(&rendered);
            result.push_str(&original[start_idx + end_idx + end_tag.len()..]);
            return result;
        }
    }

    // If not found, prepend to the file
    let mut result = rendered;
    result.push_str("\n\n");
    result.push_str(original);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rules_injection_new_file() {
        let content = "My custom agent instructions.";
        let injected = inject_rules_block(content, "MyWorkspace", "http://server");
        assert!(injected.contains("# Co-Force Team Protocol — MyWorkspace"));
        assert!(injected.contains("Server: http://server"));
        assert!(injected.contains("My custom agent instructions."));
    }

    #[test]
    fn test_rules_injection_existing_block() {
        let content = "Before block.\n<!-- CO-FORCE:BEGIN v1 -->Old rules<!-- CO-FORCE:END -->\nAfter block.";
        let injected = inject_rules_block(content, "MyWorkspace", "http://server");
        assert!(injected.contains("Before block."));
        assert!(injected.contains("# Co-Force Team Protocol — MyWorkspace"));
        assert!(injected.contains("After block."));
        assert!(!injected.contains("Old rules"));
    }
}
