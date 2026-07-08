# Developer Implementation Instructions
## Project Co-Force: MCP Server for Multi-Agent Coordination

This guide covers the Rust development workflow, environment setup, TDD approach, and running the Co-Force MCP Server.

> **⚠️ Cập nhật 2026-07-08:** Tài liệu này có một số điểm đã được review sửa lại — nguồn sự thật:
> - **Versions:** theo root `Cargo.toml` (rmcp **2.x**, rusqlite 0.32, tokio-rusqlite 0.6 — không phải rmcp 0.16 như các đoạn bên dưới).
> - **Transport:** SSE đã bị deprecate khỏi MCP spec → dùng **Streamable HTTP** (`/mcp` endpoint). Các lệnh `--transport sse` bên dưới đọc thành `--http`.
> - **Kiến trúc & storage layout:** theo `docs/architecture.md`. **Setup UX:** theo `docs/plans/05_setup_ux_and_onboarding.md` (Ollama là optional — mục 2.2 không còn là prerequisite bắt buộc).
> - **Vector store:** không dùng `embedvec`; embedding lưu BLOB trong SQLite (xem `docs/review_findings.md` F-02).
> - Macro `#[rmcp::server]` trong Section 6 là minh họa cũ; API thật của rmcp 2.x là `#[tool_router]`/`#[tool]`/`ServerHandler`.

---

## 1. Workspace & Crate Structure

```
co-force/                           # Cargo workspace root
├── Cargo.toml                      # Workspace members
├── crates/
│   ├── co-force-core/              # Business logic, domain, DB, LLM
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types/              # Domain types: Agent, Task, Memory, Skill
│   │       ├── db/                 # SQLite repository implementations
│   │       ├── engine/             # Use case handlers (CheckIn, Lock, Recall...)
│   │       ├── llm/                # Ollama + cloud provider adapters
│   │       └── workspace/          # Workspace init, config loading
│   ├── co-force-mcp/               # MCP server (rmcp-based, SSE transport)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       └── tools/              # MCP tool handlers (co_force_check_in, etc.)
│   └── co-force-tauri/             # Tauri desktop app (future)
│       ├── Cargo.toml
│       └── src/
│           └── main.rs
└── docs/
    ├── URD.md
    └── implementation_instructions.md
```

**Cargo workspace root `Cargo.toml`:**
```toml
[workspace]
members = [
    "crates/co-force-core",
    "crates/co-force-mcp",
]
resolver = "2"

[workspace.dependencies]
tokio      = { version = "1", features = ["full"] }
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
rusqlite   = { version = "0.31", features = ["bundled"] }
tokio-rusqlite = "0.5"
reqwest    = { version = "0.12", features = ["json"] }
uuid       = { version = "1", features = ["v4"] }
toml       = "0.8"
tracing    = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
rmcp       = { version = "0.16", features = ["server", "transport-sse"] }
mockall    = "0.13"
```

---

## 2. Environment Setup

### 2.1 Prerequisites

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update stable

# Ollama (local LLM server)
# macOS
brew install ollama

# Linux
curl -fsSL https://ollama.com/install.sh | sh
```

### 2.2 Pull Required Models

```bash
# Start Ollama daemon
ollama serve

# Embedding model (1024d vectors)
ollama pull mxbai-embed-large

# Classifier LLM (2B params, fast)
ollama pull gemma4:e2b
```

### 2.3 First-Time Configuration

On first run, Co-Force will prompt an interactive setup wizard to create `~/.co-force/config.toml`. To configure manually:

```toml
# ~/.co-force/config.toml

[llm]
default_classifier_provider = "ollama"
default_embedding_provider = "ollama"

[ollama]
url = "http://localhost:11434"
embedding_model = "mxbai-embed-large"
classifier_model = "gemma4:e2b"
timeout_secs = 30
retry_count = 3
concurrency_limit = 2

[rag]
chunk_min_tokens = 128
chunk_max_tokens = 256
parent_chunk_max_tokens = 1024
similarity_threshold = 0.3
classification_confidence_threshold = 0.7

