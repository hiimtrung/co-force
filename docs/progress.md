# Co-Force: Subagent Progress Tracker

> **QUAN TRỌNG:** Đây là file đồng bộ trạng thái (Keep Track) trung tâm. 
> Bất kỳ Agent hay Subagent (PM, DEV, TEST, QA) nào khi bắt đầu công việc ĐỀU PHẢI đọc file này để tránh Race Condition. Khi bắt đầu một task, phải ghi rõ `[Đang xử lý bởi <Tên Subagent>]`. Khi xong phải đánh dấu `[x]` và cập nhật kết quả.

## Tiêu chuẩn Trạng thái:
- `[ ]` Chưa bắt đầu
- `[Đang xử lý bởi PM]` Đang lên kế hoạch/phân tích
- `[Đang xử lý bởi DEV]` Đang code hoặc viết Test (TDD)
- `[Đang xử lý bởi TEST]` Đang chạy test/kiểm thử
- `[Đang xử lý bởi QA]` Đang review/linting
- `[x]` Hoàn thành

---

## Sprint Hiện tại: Triển khai Kế hoạch Kiến trúc

### 1. Database and Domain Layer (Plan 01)
- `[ ]` Setup Cargo.toml dependencies (serde, tokio, rusqlite, mockall)
- `[ ]` Định nghĩa Strong Types (AgentId, TaskId, WorkspaceId...) trong `types/mod.rs`
- `[ ]` Định nghĩa Enums (AgentState, TaskStatus, ActivityType...)
- `[ ]` Định nghĩa Core Structs (Agent, Task, AgentActivity, SharedContext)
- `[ ]` Triển khai SQLite Migrations (001_initial.sql)
- `[ ]` Định nghĩa Repository Traits (AgentRepository, LockRepository...) trong `engine/ports.rs`

### 2. MCP Server and Use Cases (Plan 02)
- `[ ]` Xây dựng `CheckInUseCase` (TDD, Unit Test trước)
- `[ ]` Xây dựng `LockFilesUseCase`
- `[ ]` Setup `co-force-mcp/src/main.rs` với struct `CoForceMcp`
- `[ ]` Đăng ký các tool handlers với macro `#[rmcp::server]`
- `[ ]` Cấu hình Transport Layer (stdio / sse) dựa trên CLI args

### 3. Active A2A Orchestration (Plan 03)
- `[ ]` Khởi tạo In-Memory Event Bus (`tokio::sync::broadcast`)
- `[ ]` Viết module Dynamic AGENTS.md Generator (`doc_generator.rs`)
- `[ ]` Viết module Process Manager (`process_mgr.rs`) để spawn lệnh OS
- `[ ]` Triển khai tool `co_force_spawn_agent`
- `[ ]` Triển khai tool `co_force_handover`

### 4. Agentic RAG and LLM (Plan 04)
- `[ ]` Định nghĩa `LlmProvider` interface
- `[ ]` Triển khai `OllamaProvider` (reqwest `/api/embeddings`, `/api/generate`)
- `[ ]` Viết thuật toán `agentic_chunking` (Structural Splitting & Semantic Boundary)
- `[ ]` Tích hợp Vector Search và Fallback logic vào Memory Use Case

---

## Log Báo cáo (Subagent Reports)
*(Các subagent ghi chú lỗi, kết quả test, hoặc report cho Agent gốc tại đây)*
- **[Hệ thống]**: Khởi tạo file tracking. Sẵn sàng cho PM subagent phân bổ việc.
