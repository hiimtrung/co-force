# Kế Hoạch Triển Khai Chi Tiết: 02 - MCP Server and Use Cases Layer

**Status:** Ready for Implementation
**Target:** `crates/co-force-core/src/engine/` và `crates/co-force-mcp/`

## 1. Context & Mục Tiêu
Tầng này đóng vai trò xử lý logic nghiệp vụ thông qua các **Use Case Classes/Structs** (Clean Architecture) và bộc lộ (expose) các tools đó cho client AI Agents thông qua giao thức MCP (Model Context Protocol).

*Tài liệu tham chiếu:*
- `implementation_instructions.md` (Section 3.3, 3.4, 4, 6)
- `URD.md` (Appendix B: MCP Tool Signatures)

---

## 2. Thiết kế Use Case Engine
**Vị trí:** `crates/co-force-core/src/engine/`

Tất cả logic phải nằm trong core, không nằm trong file của MCP Server. Mỗi Use Case là một struct nhận `Arc<dyn Trait>` qua hàm `new`.

### 2.1 Ví dụ Mẫu: CheckInUseCase
```rust
use crate::engine::ports::{AgentRepository, ActivityRepository};
use crate::types::*;
use std::sync::Arc;

pub struct CheckInRequest {
    pub workspace_path: String,
    pub agent_name: String,
    pub role: String,
    pub agent_id: Option<String>,
}

pub struct CheckInResponse {
    pub agent_id: String,
    pub onboarding_required: bool,
    pub pending_tasks: Vec<Task>,
}

pub struct CheckInUseCase {
    agent_repo: Arc<dyn AgentRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
}

impl CheckInUseCase {
    pub fn new(agent_repo: Arc<dyn AgentRepository>, activity_repo: Arc<dyn ActivityRepository>) -> Self {
        Self { agent_repo, activity_repo }
    }

    pub async fn execute(&self, req: CheckInRequest) -> anyhow::Result<CheckInResponse> {
        // 1. Phân tích agent_id. Nếu có, tìm trong DB, nếu không tạo mới.
        // 2. Upsert Agent status -> Idle/Working
        // 3. Log Activity: `ActivityType::CheckIn`
        // 4. Lấy pending tasks trả về
        todo!()
    }
}
```

### 2.2 Các Use Cases Khác Cần Triển Khai
- `LockFilesUseCase` (Đòi hỏi `LockRepository`)
- `UpdateTaskUseCase` (Đòi hỏi `TaskRepository`, ghi Activity sau khi update)
- `GetAgentContextUseCase` (Lấy dữ liệu từ Activity Repo & Context Repo)
- `ShareContextUseCase` (Lưu vào Context Repo)

---

## 3. Thiết Kế MCP Server
**Vị trí:** `crates/co-force-mcp/src/main.rs`

Dùng macro `#[rmcp::server]` của thư viện `rmcp` v0.16.

### 3.1 Server Struct
```rust
use rmcp::{ServerHandler, tool};
use co_force_core::engine::*;
use std::sync::Arc;

pub struct CoForceMcp {
    pub check_in: Arc<CheckInUseCase>,
    pub lock_files: Arc<LockFilesUseCase>,
    pub get_agent_ctx: Arc<GetAgentContextUseCase>,
    pub share_ctx: Arc<ShareContextUseCase>,
}
```

### 3.2 Tool Handlers (Macro Implementation)
Gắn mô tả chi tiết vào `description` vì đây là prompt để kích thích Agent gọi tool.

```rust
#[rmcp::server]
impl CoForceMcp {
    #[tool(description = "MANDATORY: Call this first before any workspace action...")]
    async fn co_force_check_in(
        &self,
        workspace_path: String,
        agent_name: String,
        role: String,
        agent_id: Option<String>,
    ) -> serde_json::Value {
        let req = CheckInRequest { workspace_path, agent_name, role, agent_id };
        let res = self.check_in.execute(req).await.unwrap();
        serde_json::to_value(res).unwrap()
    }

    #[tool(description = "Get recent activity and context of another agent...")]
    async fn co_force_get_agent_context(
        &self,
        agent_id: Option<String>,
        include_history: Option<bool>,
    ) -> serde_json::Value {
        // Gọi GetAgentContextUseCase và parse về JSON
        todo!()
    }
}
```

---

## 4. Cấu hình Transport
MCP Server cần chạy ở 1 trong 2 chế độ (nhận tham số qua `clap` CLI arguments):
1. **Stdio Transport:** Giao tiếp qua stdin/stdout (dành cho Claude Code chạy local).
2. **SSE Transport:** Mở HTTP server tại `localhost:3846/sse` (Dành cho Cursor kết nối).

```rust
// Trong main.rs
let server = CoForceMcp { /* init use cases */ };
if args.transport == "sse" {
    rmcp::transport::sse::run_server(server, args.port).await;
} else {
    rmcp::transport::stdio::run_server(server).await;
}
```

---

## 5. Trình tự Triển khai (Step-by-Step)
1. Trong `co-force-core`, tạo folder `engine/` và lần lượt viết Unit Test (bằng `mockall`) cho các Use Case. 
2. Triển khai logic thực tế cho các Use Case đến khi pass bài Unit Test. Đảm bảo MỌI Use Case đều có cơ chế log activity.
3. Trong `co-force-mcp/Cargo.toml`, thêm thư viện `rmcp` và `tokio`.
4. Viết `main.rs`, cài đặt `CoForceMcp` struct.
5. Cài đặt các trait method bằng macro `#[tool]`. Đảm bảo copy chính xác Tool Signatures từ Appendix B của URD.
6. Thêm bộ parser CLI args (`clap` hoặc `pico-args`) để chọn Transport mode (SSE/Stdio).
7. Khởi chạy thử: `cargo run -p co-force-mcp -- --transport stdio` và nhập JSON-RPC tay để kiểm thử vòng ngoài.
