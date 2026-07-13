use anyhow::{Context, Result};
use clap::Parser;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpService,
};
use rmcp::{serve_server, tool, tool_handler, tool_router, ServerHandler};

use co_force_core::db::activity_repo::SqliteActivityRepo;
use co_force_core::db::agent_repo::SqliteAgentRepo;
use co_force_core::db::context_repo::SqliteContextRepo;
use co_force_core::db::lock_repo::SqliteLockRepo;
use co_force_core::db::task_repo::SqliteTaskRepo;
use co_force_core::db::handover_repo::SqliteHandoverRepo;
use co_force_core::orchestration::bus::WorkspaceEventBus;
use co_force_core::orchestration::doc_generator::run_doc_generator;
use co_force_core::db::Database;
use co_force_core::engine::*;

// ===== CLI Arguments =====
#[derive(Parser, Debug)]
#[command(author, version, about = "Co-Force MCP Server")]
struct Args {
    #[arg(long, default_value = "stdio")]
    transport: String, // "stdio" or "http"

    #[arg(long, default_value = "./co-force.db")]
    db: String,

    #[arg(long, default_value = "127.0.0.1:3846")]
    addr: String,
}

// ===== Response Envelope Structures =====
#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseEnvelope<T> {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDetails>,
    pub inbox: InboxState,
    pub protocol_next_step: Option<String>,
    pub workspace_pulse: WorkspacePulse,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorDetails {
    pub code: String,
    pub message: String,
    pub recovery_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct InboxState {
    pub unread: usize,
    pub urgent: Vec<UrgentMessage>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UrgentMessage {
    pub message_id: String,
    pub kind: String,
    pub from: String,
    pub summary: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct WorkspacePulse {
    pub agents_online: usize,
    pub tasks_at_gates: usize,
    pub server_health: String,
}

// ===== CoForceMcp Server Definition =====
#[derive(Clone)]
pub struct CoForceMcp {
    agent_id: Arc<Mutex<Option<String>>>,
    workspace_id: Arc<Mutex<Option<String>>>,
    machine_id: Arc<Mutex<Option<String>>>,

    check_in_usecase: Arc<CheckInUseCase>,
    lock_files_usecase: Arc<LockFilesUseCase>,
    get_agent_context_usecase: Arc<GetAgentContextUseCase>,
    share_context_usecase: Arc<ShareContextUseCase>,
    handover_usecase: Arc<HandoverUseCase>,
    spawn_usecase: Arc<SpawnUseCase>,

    db_conn: tokio_rusqlite::Connection,
}

impl CoForceMcp {
    pub fn new(
        check_in_usecase: Arc<CheckInUseCase>,
        lock_files_usecase: Arc<LockFilesUseCase>,
        get_agent_context_usecase: Arc<GetAgentContextUseCase>,
        share_context_usecase: Arc<ShareContextUseCase>,
        handover_usecase: Arc<HandoverUseCase>,
        spawn_usecase: Arc<SpawnUseCase>,
        db_conn: tokio_rusqlite::Connection,
    ) -> Self {
        Self {
            agent_id: Arc::new(Mutex::new(None)),
            workspace_id: Arc::new(Mutex::new(None)),
            machine_id: Arc::new(Mutex::new(None)),
            check_in_usecase,
            lock_files_usecase,
            get_agent_context_usecase,
            share_context_usecase,
            handover_usecase,
            spawn_usecase,
            db_conn,
        }
    }
}

// ===== Tool Parameter Schemas =====
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckInParams {
    pub workspace_path: String,
    pub agent_name: String,
    pub role: String,
    pub agent_id: Option<String>,
    pub provider: Option<String>,
    pub machine_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LockFilesParams {
    pub file_paths: Vec<String>,
    pub task_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetAgentContextParams {
    pub agent_id: String,
    pub include_history: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ShareContextParams {
    pub target_agent_id: Option<String>,
    pub context_type: String,
    pub content: serde_json::Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpawnParams {
    pub provider: String,
    pub task_id: String,
    pub placement: String,
    pub workspace_path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HandoverParams {
    pub task_id: String,
    pub reason: String,
    pub target_provider: String,
    pub package: serde_json::Value,
    pub provider_cooldown_until: Option<String>,
}

// ===== Helpers for Envelopes =====
async fn fetch_inbox_state(conn: &tokio_rusqlite::Connection, agent_id: &str) -> InboxState {
    let agent_id = agent_id.to_string();
    let result = conn
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT context_id, source_agent_id, context_type, content \
             FROM shared_contexts \
             WHERE (target_agent_id = ?1 OR target_agent_id IS NULL) AND resolved = 0",
            )?;

            let rows = stmt
                .query_map([agent_id], |row| {
                    let context_id: String = row.get(0)?;
                    let source_id: String = row.get(1)?;
                    let context_type: String = row.get(2)?;
                    let content_str: String = row.get(3)?;
                    let content: serde_json::Value =
                        serde_json::from_str(&content_str).unwrap_or_default();

                    let summary = content
                        .get("notes")
                        .or_else(|| content.get("summary"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Shared context")
                        .to_string();

                    Ok(UrgentMessage {
                        message_id: context_id,
                        kind: context_type,
                        from: source_id,
                        summary,
                    })
                })?
                .collect::<Result<Vec<_>, rusqlite::Error>>()?;

            Ok(rows)
        })
        .await
        .unwrap_or_default();

    InboxState {
        unread: result.len(),
        urgent: result,
    }
}

async fn fetch_pulse(conn: &tokio_rusqlite::Connection, workspace_id: &str) -> WorkspacePulse {
    let ws_id1 = workspace_id.to_string();
    let ws_id2 = workspace_id.to_string();

    let online_count: usize = conn
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT count(*) FROM agents WHERE workspace_id = ?1 AND state != 'disconnected'",
            )?;
            let count: usize = stmt.query_row([ws_id1], |row| row.get(0))?;
            Ok(count)
        })
        .await
        .unwrap_or(0);

    let gates_count: usize = conn
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT count(*) FROM tasks WHERE workspace_id = ?1 \
             AND status IN ('spec_review', 'awaiting_approval', 'verification', 'code_review')",
            )?;
            let count: usize = stmt.query_row([ws_id2], |row| row.get(0))?;
            Ok(count)
        })
        .await
        .unwrap_or(0);

    WorkspacePulse {
        agents_online: online_count,
        tasks_at_gates: gates_count,
        server_health: "healthy".to_string(),
    }
}

async fn make_envelope_response<T: Serialize>(
    conn: &tokio_rusqlite::Connection,
    agent_id: Option<&str>,
    workspace_id: Option<&str>,
    data: Option<T>,
) -> CallToolResult {
    let inbox = if let Some(aid) = agent_id {
        fetch_inbox_state(conn, aid).await
    } else {
        InboxState::default()
    };

    let workspace_pulse = if let Some(wid) = workspace_id {
        fetch_pulse(conn, wid).await
    } else {
        WorkspacePulse::default()
    };

    let protocol_next_step = if inbox.unread > 0 {
        Some(format!(
            "You have {} unread context messages. Handle them via co_force_get_agent_context.",
            inbox.unread
        ))
    } else {
        None
    };

    let envelope = ResponseEnvelope {
        status: "success".to_string(),
        data,
        error: None,
        inbox,
        protocol_next_step,
        workspace_pulse,
    };

    let json_str = serde_json::to_string(&envelope).unwrap_or_default();
    CallToolResult::success(vec![ContentBlock::text(json_str)])
}

fn make_error_response(code: &str, message: &str, recovery_action: &str) -> CallToolResult {
    let envelope = ResponseEnvelope::<()>::error(code, message, recovery_action);
    let json_str = serde_json::to_string(&envelope).unwrap_or_default();
    CallToolResult::error(vec![ContentBlock::text(json_str)])
}

impl<T> ResponseEnvelope<T> {
    fn error(code: &str, message: &str, recovery_action: &str) -> Self {
        Self {
            status: "error".to_string(),
            data: None,
            error: Some(ErrorDetails {
                code: code.to_string(),
                message: message.to_string(),
                recovery_action: Some(recovery_action.to_string()),
                retry_after_secs: None,
            }),
            inbox: InboxState::default(),
            protocol_next_step: Some(format!("Error: {}. Action: {}", message, recovery_action)),
            workspace_pulse: WorkspacePulse::default(),
        }
    }
}

// ===== Tool Implementation =====
#[tool_router]
impl CoForceMcp {
    #[tool(
        description = "MANDATORY: Call this first before any workspace action to register your session."
    )]
    async fn co_force_check_in(&self, params: Parameters<CheckInParams>) -> CallToolResult {
        let args = params.0;
        let req = CheckInRequest {
            workspace_path: args.workspace_path,
            agent_name: args.agent_name,
            role: args.role,
            agent_id: args.agent_id,
            provider: args.provider,
            machine_id: args.machine_id,
        };

        match self.check_in_usecase.execute(req).await {
            Ok(res) => {
                // Store session info
                *self.agent_id.lock().await = Some(res.agent_id.clone());
                *self.workspace_id.lock().await = Some(res.workspace_id.clone());

                let aid = res.agent_id.clone();
                let wid = res.workspace_id.clone();
                make_envelope_response(&self.db_conn, Some(&aid), Some(&wid), Some(res)).await
            }
            Err(e) => make_error_response(
                "INTERNAL_ERROR",
                &format!("Check-in failed: {e}"),
                "Retry the check-in call",
            ),
        }
    }

    #[tool(
        description = "MANDATORY: MUST be called before modifying any files. Requests exclusive locks."
    )]
    async fn co_force_lock_files(&self, params: Parameters<LockFilesParams>) -> CallToolResult {
        let agent_id_opt = self.agent_id.lock().await.clone();
        let workspace_id_opt = self.workspace_id.lock().await.clone();
        let machine_id_opt = self
            .machine_id
            .lock()
            .await
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let (agent_id, workspace_id) = match (agent_id_opt, workspace_id_opt) {
            (Some(aid), Some(wid)) => (aid, wid),
            _ => {
                return make_error_response(
                    "CHECK_IN_REQUIRED",
                    "Protocol Violation: You must check-in first.",
                    "co_force_check_in(workspacePath, agentName, role)",
                );
            }
        };

        let args = params.0;
        let req = LockFilesRequest {
            workspace_id: workspace_id.clone(),
            agent_id: agent_id.clone(),
            file_paths: args.file_paths,
            machine_id: machine_id_opt,
            task_id: args.task_id,
            reason: args.reason,
        };

        match self.lock_files_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                )
                .await
            }
            Err(e) => make_error_response(
                "LOCK_CONFLICT",
                &format!("Failed to lock files: {e}"),
                "co_force_check_conflicts or coordinate with other agents",
            ),
        }
    }

    #[tool(description = "Get recent activity and context of another agent.")]
    async fn co_force_get_agent_context(
        &self,
        params: Parameters<GetAgentContextParams>,
    ) -> CallToolResult {
        let agent_id_opt = self.agent_id.lock().await.clone();
        let workspace_id_opt = self.workspace_id.lock().await.clone();

        let (agent_id, workspace_id) = match (agent_id_opt, workspace_id_opt) {
            (Some(aid), Some(wid)) => (aid, wid),
            _ => {
                return make_error_response(
                    "CHECK_IN_REQUIRED",
                    "Protocol Violation: You must check-in first.",
                    "co_force_check_in(workspacePath, agentName, role)",
                );
            }
        };

        let args = params.0;
        let req = GetAgentContextRequest {
            agent_id: args.agent_id,
            include_history: args.include_history,
        };

        match self.get_agent_context_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                )
                .await
            }
            Err(e) => make_error_response(
                "INTERNAL_ERROR",
                &format!("Failed to get context: {e}"),
                "Check agent ID or retry",
            ),
        }
    }

    #[tool(description = "Shares specific context blocks (lazy resolution).")]
    async fn co_force_share_context(
        &self,
        params: Parameters<ShareContextParams>,
    ) -> CallToolResult {
        let agent_id_opt = self.agent_id.lock().await.clone();
        let workspace_id_opt = self.workspace_id.lock().await.clone();

        let (agent_id, workspace_id) = match (agent_id_opt, workspace_id_opt) {
            (Some(aid), Some(wid)) => (aid, wid),
            _ => {
                return make_error_response(
                    "CHECK_IN_REQUIRED",
                    "Protocol Violation: You must check-in first.",
                    "co_force_check_in(workspacePath, agentName, role)",
                );
            }
        };

        let args = params.0;
        let req = ShareContextRequest {
            workspace_id: workspace_id.clone(),
            source_agent_id: agent_id.clone(),
            target_agent_id: args.target_agent_id,
            context_type: args.context_type,
            content: args.content,
        };

        match self.share_context_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                )
                .await
            }
            Err(e) => make_error_response(
                "INTERNAL_ERROR",
                &format!("Failed to share context: {e}"),
                "Retry context share",
            ),
        }
    }

    #[tool(description = "Spawns a new subagent (Lane 2 directive or Lane 3 local process).")]
    async fn co_force_spawn_agent(
        &self,
        params: Parameters<SpawnParams>,
    ) -> CallToolResult {
        let agent_id_opt = self.agent_id.lock().await.clone();
        let workspace_id_opt = self.workspace_id.lock().await.clone();

        let (agent_id, workspace_id) = match (agent_id_opt, workspace_id_opt) {
            (Some(aid), Some(wid)) => (aid, wid),
            _ => {
                return make_error_response(
                    "CHECK_IN_REQUIRED",
                    "Protocol Violation: You must check-in first.",
                    "co_force_check_in(workspacePath, agentName, role)",
                );
            }
        };

        let args = params.0;
        let req = SpawnRequest {
            provider: args.provider,
            task_id: args.task_id,
            placement: args.placement,
            workspace_path: args.workspace_path,
        };

        match self.spawn_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                )
                .await
            }
            Err(e) => make_error_response(
                "SPAWN_FAILED",
                &format!("Failed to spawn agent: {e}"),
                "Verify provider registry or task prerequisites",
            ),
        }
    }

    #[tool(description = "MANDATORY: MUST be called early upon rate limiting or context exhaustion to request handover.")]
    async fn co_force_handover(
        &self,
        params: Parameters<HandoverParams>,
    ) -> CallToolResult {
        let agent_id_opt = self.agent_id.lock().await.clone();
        let workspace_id_opt = self.workspace_id.lock().await.clone();

        let (agent_id, workspace_id) = match (agent_id_opt, workspace_id_opt) {
            (Some(aid), Some(wid)) => (aid, wid),
            _ => {
                return make_error_response(
                    "CHECK_IN_REQUIRED",
                    "Protocol Violation: You must check-in first.",
                    "co_force_check_in(workspacePath, agentName, role)",
                );
            }
        };

        let args = params.0;
        let cooldown = args.provider_cooldown_until
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&chrono::Utc)));

        let req = HandoverRequest {
            task_id: args.task_id,
            from_agent_id: agent_id.clone(),
            reason: args.reason,
            target_provider: args.target_provider,
            package: args.package,
            provider_cooldown_until: cooldown,
        };

        match self.handover_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                )
                .await
            }
            Err(e) => make_error_response(
                "HANDOVER_INCOMPLETE",
                &format!("Handover validation failed: {e}"),
                "Ensure package.next_steps and package.progress.remaining are not empty",
            ),
        }
    }
}

