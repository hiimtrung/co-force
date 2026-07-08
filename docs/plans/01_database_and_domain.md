# Kế Hoạch Triển Khai Chi Tiết: 01 - Database and Domain Layer

**Status:** Ready for Implementation
**Target:** `crates/co-force-core`

## 1. Context & Mục Tiêu
Tầng Domain chứa các định nghĩa dữ liệu cốt lõi (Strong Types) và nghiệp vụ bất biến. Tầng Database đảm nhiệm việc lưu trữ thông qua SQLite (dùng `tokio-rusqlite`). Việc thiết kế kỹ tầng này đảm bảo hệ thống không bị lỗi nhầm lẫn ID (nhờ Strong Typing) và các module khác dễ dàng mock data để test (thông qua Repository Traits).

*Tài liệu tham chiếu:*
- `architecture.md` §7 (bố cục dữ liệu 2 tầng: `server.db` + DB per-workspace — F-17)
- `URD.md` (Section 14.1, Group H); Plan 07 §3 (task state machine), Plan 06 §4.1 (`api_tokens` trong `server.db`)

---

## 2. Thiết kế Cấu trúc Dữ liệu (Domain Types)
**File cần tạo:** `crates/co-force-core/src/types/mod.rs`

### 2.1 Strong Types (Định danh)
Sử dụng pattern Newtype để bọc `String`, tránh truyền nhầm `AgentId` vào hàm đòi `TaskId`.
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AgentId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct WorkspaceId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TaskId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ActivityId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ContextId(pub String);
```

### 2.2 Core Enums
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState { Idle, Working, Paused, Disconnected }

// Theo state machine Plan 07 §3 (quality gates + F-20)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Draft, SpecReview, AwaitingApproval, Approved, InProgress,
    Verification, CodeReview, Rework, Completed,
    Blocked, PendingHandover, Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType { CheckIn, TaskStarted, TaskCompleted, FileEdited, MemoryStored, Delegation, LockAcquired, ContextShared }
```

