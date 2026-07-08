# Kế Hoạch Triển Khai Chi Tiết: 03 - Active A2A Orchestration Layer

**Status:** Ready for Implementation
**Target:** `crates/co-force-core/src/orchestration/` và Event Bus

> **⚠️ Cập nhật 2026-07-08 (v2 — mô hình production):** Trong production, server là máy remote sau cloudflared tunnel — **server không thể spawn process trên máy client**, và workspace code nằm ở client. Vì vậy spawn/handover chạy theo **mô hình 3 lane** (chốt tại `architecture.md` §5):
> - **Lane 2 (spawn-by-directive):** `co_force_spawn_agent(placement:"local")` → server KHÔNG spawn mà trả `spawn_directive {command, env, cwd}` + scoped token; agent yêu cầu tự chạy lệnh bằng shell tool của nó. `ProcessManager` phía server chỉ validate + sinh directive + giám sát check-in của agent con (timeout 120s).
> - **Lane 3 (server worker pool):** spawn headless TRÊN server trong **git worktree sandbox** (`/var/lib/co-force/workspaces/{wsId}/jobs/{taskId}`) từ mirror clone qua deploy key — dùng cho reviewer/critic auto-staffing và handover khi client offline. Code mẫu §4 dưới đây áp dụng cho lane này, bổ sung: fetch mirror + tạo worktree trước khi spawn, cgroup/nice limit, token budget, xóa worktree sau job.
> - Ma trận chọn lane + luồng handover chi tiết: `architecture.md` §5.4. Handover ưu tiên: commit + push WIP branch → L3 tiếp tục.
> - **Solo orchestration (Plan 10):** L2 mở rộng cho kịch bản 1 agent tự bootstrap cả team trên 1 máy — spawn nhận `taskIds[]`, bootstrap prompt hẹp (context sạch chống hallucinate), quy tắc git tường minh chống race trên cùng working tree, option `use_local_worktrees` cô lập tuyệt đối, stall detector báo PM qua inbox.
> - **Doc Generator (§3 bên dưới) chỉ ghi file nơi server có filesystem access:** worker worktrees (L3) và biến thể LAN. Với client remote, server **không thể** ghi `AGENTS.md`/`.cursorrules`/`session_status.json` vào workspace — state động deliver **in-band** qua response envelope (`workspace_pulse`, `inbox` — architecture.md §5.6); rules tĩnh do enrollment script ghi (Plan 05). Bản render AGENTS.md động expose tại `GET /api/workspaces/{id}/agents.md` cho dashboard.

## 1. Context & Mục Tiêu
Đây là module cốt lõi nâng cấp Co-Force thành **Active A2A Orchestrator**. Thay vì chỉ bị động trả lời MCP requests, server có khả năng theo dõi state thay đổi (qua Event Bus), tự động sinh tài liệu định hướng (`AGENTS.md`), và chủ động đẻ nhánh (spawn) các agent mới bằng OS Process (dành cho tính năng chia việc hoặc fallback/handover).

*Tài liệu tham chiếu:*
- `URD.md` (Section 14.2 Event-Driven Architecture)
- `URD.md` (Group I: Active A2A Orchestration - UC-37, UC-38)
- `URD.md` (UC-36: Dynamic AGENTS.md Generation)

---

## 2. In-Memory Event Bus
**Vị trí:** `crates/co-force-core/src/orchestration/bus.rs`

Giải quyết bài toán kết dính rời rạc (Decoupling) giữa MCP handlers và các Background Tasks.

### 2.1 Định nghĩa Sự Kiện (Events)
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
Tạo một instance của `tokio::sync::broadcast::Sender` và chia sẻ cho toàn bộ các Use Case thông qua `Arc`.
```rust
// Khởi tạo ở main.rs hoặc lib.rs
let (tx, _rx) = tokio::sync::broadcast::channel::<WorkspaceEvent>(1024);
let bus_sender = Arc::new(tx);
```
Mỗi Use Case (như `LockFilesUseCase`) khi làm xong nghiệp vụ DB, sẽ gọi `bus_sender.send(WorkspaceEvent::FilesLocked { ... }).ok();`.

