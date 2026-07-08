# Detailed Implementation Plan: 02 - MCP Server and Use Cases Layer

**Status:** Ready for Implementation
**Target:** `crates/co-force-core/src/engine/` and `crates/co-force-mcp/`

> **⚠️ Update 2026-07-08 (see `docs/review_findings.md` F-01):** `rmcp` is now **2.x stable** — not 0.16 as in the URD. Two consequences for this plan:
> 1. The `#[rmcp::server]` macro **does not exist**. The correct API for rmcp 2.x is: `#[tool_router]` on `impl`, `#[tool(description = ...)]` on each method, parameters are structs deriving `serde::Deserialize + schemars::JsonSchema` wrapped in `Parameters<T>`, and implementing the `ServerHandler` trait (using `#[tool_handler]`). Refer to examples in the `modelcontextprotocol/rust-sdk` repo.
> 2. **SSE transport has been deprecated from the MCP spec** — replaced by **Streamable HTTP** (features `transport-streamable-http-server` + `transport-streamable-http-server-session` to bind sessions, serving Implicit Session Binding via the `Mcp-Session-Id` header). The sample code blocks below are for structural illustration only; the actual code must follow the real API.

## 1. Context & Objectives
This layer is responsible for processing business logic via **Use Case Classes/Structs** (Clean Architecture) and exposing those tools to client AI Agents via the Model Context Protocol (MCP) protocol.

*Reference Documents:*
- `architecture.md` §6 (response envelope, error codes, 39-tool catalog)
- `URD.md` (Appendix B: MCP Tool Signatures)

---

## 2. Use Case Engine Design
**Location:** `crates/co-force-core/src/engine/`

All logic must reside in the core, not in the MCP Server files. Each Use Case is a struct that receives `Arc<dyn Trait>` via the `new` function.

### 2.1 Pattern Example: CheckInUseCase
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
        // 1. Analyze agent_id. If present, find in DB, otherwise create new.
        // 2. Upsert Agent status -> Idle/Working
        // 3. Log Activity: `ActivityType::CheckIn`
        // 4. Retrieve pending tasks and return them
        todo!()
    }
}
```

### 2.2 Other Use Cases to Implement
- `LockFilesUseCase` (Requires `LockRepository`)
- `UpdateTaskUseCase` (Requires `TaskRepository`, records Activity after update)
- `GetAgentContextUseCase` (Retrieves data from Activity Repo & Context Repo)
- `ShareContextUseCase` (Saves into Context Repo)

---

## 3. MCP Server Design
**Location:** `crates/co-force-mcp/src/main.rs`

Use rmcp **2.x** (`#[tool_router]` + `#[tool]` + `ServerHandler` — see banner at top of file). The sample code below remains in the old structural illustration form; the actual code must follow the real API.

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
Attach detailed descriptions in `description` as this is the prompt to encourage the Agent to call the tool.

```rust
#[tool_router] // Real API rmcp 2.x — includes #[tool_handler] impl ServerHandler; params wrapped in Parameters<T> (derive JsonSchema)
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
        // Call GetAgentContextUseCase and parse to JSON
        todo!()
    }
}
```

---

## 4. Transport Configuration
The MCP Server needs to run in one of two modes (receiving parameters via `clap` CLI arguments):
1. **Stdio Transport** (`transport-io`): Communicates via stdin/stdout — for single-agent or clients that do not speak HTTP.
2. **Streamable HTTP Transport** (`transport-streamable-http-server`): HTTP server at `127.0.0.1:3846/mcp` — **default mode** (multiple agents share 1 server, session binding via `Mcp-Session-Id`). This is the replacement transport for the deprecated SSE.

```rust
// In main.rs (illustration — according to rmcp 2.x API: serve_server + StreamableHttpService)
match args.transport {
    Transport::Stdio => {
        let service = CoForceMcp::new(/* use cases */);
        rmcp::serve_server(service, rmcp::transport::io::stdio()).await?;
    }
    Transport::Http { addr } => {
        // StreamableHttpService mounted into axum Router at /mcp,
        // sharing the listener serving /dashboard (decision F-13)
    }
}
```

---

## 5. Steps to Implement (Step-by-Step)
1. In `co-force-core`, create the `engine/` folder and write Unit Tests (using `mockall`) for each Use Case.
2. Implement actual logic for the Use Cases until they pass the Unit Tests. Ensure EVERY Use Case has an activity logging mechanism.
3. In `co-force-mcp/Cargo.toml`, add `rmcp` and `tokio` libraries.
4. Write `main.rs`, implementing the `CoForceMcp` struct.
5. Implement trait methods using the `#[tool]` macro. Make sure to copy the exact Tool Signatures from Appendix B of the URD.
6. Add the CLI args parser (`clap`) to select Transport mode (Streamable HTTP / Stdio).
7. Test run locally: `cargo run -p co-force-mcp -- --transport stdio` and manually input JSON-RPC to test.
