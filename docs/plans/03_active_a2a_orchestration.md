# Kế Hoạch Triển Khai Chi Tiết: 03 - Active A2A Orchestration Layer

**Status:** Ready for Implementation
**Target:** `crates/co-force-core/src/orchestration/` và Event Bus

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
Khi nhận MCP Request `co_force_handover`:
1. Use Case cập nhật Task Status thành `PendingHandover`, lưu `state_summary` vào task description.
2. Gọi `LockRepository::release_all_for_agent(agent_id)` để đảm bảo Agent cũ buông toàn bộ lock.
3. Gửi `WorkspaceEvent::HandoverRequested` lên Event Bus.
4. Một task khác bắt được event này, gọi `ProcessManager::spawn_agent(next_provider, task_id, state_summary)`.
5. Trả về `safe_to_exit: true` cho Agent cũ để nó tự sát.

---

## 5. Trình tự Triển khai (Step-by-Step)
1. Thêm `tokio::sync::broadcast` vào Core, định nghĩa Enum `WorkspaceEvent`.
2. Truyền `Sender` vào các Use Cases, bổ sung lệnh `send()` ở cuối mỗi Use Case.
3. Viết module `doc_generator.rs`, triển khai vòng lặp `recv()` và logic replace string bằng Regex (tôn trọng các block code thủ công của user).
4. Viết module `process_mgr.rs`, thiết lập các lệnh bash phù hợp cho Antigravity CLI và Claude Code. Lưu ý gắn cờ `--auto-approve` hoặc cờ tương đương để nó chạy không cần tương tác (Non-interactive mode).
5. Tích hợp Handover Use Case: liên kết giữa việc nhả lock và gọi Process Manager.
6. Viết Integration Test: Mock OS Command để đảm bảo sự kiện Handover thực sự kích hoạt hàm `spawn`.