[dashboard]
enabled = true
port = 3847
websocket_port = 3848
```

---

## 3. Test-Driven Development (TDD) Workflow

Co-Force uses Rust's built-in test framework with `mockall` for mocking traits.

### 3.1 Domain Types (co-force-core/src/types/)

Define strong types before writing logic:

```rust
// crates/co-force-core/src/types/mod.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Working,
    Paused,
    Disconnected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Draft,
    PendingReview,
    Approved,
    InProgress,
    Blocked,
    Completed,
    Failed,
}

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
```

### 3.2 Repository Traits (Ports Layer)

Define traits that use cases depend on:

```rust
// crates/co-force-core/src/engine/ports.rs
use crate::types::{Agent, AgentId, WorkspaceId, Task, TaskId, FileLock};
use async_trait::async_trait;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait AgentRepository: Send + Sync {
    async fn find_by_id(&self, id: &AgentId) -> anyhow::Result<Option<Agent>>;
    async fn upsert(&self, agent: &Agent) -> anyhow::Result<()>;
    async fn list_active(&self, workspace_id: &WorkspaceId) -> anyhow::Result<Vec<Agent>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait LockRepository: Send + Sync {
    async fn acquire(&self, lock: &FileLock) -> anyhow::Result<bool>;
    async fn release(&self, file_path: &str, agent_id: &AgentId) -> anyhow::Result<()>;
    async fn find_conflict(&self, file_path: &str) -> anyhow::Result<Option<FileLock>>;
    async fn release_all_for_agent(&self, agent_id: &AgentId) -> anyhow::Result<Vec<String>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
    async fn classify(&self, content: &str) -> anyhow::Result<(String, f32)>;
}
```

### 3.3 Unit Testing a Use Case

Example: TDD for the `CheckIn` use case:

```rust
// crates/co-force-core/src/engine/check_in.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ports::{MockAgentRepository};
    use crate::types::{AgentId, WorkspaceId};
    use mockall::predicate::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_new_agent_gets_registered() {
        let mut mock_repo = MockAgentRepository::new();

        // Agent does not exist yet
        mock_repo
            .expect_find_by_id()
            .returning(|_| Ok(None));

        // Expect an upsert with state = Idle
        mock_repo
            .expect_upsert()
            .withf(|agent| agent.state == AgentState::Idle)
            .returning(|_| Ok(()));

        let use_case = CheckInUseCase::new(Arc::new(mock_repo));

        let result = use_case.execute(CheckInRequest {
            workspace_path: "/projects/my-app".into(),
            agent_name: "Claude-CLI".into(),
            role: "developer".into(),
            agent_id: None,
            machine_id: "machine-abc".into(),
        }).await.unwrap();

        assert!(result.onboarding_required);
        assert!(!result.agent_id.0.is_empty());
    }

    #[tokio::test]
    async fn test_returning_agent_restores_session() {
        let existing_id = AgentId("existing-uuid".into());
        let mut mock_repo = MockAgentRepository::new();

        let agent = Agent {
            agent_id: existing_id.clone(),
            state: AgentState::Disconnected,
            ..Default::default()
        };
        mock_repo
            .expect_find_by_id()
            .returning(move |_| Ok(Some(agent.clone())));

        mock_repo
            .expect_upsert()
            .withf(|a| a.state == AgentState::Idle)
            .returning(|_| Ok(()));

        let use_case = CheckInUseCase::new(Arc::new(mock_repo));
        let result = use_case.execute(CheckInRequest {
            agent_id: Some(existing_id),
            ..Default::default()
        }).await.unwrap();

        assert!(!result.onboarding_required);
    }
}
```

### 3.4 Integration Tests

Integration tests run against a real in-memory SQLite database:

```rust
// crates/co-force-core/tests/integration_lock_flow.rs
use co_force_core::{
    db::SqliteAgentRepo, db::SqliteLockRepo,
    engine::{CheckInUseCase, LockFilesUseCase},
};
use std::sync::Arc;

async fn setup_test_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    co_force_core::db::run_migrations(&conn).unwrap();
    conn
}