---

## 3. Dynamic AGENTS.md Generator
**Vị trí:** `crates/co-force-core/src/orchestration/doc_generator.rs`

Background task lắng nghe Event Bus, gom nhóm (debounce) và tự động ghi đè nội dung file AGENTS.md.

### 3.1 Vòng lặp Daemon
```rust
pub async fn run_doc_generator(
    mut rx: tokio::sync::broadcast::Receiver<WorkspaceEvent>,
    agent_repo: Arc<dyn AgentRepository>,
    task_repo: Arc<dyn TaskRepository>
) {
    loop {
        // Lắng nghe sự kiện
        let Ok(event) = rx.recv().await else { break };
        
        // Debounce: Đợi 2-3s xem có event nào nữa không (tránh ghi ổ cứng liên tục)
        // ... (tokio::time::sleep logic)
        
        // Query DB lấy state mới nhất
        let agents = agent_repo.list_active(...).await;
        let tasks = task_repo.find_pending(...).await;
        
        // Format thành Markdown
        let md_content = format_to_managed_block(&agents, &tasks);
        
        // Ghi vào .co-force/AGENTS.md (sử dụng Regex để chỉ thay thế trong phần BEGIN/END marker)
        write_managed_block(".co-force/AGENTS.md", &md_content).await;
        
        // Ghi kèm vào .cursorrules và .clauderules nếu file tồn tại
    }
}
```

---

## 4. OS Process Manager (Spawn & Kill Agents)
**Vị trí:** `crates/co-force-core/src/orchestration/process_mgr.rs`

Chịu trách nhiệm thực thi các lệnh hệ thống để đẻ ra các agent ẩn (Sub-agents).

### 4.1 Cấu trúc ProcessManager
```rust
use tokio::process::Command;

pub struct ProcessManager;

impl ProcessManager {
    /// Spawn một agent CLI trong chế độ background (detached)
    /// LƯU Ý (F-05/F-23): match hardcode dưới đây CHỈ là minh họa — bản thật đọc
    /// command template từ config.toml [providers] (provider registry), không match trong Rust.
    pub async fn spawn_agent(provider: &str, task_id: &str, context: &str) -> anyhow::Result<u32> {
        let mut cmd = match provider {
            "antigravity" => {
                let mut c = Command::new("antigravity-cli");
                c.arg("--task").arg(context);
                c.arg("--auto-approve"); // Rất quan trọng: agent ẩn không được block hỏi user
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
        
        // Tuỳ chọn: Có thể spawn task để wait() child này nhằm dọn dẹp (reap) zombie process.
        Ok(pid)
    }
}
```

### 4.2 Use Case: Handover (Chạy tiếp gậy)
> **⚠️ Flow dưới đây là bản gốc, đã được thay bằng §5 (cross-provider, escrow locks):** bước 2 "nhả toàn bộ lock" tạo lỗ hổng — agent thứ 3 chen vào claim files giữa lúc chuyển giao. Bản chốt: locks đi **escrow theo task** và chuyển atomic cho người kế nhiệm.

Khi nhận MCP Request `co_force_handover`:
1. Use Case cập nhật Task Status thành `PendingHandover`, lưu `state_summary` vào task description.
2. ~~Gọi `LockRepository::release_all_for_agent(agent_id)`~~ → locks vào escrow gắn task (§5.3).
3. Gửi `WorkspaceEvent::HandoverRequested` lên Event Bus.
4. Chọn target theo ma trận §5.4 (agent online khác → offer; hoặc spawn L2/L3).
5. Trả về `safe_to_exit: true` cho Agent cũ để nó tự thoát.

---