#[tool_handler]
impl ServerHandler for CoForceMcp {}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse arguments
    let args = Args::parse();

    // Initialize tracing linter / logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Open and migrate database
    let db = Database::open_and_migrate(&args.db)
        .await
        .context("Failed to open and migrate database")?;
    let db_conn = db.conn().clone();

    // Instantiate SQLite repositories
    let agent_repo = Arc::new(SqliteAgentRepo::new(db.conn().clone()));
    let activity_repo = Arc::new(SqliteActivityRepo::new(db.conn().clone()));
    let task_repo = Arc::new(SqliteTaskRepo::new(db.conn().clone()));
    let lock_repo = Arc::new(SqliteLockRepo::new(db.conn().clone()));
    let context_repo = Arc::new(SqliteContextRepo::new(db.conn().clone()));
    let handover_repo = Arc::new(SqliteHandoverRepo::new(db.conn().clone()));

    // Create WorkspaceEventBus
    let bus = WorkspaceEventBus::new(1024);

    // Run doc generator dynamically monitoring task events
    let bus_clone = bus.clone();
    let agent_repo_clone = agent_repo.clone();
    let task_repo_clone = task_repo.clone();
    tokio::spawn(async move {
        run_doc_generator(
            bus_clone,
            agent_repo_clone,
            task_repo_clone,
            co_force_core::types::WorkspaceId::from("default"),
            Some("./AGENTS.md".to_string()),
        )
        .await;
    });

    // Instantiate Use Cases
    let check_in = Arc::new(CheckInUseCase::new(
        agent_repo.clone(),
        activity_repo.clone(),
        task_repo.clone(),
        bus.clone(),
    ));
    let lock_files = Arc::new(LockFilesUseCase::new(
        lock_repo.clone(),
        activity_repo.clone(),
        bus.clone(),
    ));
    let get_agent_context = Arc::new(GetAgentContextUseCase::new(
        agent_repo.clone(),
        activity_repo.clone(),
        context_repo.clone(),
    ));
    let share_context = Arc::new(ShareContextUseCase::new(
        context_repo.clone(),
        activity_repo.clone(),
    ));
    let handover = Arc::new(HandoverUseCase::new(
        handover_repo.clone(),
        task_repo.clone(),
        activity_repo.clone(),
        bus.clone(),
    ));
    let spawn = Arc::new(SpawnUseCase::new());

    match args.transport.as_str() {
        "stdio" => {
            let stdio_server = CoForceMcp::new(
                check_in,
                lock_files,
                get_agent_context,
                share_context,
                handover,
                spawn,
                db_conn,
            );
            let transport = rmcp::transport::io::stdio();
            let running = serve_server(stdio_server, transport).await?;
            running.waiting().await?;
        }
        "http" => {
            let service = StreamableHttpService::new(
                move || {
                    Ok(CoForceMcp::new(
                        check_in.clone(),
                        lock_files.clone(),
                        get_agent_context.clone(),
                        share_context.clone(),
                        handover.clone(),
                        spawn.clone(),
                        db_conn.clone(),
                    ))
                },
                LocalSessionManager::default().into(),
                Default::default(),
            );

            // Set up Axum router
            let app = axum::Router::new().nest_service("/mcp", service);

            let listener = tokio::net::TcpListener::bind(&args.addr).await?;
            tracing::info!(
                "Server listening on streamable HTTP at http://{}",
                args.addr
            );
            axum::serve(listener, app).await?;
        }
        other => {
            anyhow::bail!("Unsupported transport: {}", other);
        }
    }

    Ok(())
}
