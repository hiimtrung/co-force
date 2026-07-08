# Kế Hoạch Triển Khai Chi Tiết: 02 - MCP Server and Use Cases Layer

**Status:** Ready for Implementation
**Target:** `crates/co-force-core/src/engine/` và `crates/co-force-mcp/`

> **⚠️ Cập nhật 2026-07-08 (xem `docs/review_findings.md` F-01):** `rmcp` hiện đã là **2.x stable** — không phải 0.16 như URD. Hai hệ quả cho plan này:
> 1. Macro `#[rmcp::server]` **không tồn tại**. API đúng của rmcp 2.x: `#[tool_router]` trên `impl`, `#[tool(description = ...)]` trên từng method, params là struct derive `serde::Deserialize + schemars::JsonSchema` bọc trong `Parameters<T>`, và implement trait `ServerHandler` (dùng `#[tool_handler]`). Tham khảo examples trong repo `modelcontextprotocol/rust-sdk`.
> 2. **SSE transport đã bị deprecate khỏi MCP spec** — thay bằng **Streamable HTTP** (feature `transport-streamable-http-server` + `transport-streamable-http-server-session` để bind session, phục vụ Implicit Session Binding qua header `Mcp-Session-Id`). Các đoạn code mẫu bên dưới mang tính minh họa cấu trúc, khi code phải theo API thật.

## 1. Context & Mục Tiêu
Tầng này đóng vai trò xử lý logic nghiệp vụ thông qua các **Use Case Classes/Structs** (Clean Architecture) và bộc lộ (expose) các tools đó cho client AI Agents thông qua giao thức MCP (Model Context Protocol).

*Tài liệu tham chiếu:*
- `architecture.md` §6 (response envelope, error codes, catalog 38 tools)
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

Dùng rmcp **2.x** (`#[tool_router]` + `#[tool]` + `ServerHandler` — xem banner đầu file). Code mẫu bên dưới giữ nguyên dạng minh họa cấu trúc cũ; khi code phải theo API thật.

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
#[tool_router] // API thật rmcp 2.x — kèm #[tool_handler] impl ServerHandler; params bọc Parameters<T> (derive JsonSchema)
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
1. **Stdio Transport** (`transport-io`): Giao tiếp qua stdin/stdout — dành cho single-agent hoặc client không nói HTTP.
2. **Streamable HTTP Transport** (`transport-streamable-http-server`): HTTP server tại `127.0.0.1:3846/mcp` — **chế độ mặc định** (nhiều agent chia sẻ 1 server, có session binding qua `Mcp-Session-Id`). Đây là transport thay thế SSE đã bị deprecate.

```rust
// Trong main.rs (minh họa — theo API rmcp 2.x: serve_server + StreamableHttpService)
match args.transport {
    Transport::Stdio => {
        let service = CoForceMcp::new(/* use cases */);
        rmcp::serve_server(service, rmcp::transport::io::stdio()).await?;
    }
    Transport::Http { addr } => {
        // StreamableHttpService mount vào axum Router tại /mcp,
        // cùng listener phục vụ luôn /dashboard (quyết định F-13)
    }
}
```

---

## 5. Trình tự Triển khai (Step-by-Step)
1. Trong `co-force-core`, tạo folder `engine/` và lần lượt viết Unit Test (bằng `mockall`) cho các Use Case. 
2. Triển khai logic thực tế cho các Use Case đến khi pass bài Unit Test. Đảm bảo MỌI Use Case đều có cơ chế log activity.
3. Trong `co-force-mcp/Cargo.toml`, thêm thư viện `rmcp` và `tokio`.
4. Viết `main.rs`, cài đặt `CoForceMcp` struct.
5. Cài đặt các trait method bằng macro `#[tool]`. Đảm bảo copy chính xác Tool Signatures từ Appendix B của URD.
6. Thêm bộ parser CLI args (`clap`) để chọn Transport mode (Streamable HTTP / Stdio).
7. Khởi chạy thử: `cargo run -p co-force-mcp -- --transport stdio` và nhập JSON-RPC tay để kiểm thử vòng ngoài.