#[tokio::test]
async fn test_full_check_in_then_lock_flow() {
    let conn = Arc::new(tokio_rusqlite::Connection::open_in_memory().await.unwrap());
    co_force_core::db::run_migrations_async(&conn).await.unwrap();

    let agent_repo = Arc::new(SqliteAgentRepo::new(conn.clone()));
    let lock_repo  = Arc::new(SqliteLockRepo::new(conn.clone()));

    // Step 1: Check in
    let check_in = CheckInUseCase::new(agent_repo.clone());
    let session = check_in.execute(CheckInRequest {
        workspace_path: "/tmp/test-ws".into(),
        agent_name: "test-agent".into(),
        role: "developer".into(),
        agent_id: None,
        machine_id: "test-machine".into(),
    }).await.unwrap();

    // Step 2: Lock file
    let lock_uc = LockFilesUseCase::new(lock_repo.clone(), agent_repo.clone());
    let lock_result = lock_uc.execute(LockFilesRequest {
        agent_id: session.agent_id.clone(),
        files: vec!["src/main.rs".into()],
        task_id: None,
    }).await.unwrap();

    assert_eq!(lock_result.locked, vec!["src/main.rs"]);
    assert!(lock_result.conflicts.is_empty());

    // Step 3: Conflict detected for second agent
    let second_session = check_in.execute(CheckInRequest {
        workspace_path: "/tmp/test-ws".into(),
        agent_name: "agent-2".into(),
        role: "developer".into(),
        agent_id: None,
        machine_id: "test-machine".into(),
    }).await.unwrap();

    let conflict_result = lock_uc.execute(LockFilesRequest {
        agent_id: second_session.agent_id,
        files: vec!["src/main.rs".into()],
        task_id: None,
    }).await.unwrap();

    assert!(conflict_result.locked.is_empty());
    assert_eq!(conflict_result.conflicts.len(), 1);
}
```

---

## 4. Running the MCP Server

### 4.1 Development Mode (stdio transport — for local agent testing)

```bash
# Build and run with stdio transport (Claude CLI compatible)
cargo run -p co-force-mcp -- --transport stdio

# Or with SSE transport on port 3846
cargo run -p co-force-mcp -- --transport sse --port 3846
```

### 4.2 Configure Claude CLI / Cursor to use Co-Force MCP

**Claude Code — project scope (`<project>/.mcp.json`, do `co-force init` tự sinh):**
```json
{
  "mcpServers": {
    "co-force": {
      "type": "http",
      "url": "http://127.0.0.1:3846/mcp"
    }
  }
}
```
(Hoặc stdio: `claude mcp add co-force -- co-force serve --stdio`. Lưu ý: `claude_desktop_config.json` là của Claude **Desktop**, không phải Claude Code.)

**Cursor (`.cursor/mcp.json`) / Windsurf / other IDEs (Streamable HTTP):**
```json
{
  "mcpServers": {
    "co-force": {
      "url": "http://127.0.0.1:3846/mcp"
    }
  }
}
```

### 4.3 Running Tests

```bash
# All unit tests
cargo test --workspace

# Integration tests only
cargo test --test '*' --workspace

# With logging output
RUST_LOG=debug cargo test --workspace -- --nocapture

# Watch mode during development
cargo watch -x "test --workspace"
```

### 4.4 Linting & Code Quality

```bash
# Clippy (strict — must pass before commit)
cargo clippy --workspace -- -D warnings -D clippy::pedantic

# Format check
cargo fmt --check

# Security audit
cargo audit
```

---

## 5. SQLite Schema & Migrations

Migrations live in `crates/co-force-core/src/db/migrations/` and are embedded via `include_str!` at compile time:

```rust
// crates/co-force-core/src/db/mod.rs
pub fn run_migrations(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(include_str!("migrations/001_initial.sql"))?;
    Ok(())
}
```

```sql
-- crates/co-force-core/src/db/migrations/001_initial.sql
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
    status       TEXT DEFAULT 'draft',
    assigned_agent_id     TEXT,
    delegated_from_agent_id TEXT,
    parent_task_id        TEXT,
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
    vector_id    TEXT,
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
    content_hash TEXT PRIMARY KEY,
    embedding    TEXT NOT NULL,  -- JSON float array
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