### 2.3 Core Structs
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub agent_id: AgentId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub role: String,
    pub provider: Option<String>,
    pub machine_id: String,
    pub state: AgentState,
    pub current_task_id: Option<TaskId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentActivity {
    pub activity_id: ActivityId,
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub activity_type: ActivityType,
    pub content: Option<serde_json::Value>, // {summary, details}
    pub related_task_id: Option<TaskId>,
    pub related_files: Option<Vec<String>>,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedContext {
    pub context_id: ContextId,
    pub workspace_id: WorkspaceId,
    pub source_agent_id: AgentId,
    pub target_agent_id: Option<AgentId>,
    pub context_type: String,
    pub content: serde_json::Value,
    pub resolved: bool,
}
```

---

## 3. SQLite Schema & Migrations

**Hai tầng DB (F-17):** migrations riêng cho `server.db` (bảng `api_tokens`, `workspaces`, `audit_log` — schema tại Plan 06 §4.1) và cho DB per-workspace (dưới đây). Các bảng Quality Engine (`agent_messages`, `reviews`, `critiques`, `verification_records`, `quality_policies`, `quality_scores`) theo Plan 07 §4–5.

**File cần tạo:** `crates/co-force-core/src/db/migrations/001_initial.sql` (per-workspace)

```sql
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;

CREATE TABLE IF NOT EXISTS agents (
    agent_id     TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    name         TEXT NOT NULL,
    role         TEXT DEFAULT 'developer',
    provider     TEXT,
    machine_id   TEXT NOT NULL,
    state        TEXT DEFAULT 'idle',
    current_task_id TEXT,
    last_seen    TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS tasks (
    task_id      TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    title        TEXT NOT NULL,
    objective    TEXT,
    status       TEXT DEFAULT 'draft',   -- giá trị theo TaskStatus (Plan 07 §3)
    revision     INTEGER DEFAULT 1,      -- F-21: tăng theo sự kiện server quan sát được
    rework_cycle INTEGER DEFAULT 0,      -- Plan 07 §3: quá max → escalate
    assigned_agent_id       TEXT,
    delegated_from_agent_id TEXT,
    parent_task_id          TEXT,
    use_cases    TEXT,   -- JSON
    prerequisites TEXT,  -- JSON
    verification_plan TEXT, -- JSON
    required_skills   TEXT, -- JSON
    locked_files      TEXT, -- JSON
    impact_analysis   TEXT, -- JSON
    priority     INTEGER DEFAULT 0,
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP,
    FOREIGN KEY (assigned_agent_id) REFERENCES agents(agent_id),
    FOREIGN KEY (parent_task_id)    REFERENCES tasks(task_id)
);

CREATE TABLE IF NOT EXISTS file_locks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id TEXT NOT NULL,
    file_path    TEXT NOT NULL,
    agent_id     TEXT NOT NULL,
    machine_id   TEXT NOT NULL,
    task_id      TEXT,
    reason       TEXT,
    locked_at    TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    expires_at   TIMESTAMP,
    UNIQUE(workspace_id, file_path),
    FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
);

CREATE TABLE IF NOT EXISTS memory_entries (
    entry_id     TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    entry_type   TEXT NOT NULL,  -- 'memory' | 'knowledge' | 'skill'
    content      TEXT NOT NULL,
    source       TEXT,
    agent_id     TEXT,
    confidence   REAL DEFAULT 1.0,
    tags         TEXT,           -- JSON array
    embedding    BLOB,           -- F-02: vector nằm ngay trong DB; NULL = đang chờ re-embed
                                 -- (recall báo PARTIAL_INDEX — F-19, không có vector_id/index file riêng)
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    accessed_at  TIMESTAMP,
    access_count INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS skills (
    skill_id     TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    name         TEXT NOT NULL,
    description  TEXT,
    category     TEXT,
    steps        TEXT,           -- JSON array
    source_memories TEXT,        -- JSON array of entry_ids
    usage_count  INTEGER DEFAULT 0,
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS embedding_cache (
    content_hash TEXT PRIMARY KEY, -- SHA-256 của content
    embedding    BLOB NOT NULL,
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Bổ sung từ URD Group H:
CREATE TABLE IF NOT EXISTS agent_activities (
    activity_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    content TEXT, -- JSON
    related_task_id TEXT,
    related_files TEXT, -- JSON
    version INTEGER DEFAULT 1,
    occurred_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
);

CREATE TABLE IF NOT EXISTS shared_contexts (
    context_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    source_agent_id TEXT NOT NULL,
    target_agent_id TEXT,
    context_type TEXT NOT NULL,
    content TEXT NOT NULL,
    resolved BOOLEAN DEFAULT FALSE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    resolved_at TIMESTAMP,
    FOREIGN KEY (source_agent_id) REFERENCES agents(agent_id)
);

-- Hot-path indexes (Master Plan WS-A)
CREATE INDEX IF NOT EXISTS idx_activities_ws_time ON agent_activities(workspace_id, occurred_at);
CREATE INDEX IF NOT EXISTS idx_memory_ws_type ON memory_entries(workspace_id, entry_type);
```

**File cần tạo:** `crates/co-force-core/src/db/mod.rs`
Hàm `run_migrations_async` dùng `tokio_rusqlite` để load và thực thi script SQL trên.

---

## 4. Giao diện Repository (Ports Layer)
**File cần tạo:** `crates/co-force-core/src/engine/ports.rs`

Định nghĩa các Trait và dùng `#[async_trait]` cùng `#[mockall::automock]` để hỗ trợ TDD.

```rust
use async_trait::async_trait;
use crate::types::*;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait AgentRepository: Send + Sync {
    async fn find_by_id(&self, id: &AgentId) -> anyhow::Result<Option<Agent>>;
    async fn upsert(&self, agent: &Agent) -> anyhow::Result<()>;
    async fn list_active(&self, workspace_id: &WorkspaceId) -> anyhow::Result<Vec<Agent>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ActivityRepository: Send + Sync {
    async fn log_activity(&self, activity: &AgentActivity) -> anyhow::Result<()>;
    async fn get_workspace_stream(&self, ws: &WorkspaceId, limit: usize) -> anyhow::Result<Vec<AgentActivity>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ContextRepository: Send + Sync {
    async fn share_context(&self, ctx: &SharedContext) -> anyhow::Result<()>;
    async fn get_unresolved(&self, target_agent: &AgentId) -> anyhow::Result<Vec<SharedContext>>;
    async fn mark_resolved(&self, context_id: &ContextId) -> anyhow::Result<()>;
}
```

---

## 5. Trình tự Triển khai (Step-by-Step)
1. Cấu hình Cargo.toml cho `co-force-core`, thêm dependencies: `serde`, `tokio`, `rusqlite`, `tokio-rusqlite`, `mockall`, `async-trait`, `chrono`.
2. Tạo file `types/mod.rs` và gõ toàn bộ Strong Types & Structs.
3. Tạo file `db/migrations/001_initial.sql` và chép schema.
4. Tạo file `engine/ports.rs` và định nghĩa Traits.
5. Tạo folder `db/` và implement các Trait bằng concrete struct (vd: `SqliteAgentRepo`).
6. Chạy `cargo check` để đảm bảo không lỗi kiểu dữ liệu.
