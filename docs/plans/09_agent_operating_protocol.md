# Kế Hoạch Triển Khai Chi Tiết: 09 - Agent Operating Protocol (Onboarding & Hành vi Đồng nhất)

**Status:** Ready for Implementation (bổ trợ WS-B/C/G — chốt 2026-07-08)
**Target:** `crates/co-force-core/src/workspace/protocol_templates/` (rules template + guide renderer), tool descriptions trong `co-force-mcp/src/tools/`
**Trả lời câu hỏi:** *Sau khi client setup xong, Claude Code (hay CLI bất kỳ) làm sao biết dùng tool nào để làm gì, điểm khởi đầu ở đâu, và làm sao mọi agent hành xử giống nhau?*

Thay thế URD §9.3 (template cũ có chỉ dẫn sai: dọa "OS Permission Denied"/chmod ban không còn tồn tại sau khi Lớp 4 đổi hình thái in-band — architecture §5.6; flow cũ `update_task(completed)` nay trả `GATE_VIOLATION`).

---

## 1. Chuỗi khám phá của một agent lạnh (cold start) — 4 điểm chạm

Một agent mới mở phiên không biết gì về Co-Force. Nó "học" protocol qua đúng 4 điểm chạm, theo thứ tự thời gian:

| # | Điểm chạm | Khi nào | Agent nhận được gì |
| :- | :--- | :--- | :--- |
| 1 | **Rules file** (`AGENTS.md`/`CLAUDE.md`/`.cursorrules` — managed block §2, enrollment script tiêm) | Client tự nạp vào system context khi mở project | Điểm khởi đầu (check_in), vòng đời task, quy tắc hành vi, bản đồ tool §2.4 |
| 2 | **Tool descriptions** (Lớp 2 — §3) | Khi client nạp danh sách 39 tools từ server | Mỗi tool tự nói khi nào PHẢI dùng nó ("MANDATORY: call first...") |
| 3 | **`co_force_check_in` response** | Tool call đầu tiên của phiên | Pending tasks + team online + inbox tồn + `protocol_next_step` + `onboarding: true` lần đầu → được dẫn tới `co_force_guide()` |
| 4 | **Mọi tool response sau đó** (envelope §6.2 architecture) | Suốt phiên | `protocol_next_step` chỉ hành động kế tiếp, `inbox` đẩy việc team, error + `recovery_action` tự sửa sai |

Nguyên tắc thiết kế: **agent không cần nhớ protocol** — protocol tự tìm đến agent ở mọi bước. Rules file chỉ cần đủ để bước 3 xảy ra; từ đó server dẫn dắt in-band.

---

## 2. Rules Template chốt (managed block — enrollment script tiêm, Plan 05 §3 bước 5)

Nguyên tắc viết: (a) **tiếng Anh** (mọi CLI tuân thủ tốt nhất); (b) **chỉ hứa những gì server thật sự enforce** — không dọa suông kiểu "OS sẽ chặn" (agent phát hiện nói dối sẽ mất tin toàn bộ rules); (c) ngắn đủ nằm trong context mọi phiên, chi tiết dồn cho `co_force_guide()`; (d) có version để re-enroll cập nhật.

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
| Context nearly full / rate-limit warning | push WIP → `handover` (package đầy đủ) |
| Learned something durable | `store_memory` |
| Who is doing what | `list_agents` / `workspace_status` / `whoami` |

