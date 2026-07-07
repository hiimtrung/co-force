# Implementation Plan: 01 - Database and Domain Layer

**Status:** Plan
**Target:** `crates/co-force-core`

## 1. Overview
Phần này tập trung vào thiết kế Tầng Domain (lõi nghiệp vụ) và Tầng Database (SQLite), đảm bảo theo đúng Clean Architecture.

## 2. Giải pháp Kỹ thuật

### 2.1 Strong Domain Types
*Tham chiếu:* `implementation_instructions.md` (Section 3.1)

Tất cả các định danh và Enum phải dùng Strong Typing để tránh lỗi nhầm lẫn ID tại compile time.
- **Vị trí:** `crates/co-force-core/src/types/mod.rs`
- **Các type cần tạo:**
  - Định danh: `AgentId(String)`, `WorkspaceId(String)`, `TaskId(String)`, `ActivityId(String)`, `ContextId(String)`.
  - Enums: `AgentState`, `TaskStatus`, `ActivityType`, `ContextType`.
  - Core Structs: `Agent`, `Task`, `FileLock`, `MemoryEntry`, `Skill`, `AgentActivity`, `SharedContext`.

### 2.2 Schema & Migrations
*Tham chiếu:* `implementation_instructions.md` (Section 5), `URD.md` (Section 5 Database Schema)

Sử dụng `rusqlite` và `tokio-rusqlite` cho asycn SQLite. Migration chạy khi app start bằng `include_str!`.

- **Vị trí:** `crates/co-force-core/src/db/migrations/001_initial.sql`
- Bảng cốt lõi (từ `implementation_instructions.md`): `agents`, `tasks`, `file_locks`, `memory_entries`, `skills`, `embedding_cache`.
- Bảng mới bổ sung (từ `URD.md` Group H):
  - `agent_activities`: Ghi log dạng append-only.
  - `shared_contexts`: Hỗ trợ lazy resolution và @ mentions.

### 2.3 Repository Interfaces (Ports Layer)
*Tham chiếu:* `implementation_instructions.md` (Section 3.2), `URD.md` (Section 14.1 Clean Architecture)

Tách biệt giao tiếp CSDL bằng các Trait để TDD dễ dàng thông qua `mockall`.
- **Vị trí:** `crates/co-force-core/src/engine/ports.rs`
- **Các Traits:**
  - `AgentRepository`: `upsert`, `find_by_id`, `list_active`.
  - `TaskRepository`: `create`, `update_status`, `find_pending`.
  - `LockRepository`: `acquire`, `release`, `find_conflict`.
  - `ActivityRepository` (Mới): `log_activity`, `get_workspace_stream`.
  - `ContextRepository` (Mới): `share_context`, `get_unresolved`.

## 3. TDD Approach
*Tham chiếu:* `implementation_instructions.md` (Section 3.3)

Trước khi implement SQLite, viết Unit Tests cho các Use Cases (ví dụ `CheckInUseCase`) bằng cách mock các Repository Traits. Điều này đảm bảo logic hoạt động độc lập với DB engine.
