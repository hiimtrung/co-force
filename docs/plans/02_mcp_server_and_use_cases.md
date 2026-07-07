# Implementation Plan: 02 - MCP Server and Use Cases

**Status:** Plan
**Target:** `crates/co-force-core/src/engine/` và `crates/co-force-mcp/`

## 1. Overview
Phần này định nghĩa cách Co-Force đóng gói các logic (Use Cases) theo chuẩn Single Responsibility và làm thế nào MCP Server giao tiếp với các client agent.

## 2. Giải pháp Kỹ thuật

### 2.1 Use Case Engine
*Tham chiếu:* `implementation_instructions.md` (Section 3.3 Unit Testing a Use Case), `URD.md` (Section 14.1 Clean Architecture)

Tất cả logic nghiệp vụ đều nằm trong các Use Cases, nhận đầu vào là các Repositories qua Trait (Dependency Injection).
- **Vị trí:** `crates/co-force-core/src/engine/`
- **Các Use Case Cốt lõi:**
  - `CheckInUseCase`: Đăng ký agent, phân tích trạng thái workspace.
  - `LockFilesUseCase`: Kiểm tra conflict, ghi file lock.
  - `CreateTasksUseCase`: Nhận danh sách task từ agent, cấp ID, lưu DB.
  - `UpdateTaskUseCase`: Đổi trạng thái task (Draft -> InProgress -> Completed).
  - `StoreMemoryUseCase`: Băm (Chunking) -> Embed -> Classify -> Store.

### 2.2 MCP Server Transport
*Tham chiếu:* `implementation_instructions.md` (Section 4 & 6)

Sử dụng macro `#[rmcp::server]` để tự động sinh JSON-RPC handlers. Hỗ trợ song song 2 transport:
- **Stdio Transport:** Dành cho chế độ Localhost, CLI agent (như Claude CLI) chạy Co-Force như một child process.
- **SSE Transport:** Dành cho chế độ LAN/Server hoặc IDE (Cursor/Windsurf) kết nối qua HTTP. `tokio` đảm nhiệm concurrent runtime.

### 2.3 Tool Signatures Registration
*Tham chiếu:* `URD.md` (Appendix B: MCP Tool Signatures)

Trong `crates/co-force-mcp/src/main.rs`, tạo struct `CoForceMcp` chứa các `Arc<UseCase>`. Gắn attribute `#[tool]` cho từng phương thức:
- Cần ánh xạ đúng Input (JSON) sang Struct Request của Use Case.
- Ví dụ: `co_force_check_in`, `co_force_lock_files`, `co_force_recall`, `co_force_delegate_task`.
- Trả về `serde_json::Value` (JSON Object) cho client theo đúng Schema URD.

## 3. TDD & Tích hợp
*Tham chiếu:* `implementation_instructions.md` (Section 3.4 Integration Tests)

Viết Test Tích hợp chạy thực tế các Use Case qua In-Memory DB (`tokio-rusqlite`), ví dụ: `integration_lock_flow.rs` sẽ test toàn bộ flow Check-in -> Lock -> Conflict.