Server: {{server_url}} · All tool names are prefixed `co_force_`.
<!-- CO-FORCE:END -->
```

Biến template: `{{workspace_name}}`, `{{server_url}}` (+ `{{role_hint}}` nếu máy được enroll với role cố định). Cùng một template cho mọi client — khác biệt duy nhất là file đích (`AGENTS.md` dùng chung cho Claude Code/Codex/agy; `.cursorrules` cho Cursor — Plan 08 §3 cột rules).

---

## 3. Lớp 2 — Chuẩn viết Tool Descriptions (nhất quán với rules)

Tool description là "rules tại điểm sử dụng" — agent đọc nó ngay lúc chọn tool. Chuẩn bắt buộc khi implement (`co-force-mcp/src/tools/`):

1. **Tool cổng vào nói rõ tính bắt buộc:** `check_in` = "MANDATORY first call of every session. All other tools fail with CHECK_IN_REQUIRED until called."
2. **Tool có precondition nói rõ precondition:** `lock_files` = "MANDATORY before editing any file. Requires an approved task."; `submit_verification` = "The ONLY way to move a task toward completed. Requires real test evidence + commit_sha."
3. **Tool nguy hiểm nói rõ hậu quả:** `update_task` = "Cannot set completed (GATE_VIOLATION) — use submit_verification."
4. **Không hứa điều server không làm** (đồng bộ nguyên tắc §2b).
5. Description là contract — đổi description = đổi protocol → cùng PR phải cập nhật template §2 và guide §4 nếu lệch.

## 4. `co_force_guide()` — Onboarding động (chi tiết mà rules tĩnh không chứa)

Server render theo trạng thái workspace thật (không phải markdown tĩnh):
- Quality policy đang bật (reviews_required, reviewer_must_differ, evidence kinds) — "vì sao task của bạn cần X".
- Team hiện tại: agents + roles + ai đang giữ lock nào; task backlog đang chờ claim.
- **3 ví dụ tool-call đúng chuẩn** bám đúng policy hiện hành (create_tasks đủ trường, submit_verification đủ evidence, submit_review đủ findings schema).
- Lỗi phổ biến → recovery (bảng error codes architecture §6.3 rút gọn).
- Trigger: response check_in đầu tiên của agent mới có `onboarding: true` + `protocol_next_step: "Call co_force_guide() once before taking any task."`

## 5. Playbook theo role (server gửi qua guide + review_request payload)

| Role | Vòng lặp chuẩn |
| :--- | :--- |
| `developer` | check_in → recall → (claim task approved hoặc create_tasks) → lock → code → submit_verification → xử lý findings → store_memory |
| `reviewer` (kể cả worker L3) | check_in(role=reviewer) → `wait_events` loop → nhận review_request (kèm assist checklist) → đọc code thật (worktree/workspace) → tự chạy test độc lập → submit_review(findings có file/line/severity) → quay lại wait_events |
| `critic` | nhận critique_request → submit_critique(position, arguments, risks, alternatives) — phản biện thật, không lịch sự xã giao |
| `pm`/`architect` | create_tasks + request_critique trước quyết định lớn; không tự approve. **PM solo-bootstrap (Plan 10):** plan_team → trình user estimate → spawn subagents → vòng lặp giám sát `wait_events` (xử lý stall/respawn, gom câu hỏi trình user 1 lần) — **không code khi team đang chạy** |

## 6. Ma trận "hành vi đồng nhất" — mỗi quy tắc được lớp nào ép

Hành vi đồng nhất KHÔNG dựa vào thiện chí LLM — mỗi quy tắc trong §2 có ít nhất một lớp cưỡng chế phía server:

| Quy tắc | Lớp 1 (rules) | Lớp 2 (descriptions) | Lớp 3 (interlocking server) | Lớp 4 (in-band state) |
| :--- | :---: | :---: | :---: | :---: |
| Check-in trước mọi việc | ✓ | ✓ | ✓ `CHECK_IN_REQUIRED` chặn 37 tools còn lại | |
| Lock trước khi sửa | ✓ | ✓ | ✓ `LOCK_CONFLICT` khi claim trùng | ✓ pulse hiện lock của team |
| Không tự set completed | ✓ | ✓ | ✓ `GATE_VIOLATION` | |
| Evidence thật, đúng revision | ✓ | ✓ | ✓ validator + `EVIDENCE_STALE` (F-21) | |
| Không tự review bài mình | ✓ | | ✓ separation-of-duties server-side | |
| Xử lý inbox trước | ✓ | | | ✓ inbox + `protocol_next_step` mọi response |
| Tự sửa sai theo recovery_action | ✓ | | ✓ mọi error đều kèm recovery_action | |
| Handover thay vì bỏ ngang | ✓ | ✓ | ✓ reclaim sau grace 2' nếu bỏ ngang | ✓ pulse cảnh báo |

→ Agent "bướng" nhất cũng hội tụ về đúng flow vì **con đường sai đều bị chặn kèm chỉ dẫn con đường đúng** (self-correction loop). Rules Lớp 1 chỉ giúp hội tụ nhanh (ít lượt lỗi hơn), không phải hàng rào duy nhất.

## 7. Trình tự Triển khai (Step-by-Step)

1. Template §2 thành file trong `workspace/protocol_templates/rules_v1.md` (kèm version constant); writer managed-block dùng chung với Plan 03/05; golden-file test render với biến mẫu.
2. Chuẩn hóa 39 tool descriptions theo §3 (bảng description nằm cạnh handler; review chéo với template để không lệch).
3. `co_force_guide` renderer (§4): input = quality policy + team snapshot + backlog; output markdown; unit test với policy khác nhau cho ra ví dụ khác nhau.
4. Cờ `onboarding: true` trong check_in response (agent chưa từng check-in workspace này) + `protocol_next_step` trỏ guide.
5. Playbook §5: nhúng vào guide theo role; review_request payload kèm checklist + nhắc quy trình reviewer.
6. **E2E "cold agent"** (nghiệm thu của plan này): container sạch → enroll → mở Claude Code thật với 1 prompt trung tính ("add a hello endpoint") → assert agent tự: check_in → recall → create_tasks → dừng chờ approve (không sửa file trước lock) — lặp lại với Codex + agy (Plan 08).