## 5. Cross-Provider Handover — kịch bản chuẩn: Claude CLI chạm rate limit, agy CLI tiếp quản

**Sự thật nền tảng:** conversation/context KHÔNG port được giữa 2 provider (`claude --resume` vô nghĩa với `agy`). Vì vậy "đem context sang" = **externalize toàn bộ trạng thái ra server + git**, và agent kế nhiệm tái tạo context từ đó. Đây chính là giá trị cốt lõi của Co-Force: context sống ở server, agent chỉ là worker tạm thời.

### 5.1 Context được chuyển qua 5 kênh (không kênh nào phụ thuộc provider)

| Kênh | Chứa gì | Ai ghi |
| :--- | :--- | :--- |
| Task record (DB) | spec, use cases, verification plan, revision, gate hiện tại | create_tasks + update_task |
| **Handover package** (bảng `handovers`) | §5.2 — done/remaining, decisions, gotchas, next steps | Agent cũ lúc handover |
| Activity stream | journal mọi tool call của agent cũ (append-only) | Tự động |
| Memory (`recall`) | knowledge/gotchas đã distill | store_memory + nightly distill |
| Code state (git) | WIP branch + commit_sha, hoặc working tree local | Agent cũ commit/push |

### 5.2 Handover package (validated — chất lượng bàn giao cũng là quality gate)

```json
{
  "reason": "rate_limit",                     // rate_limit | context_exhaustion | session_end | manual
  "provider_cooldown_until": "2026-07-08T21:00:00Z",  // thời điểm limit reset (nếu CLI báo)
  "progress": {"done": ["API skeleton", "3/5 tests"], "remaining": ["wire auth", "2 tests còn fail"]},
  "decisions": [{"what": "dùng middleware X", "why": "..."}],
  "gotchas": ["test Y flaky khi chạy song song", "đừng đổi signature Z — client cũ phụ thuộc"],
  "code_state": {"kind": "pushed_wip", "branch": "co-force/t42", "commit_sha": "a1b2c3",
                  "files_touched": ["src/auth.rs", "tests/auth_test.rs"]},
  "next_steps": ["chạy cargo test -p core trước", "bắt đầu từ TODO trong auth.rs:88"]
}
```

Server dùng **reasoner validate độ đầy đủ** (thiếu `remaining`/`next_steps` hoặc mơ hồ → lỗi `HANDOVER_INCOMPLETE` + recovery_action chỉ rõ thiếu gì) — bàn giao cẩu thả bị chặn như mọi gate khác.

### 5.3 Flow chủ động (Claude thấy cảnh báo rate limit — còn gọi được tool)

1. **Rules dạy handover SỚM** (Plan 09): thấy cảnh báo limit đầu tiên → KHÔNG bắt đầu việc mới; commit + push WIP → gọi `co_force_handover(taskId, reason="rate_limit", resetAt, package)`.
2. Server validate package → task → `pending_handover`; **locks vào escrow gắn task** (không nhả tự do — chuyển atomic cho người kế nhiệm, agent thứ 3 không chen được).
3. Ghi **provider cooldown**: `provider_status[machine][claude-code].rate_limited_until = resetAt` → plan_team/staffing/delegation tránh giao việc cho provider này đến khi hết cooldown (Plan 08).
4. Chọn target theo §5.4 → agent kế nhiệm nhận `handover_offer` qua inbox (agy đang online cùng feature nhận NGAY nếu đang `wait_events`).
5. agy accept → server chuyển **assignee + locks atomic**, task về `in_progress`; response kèm package + `protocol_next_step: "Read package.next_steps, checkout branch co-force/t42, run co_force_recall on the feature topic before coding."`
6. Claude nhận `safe_to_exit: true`. Hết cooldown → Claude check-in lại nhận việc mới bình thường.

### 5.4 Ma trận chọn target (mở rộng architecture §5.4)