---

## 6. MCP Tool Registration (co-force-mcp)

Each tool is registered using `rmcp`'s proc-macro:

```rust
// crates/co-force-mcp/src/main.rs
use rmcp::{ServerHandler, tool};
use co_force_core::engine::{CheckInUseCase, LockFilesUseCase, RecallUseCase};
use std::sync::Arc;

pub struct CoForceMcp {
    check_in: Arc<CheckInUseCase>,
    lock_files: Arc<LockFilesUseCase>,
    recall: Arc<RecallUseCase>,
    // ... other use cases
}

#[rmcp::server]
impl CoForceMcp {
    #[tool(description = "MANDATORY: Call this first before any workspace action. \
        Registers agent identity, syncs pending tasks and active agents. \
        Skipping this will block all other tools with CHECK_IN_REQUIRED error.")]
    async fn co_force_check_in(
        &self,
        workspace_path: String,
        agent_name: String,
        role: String,
        agent_id: Option<String>,
    ) -> serde_json::Value {
        // delegate to self.check_in use case
        todo!()
    }

    #[tool(description = "MANDATORY: Acquire exclusive write lock on files before editing. \
        Never edit a file without a lock. Returns conflicts if files are locked by other agents.")]
    async fn co_force_lock_files(
        &self,
        files: Vec<String>,
        task_id: Option<String>,
    ) -> serde_json::Value {
        todo!()
    }

    #[tool(description = "Semantic search across stored memory, knowledge, and skills.")]
    async fn co_force_recall(
        &self,
        query: String,
        types: Option<Vec<String>>,
        limit: Option<usize>,
    ) -> serde_json::Value {
        todo!()
    }
}
```

---

## 7. Implementing Ollama Integration

```rust
// crates/co-force-core/src/llm/ollama.rs
use crate::engine::ports::LlmProvider;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

pub struct OllamaProvider {
    client: Client,
    base_url: String,
    embedding_model: String,
    classifier_model: String,
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        #[derive(Deserialize)]
        struct EmbedResponse { embedding: Vec<f32> }

        let resp: EmbedResponse = self.client
            .post(format!("{}/api/embeddings", self.base_url))
            .json(&serde_json::json!({
                "model": self.embedding_model,
                "prompt": text
            }))
            .send().await?
            .json().await?;

        Ok(resp.embedding)
    }

    async fn classify(&self, content: &str) -> anyhow::Result<(String, f32)> {
        let prompt = format!(
            "Classify into exactly one: MEMORY, KNOWLEDGE, or SKILL.\n\
             MEMORY: session-specific fact (e.g. 'file X has bug on line 42')\n\
             KNOWLEDGE: general principle (e.g. 'always use parameterized queries')\n\
             SKILL: step-by-step reusable procedure\n\n\
             Text: \"{content}\"\n\nRespond with ONLY the category name.",
        );

        // call Ollama /api/generate, parse response
        // fallback: keyword heuristic if Ollama unreachable
        todo!()
    }
}
```

---

## 8. Deployment Modes

### Solo Developer (localhost)

```
User's machine:
  ├── Ollama (localhost:11434)
  ├── co-force-mcp (stdio or localhost:3846)
  └── AI Agent (Claude CLI / Cursor) → connects via MCP
```

No extra setup needed. All data is stored in `~/.co-force/` and `<project>/.co-force/`.

### Team (LAN / dedicated server)

```
Server machine:
  ├── Ollama (port 11434)
  └── co-force-mcp (port 3846, SSE transport)

Developer machines (each):
  └── AI Agent → connects to http://<server-ip>:3846/sse
```

The server stores all shared state (SQLite DB, vector index). Developer machines only need a local `.co-force/agent.json` to cache their agent ID.

### Starting the server on a dedicated host

```bash
# On the server
RUST_LOG=info cargo run -p co-force-mcp -- \
  --transport sse \
  --port 3846 \
  --host 0.0.0.0

# Or with a pre-built binary
./co-force-mcp --transport sse --port 3846 --host 0.0.0.0
```