| Điều kiện | Target |
| :--- | :--- |
| Agent provider khác đang **online** cùng workspace, có capacity (agy trong use case này) | **Offer qua inbox** — nhanh nhất, agy đã có sẵn context feature qua các lần pulse/inbox trước |
| Không ai online, code **đã push** WIP | **L3 worker** provider khác (đọc từ mirror @ commit_sha) |
| Code **local chưa push được** (chưa kịp/không có remote) | **L2 spawn directive cho agy trên CÙNG MÁY** — đọc thẳng working tree; directive phải được Claude chạy TRƯỚC khi cạn hẳn (lý do rules dạy handover sớm) |
| Không phương án nào khả thi | Task đứng `pending_handover` + alert user + cooldown ghi nhận; timeout → về backlog (Plan 07 §3) |

### 5.5 Flow bị động (Claude chết đột ngột giữa chừng — không kịp handover)

1. Session drop → grace 2 phút → reclaim (architecture §9). Điểm mới: reclaim **không chỉ trả task về backlog** — nếu có agent online provider khác → tự động gửi `handover_offer` với **package tổng hợp bởi server**: task record + activity stream gần nhất + git state (không có summary xịn của agent cũ).
2. Đây là lý do rules bắt `update_task` ghi progress notes thường xuyên — **mỗi update_task là bảo hiểm handover**; chết đột ngột thì journal chính là bàn giao.
3. Phát hiện nguyên nhân: L2/L3 worker exit với stderr chứa pattern rate-limit của provider (parser per provider — Plan 08 C4 mở rộng) → server ghi cooldown như flow chủ động.

### 5.6 Schema bổ sung (WS-A)

```sql
CREATE TABLE IF NOT EXISTS handovers (
    handover_id  TEXT PRIMARY KEY,
    task_id      TEXT NOT NULL,
    from_agent_id TEXT NOT NULL,
    to_agent_id  TEXT,              -- NULL đến khi có người nhận
    reason       TEXT NOT NULL,     -- rate_limit | context_exhaustion | session_end | manual
    package      TEXT NOT NULL,     -- JSON §5.2 (đã qua validate)
    provider_cooldown_until TIMESTAMP,
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    accepted_at  TIMESTAMP,
    FOREIGN KEY (task_id) REFERENCES tasks(task_id)
);
-- provider_status (trong server.db): machine_id, provider, rate_limited_until, last_error
```

---

## 6. Trình tự Triển khai (Step-by-Step)
1. Thêm `tokio::sync::broadcast` vào Core, định nghĩa Enum `WorkspaceEvent`.
2. Truyền `Sender` vào các Use Cases, bổ sung lệnh `send()` ở cuối mỗi Use Case.
3. Viết module `doc_generator.rs`, triển khai vòng lặp `recv()` và logic replace string bằng Regex (tôn trọng các block code thủ công của user).
4. Viết module `process_mgr.rs` — command templates lấy từ **provider registry trong config** (quyết định F-05, không hardcode provider trong Rust). Spec đã verify cho từng CLI (Claude Code `claude -p`, Codex `codex exec`, Antigravity `agy -p`, kèm caveats C1–C4): **Plan 08 §3**.
5. Handover use case theo §5: bảng `handovers` + `provider_status` (server.db), lock escrow + chuyển atomic (unit test: agent thứ 3 không claim được trong lúc pending_handover), package validator qua reasoner (mock), target matrix §5.4, cooldown tracking.
6. Reclaim mở rộng (§5.5): re-dispatch tự động sang agent provider khác + package server tổng hợp; stderr rate-limit parser per provider.
7. Viết Integration Test: (a) Mock OS Command đảm bảo Handover kích hoạt `spawn`; (b) **kịch bản chuẩn cross-provider**: mock 2 sessions (claude + agy), claude handover(reason=rate_limit) → agy nhận offer, locks chuyển atomic, task tiếp tục không rơi gate nào; (c) claude kill -9 → sau grace, agy nhận offer với package server tổng hợp.
