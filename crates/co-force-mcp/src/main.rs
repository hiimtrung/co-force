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
use co_force_core::db::handover_repo::SqliteHandoverRepo;
use co_force_core::db::lock_repo::SqliteLockRepo;
use co_force_core::db::task_repo::SqliteTaskRepo;
use co_force_core::db::Database;
use co_force_core::engine::*;
use co_force_core::llm::{
    ConsolidateMemoryUseCase, CreateSkillUseCase, GetSkillUseCase, ListSkillsUseCase,
    RecallUseCase, StoreMemoryUseCase,
};
use co_force_core::orchestration::bus::WorkspaceEventBus;
use co_force_core::orchestration::doc_generator::run_doc_generator;
use co_force_core::quality::messaging::{SendMessageUseCase, WaitEventsUseCase};
use co_force_core::quality::review::SubmitReviewUseCase;
use co_force_core::types::{AgentId, WorkspaceId};

// ===== CLI Arguments =====
#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Co-Force MCP Server — Quality-Driven Multi-Agent Orchestration"
)]
struct Args {
    #[arg(long, default_value = "stdio")]
    transport: String, // "stdio" or "http"

    #[arg(long, default_value = "./co-force.db")]
    db: String,

    #[arg(long, default_value = "./server.db")]
    server_db: String,

    #[arg(long, default_value = "https://mcp.example.com")]
    public_url: String,

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

    // Task management use cases
    create_tasks_usecase: Arc<CreateTasksUseCase>,
    list_tasks_usecase: Arc<ListTasksUseCase>,
    update_task_usecase: Arc<UpdateTaskUseCase>,
    approve_tasks_usecase: Arc<ApproveTasksUseCase>,
    delegate_task_usecase: Arc<DelegateTaskUseCase>,
    submit_verification_usecase: Arc<SubmitVerificationUseCase>,
    unlock_files_usecase: Arc<UnlockFilesUseCase>,
    check_conflicts_usecase: Arc<CheckConflictsUseCase>,
    list_agents_usecase: Arc<ListAgentsUseCase>,
    get_workspace_activity_usecase: Arc<GetWorkspaceActivityUseCase>,

    // Quality Engine use cases
    send_message_usecase: Arc<SendMessageUseCase>,
    wait_events_usecase: Arc<WaitEventsUseCase>,
    submit_review_usecase: Arc<SubmitReviewUseCase>,

    // RAG & Skills use cases
    store_memory_usecase: Arc<StoreMemoryUseCase>,
    recall_usecase: Arc<RecallUseCase>,
    consolidate_memory_usecase: Arc<ConsolidateMemoryUseCase>,
    create_skill_usecase: Arc<CreateSkillUseCase>,
    list_skills_usecase: Arc<ListSkillsUseCase>,
    get_skill_usecase: Arc<GetSkillUseCase>,

    db_conn: tokio_rusqlite::Connection,
}

impl CoForceMcp {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        check_in_usecase: Arc<CheckInUseCase>,
        lock_files_usecase: Arc<LockFilesUseCase>,
        get_agent_context_usecase: Arc<GetAgentContextUseCase>,
        share_context_usecase: Arc<ShareContextUseCase>,
        handover_usecase: Arc<HandoverUseCase>,
        spawn_usecase: Arc<SpawnUseCase>,
        create_tasks_usecase: Arc<CreateTasksUseCase>,
        list_tasks_usecase: Arc<ListTasksUseCase>,
        update_task_usecase: Arc<UpdateTaskUseCase>,
        approve_tasks_usecase: Arc<ApproveTasksUseCase>,
        delegate_task_usecase: Arc<DelegateTaskUseCase>,
        submit_verification_usecase: Arc<SubmitVerificationUseCase>,
        unlock_files_usecase: Arc<UnlockFilesUseCase>,
        check_conflicts_usecase: Arc<CheckConflictsUseCase>,
        list_agents_usecase: Arc<ListAgentsUseCase>,
        get_workspace_activity_usecase: Arc<GetWorkspaceActivityUseCase>,
        send_message_usecase: Arc<SendMessageUseCase>,
        wait_events_usecase: Arc<WaitEventsUseCase>,
        submit_review_usecase: Arc<SubmitReviewUseCase>,
        store_memory_usecase: Arc<StoreMemoryUseCase>,
        recall_usecase: Arc<RecallUseCase>,
        consolidate_memory_usecase: Arc<ConsolidateMemoryUseCase>,
        create_skill_usecase: Arc<CreateSkillUseCase>,
        list_skills_usecase: Arc<ListSkillsUseCase>,
        get_skill_usecase: Arc<GetSkillUseCase>,
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
            create_tasks_usecase,
            list_tasks_usecase,
            update_task_usecase,
            approve_tasks_usecase,
            delegate_task_usecase,
            submit_verification_usecase,
            unlock_files_usecase,
            check_conflicts_usecase,
            list_agents_usecase,
            get_workspace_activity_usecase,
            send_message_usecase,
            wait_events_usecase,
            submit_review_usecase,
            store_memory_usecase,
            recall_usecase,
            consolidate_memory_usecase,
            create_skill_usecase,
            list_skills_usecase,
            get_skill_usecase,
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTasksParams {
    /// Array of task objects with fields: title, objective, priority, etc.
    pub tasks: Vec<NewTaskParams>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NewTaskParams {
    pub title: String,
    pub objective: Option<String>,
    pub use_cases: Option<serde_json::Value>,
    pub prerequisites: Option<serde_json::Value>,
    pub verification_plan: Option<serde_json::Value>,
    pub required_skills: Option<serde_json::Value>,
    pub impact_analysis: Option<serde_json::Value>,
    pub priority: Option<i64>,
    pub parent_task_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTasksParams {
    pub status_filter: Option<String>,
    pub agent_id_filter: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTaskParams {
    pub task_id: String,
    pub new_status: Option<String>,
    pub progress_note: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApproveTasksParams {
    pub task_ids: Vec<String>,
    pub reject: Option<bool>,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DelegateTaskParams {
    pub task_id: String,
    pub to_agent_id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubmitVerificationParams {
    pub task_id: String,
    pub commit_sha: Option<String>,
    /// Array of steps: [{kind, command, exit_code, summary, output_digest}]
    pub steps: serde_json::Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnlockFilesParams {
    pub file_paths: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckConflictsParams {
    pub file_paths: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListAgentsParams {
    pub include_disconnected: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetWorkspaceActivityParams {
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendMessageParams {
    pub to_agent_id: Option<String>,
    pub role_filter: Option<String>,
    pub kind: String,
    pub payload: serde_json::Value,
    pub correlation_id: Option<String>,
    pub requires_response: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubmitReviewParams {
    pub task_id: String,
    pub verdict: String,
    pub findings: Option<serde_json::Value>,
    pub task_revision: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StoreMemoryParams {
    pub content: String,
    pub entry_type: Option<String>,
    pub tags: Vec<String>,
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecallParams {
    pub query: String,
    pub top_k: Option<usize>,
    pub min_score: Option<f32>,
    pub type_filter: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClassifyParams {
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateSkillParams {
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub steps: Vec<String>,
    pub source_memories: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSkillsParams {
    pub category_filter: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSkillParams {
    pub skill_id: String,
}

// ===== Helpers for Envelopes =====
async fn fetch_inbox_state(conn: &tokio_rusqlite::Connection, agent_id: &str) -> InboxState {
    let agent_id = agent_id.to_string();
    let result = conn
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT m.message_id, m.from_agent_id, m.kind, m.payload \
                 FROM agent_messages m \
                 WHERE (m.to_agent_id = ?1 OR m.to_agent_id IS NULL) \
                   AND m.delivered_at IS NULL \
                 ORDER BY m.created_at DESC \
                 LIMIT 20",
            )?;

            let rows = stmt
                .query_map([agent_id], |row| {
                    let message_id: String = row.get(0)?;
                    let from_id: String = row.get(1)?;
                    let kind: String = row.get(2)?;
                    let payload_str: String = row.get(3)?;
                    let payload: serde_json::Value =
                        serde_json::from_str(&payload_str).unwrap_or_default();

                    let summary = payload
                        .get("text")
                        .or_else(|| payload.get("summary"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Incoming message")
                        .to_string();

                    Ok(UrgentMessage {
                        message_id,
                        kind,
                        from: from_id,
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
            let count: usize = conn
                .query_row(
                    "SELECT count(*) FROM agents WHERE workspace_id = ?1 AND state != 'disconnected'",
                    [ws_id1],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            Ok(count)
        })
        .await
        .unwrap_or(0);

    let gates_count: usize = conn
        .call(move |conn| {
            let count: usize = conn
                .query_row(
                    "SELECT count(*) FROM tasks WHERE workspace_id = ?1 \
                 AND status IN ('spec_review', 'awaiting_approval', 'verification', 'code_review')",
                    [ws_id2],
                    |row| row.get(0),
                )
                .unwrap_or(0);
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
    override_next_step: Option<String>,
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

    let protocol_next_step = override_next_step.or_else(|| {
        if inbox.unread > 0 {
            Some(format!(
                "You have {} unread message(s). Use co_force_wait_events to process them.",
                inbox.unread
            ))
        } else {
            None
        }
    });

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

/// Guard macro to ensure agent is checked in.
macro_rules! require_session {
    ($self:expr) => {{
        let agent_id_opt = $self.agent_id.lock().await.clone();
        let workspace_id_opt = $self.workspace_id.lock().await.clone();
        match (agent_id_opt, workspace_id_opt) {
            (Some(aid), Some(wid)) => {
                let conn = $self.db_conn.clone();
                let aid_clone = aid.clone();
                tokio::spawn(async move {
                    let now = chrono::Utc::now().to_rfc3339();
                    let _ = conn
                        .call(move |c| {
                            let res = c.execute(
                                "UPDATE agents SET last_seen = ?1 WHERE agent_id = ?2",
                                rusqlite::params![now, aid_clone],
                            );
                            match res {
                                Ok(_) => Ok(()),
                                Err(e) => Err(tokio_rusqlite::Error::Rusqlite(e)),
                            }
                        })
                        .await;
                });
                (aid, wid)
            }
            _ => {
                return make_error_response(
                    "CHECK_IN_REQUIRED",
                    "Protocol Violation: You must call co_force_check_in first.",
                    "co_force_check_in(workspace_path, agent_name, role)",
                );
            }
        }
    }};
}

// ===== Tool Implementation =====
#[tool_router]
impl CoForceMcp {
    // -------------------------------------------------------------------------
    // IDENTITY TOOLS
    // -------------------------------------------------------------------------

    #[tool(
        description = "MANDATORY first call of every session. All other tools fail with CHECK_IN_REQUIRED until called. Registers your session, receives workspace rules, team context, and next steps."
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
                *self.agent_id.lock().await = Some(res.agent_id.clone());
                *self.workspace_id.lock().await = Some(res.workspace_id.clone());

                let aid = res.agent_id.clone();
                let wid = res.workspace_id.clone();
                let next_step = if res.onboarding_required {
                    Some("Call co_force_guide() once before taking any task.".to_string())
                } else {
                    None
                };
                make_envelope_response(&self.db_conn, Some(&aid), Some(&wid), Some(res), next_step).await
            }
            Err(e) => make_error_response(
                "INTERNAL_ERROR",
                &format!("Check-in failed: {e}"),
                "Retry the check-in call",
            ),
        }
    }

    #[tool(
        description = "Returns your current session identity, role, workspace, and team context. \
        Call after check_in to confirm your identity and assigned tasks."
    )]
    async fn co_force_whoami(&self) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);

        let aid = agent_id.clone();
        let wid = workspace_id.clone();

        let data = serde_json::json!({
            "agent_id": agent_id,
            "workspace_id": workspace_id,
            "role": "developer", // TODO: load from session
            "protocol": "Co-Force v1.0",
        });

        make_envelope_response(&self.db_conn, Some(&aid), Some(&wid), Some(data), None).await
    }

    #[tool(
        description = "Returns the Co-Force protocol guide, dynamic quality policy, active team/locks, and standard examples."
    )]
    async fn co_force_guide(&self) -> CallToolResult {
        let (_agent_id, workspace_id) = require_session!(self);

        let policy_res: Option<co_force_core::quality::state_machine::QualityPolicy> = self.db_conn.call({
            let wid = workspace_id.clone();
            move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT reviews_required, reviewer_must_differ, require_recheck, \
                     require_verification_evidence, required_evidence_kinds, critique_fanout, \
                     max_rework_cycles, definition_of_done FROM quality_policies \
                     WHERE workspace_id = ?1"
                )?;
                let mut rows = stmt.query(rusqlite::params![&wid])?;
                if let Some(row) = rows.next()? {
                    let ev_str: String = row.get(4)?;
                    let dod_str: String = row.get(7)?;
                    let kinds: Vec<String> = serde_json::from_str(&ev_str).unwrap_or_default();
                    let dod: Vec<String> = serde_json::from_str(&dod_str).unwrap_or_default();
                    Ok(Some(co_force_core::quality::state_machine::QualityPolicy {
                        reviews_required: row.get(0)?,
                        reviewer_must_differ: row.get(1)?,
                        require_recheck: row.get(2)?,
                        require_verification_evidence: row.get(3)?,
                        required_evidence_kinds: kinds,
                        critique_fanout: row.get(5)?,
                        max_rework_cycles: row.get(6)?,
                        definition_of_done: dod,
                    }))
                } else {
                    Ok(None)
                }
            }
        }).await.unwrap_or(None);

        let policy = policy_res.unwrap_or_else(|| co_force_core::quality::state_machine::QualityPolicy {
            reviews_required: 1,
            reviewer_must_differ: "agent".to_string(),
            require_recheck: false,
            require_verification_evidence: true,
            required_evidence_kinds: vec!["test".to_string()],
            critique_fanout: 0,
            max_rework_cycles: 3,
            definition_of_done: vec![],
        });

        let team_res = self.db_conn.call({
            let wid = workspace_id.clone();
            move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT agent_id, name, role, state FROM agents \
                     WHERE workspace_id = ?1 AND state != 'disconnected'"
                )?;
                let mut rows = stmt.query(rusqlite::params![&wid])?;
                let mut agents = Vec::new();
                while let Some(row) = rows.next()? {
                    agents.push(serde_json::json!({
                        "agent_id": row.get::<_, String>(0)?,
                        "name": row.get::<_, String>(1)?,
                        "role": row.get::<_, String>(2)?,
                        "state": row.get::<_, String>(3)?,
                    }));
                }

                let mut stmt_locks = conn.prepare(
                    "SELECT file_path, agent_id FROM file_locks \
                     WHERE workspace_id = ?1"
                )?;
                let mut rows_locks = stmt_locks.query(rusqlite::params![&wid])?;
                let mut locks = Vec::new();
                while let Some(row) = rows_locks.next()? {
                    locks.push(serde_json::json!({
                        "file_path": row.get::<_, String>(0)?,
                        "held_by": row.get::<_, String>(1)?,
                    }));
                }

                Ok((agents, locks))
            }
        }).await.unwrap_or((vec![], vec![]));
        let (active_agents, active_locks) = team_res;

        let backlog = self.db_conn.call({
            let wid = workspace_id.clone();
            move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT task_id, title, status, assigned_agent_id FROM tasks \
                     WHERE workspace_id = ?1 AND status IN ('approved', 'in_progress')"
                )?;
                let mut rows = stmt.query(rusqlite::params![&wid])?;
                let mut tasks = Vec::new();
                while let Some(row) = rows.next()? {
                    tasks.push(serde_json::json!({
                        "task_id": row.get::<_, String>(0)?,
                        "title": row.get::<_, String>(1)?,
                        "status": row.get::<_, String>(2)?,
                        "assignee": row.get::<_, Option<String>>(3)?,
                    }));
                }
                Ok(tasks)
            }
        }).await.unwrap_or(vec![]);

        let examples = serde_json::json!({
            "co_force_create_tasks": {
                "description": "Create tasks with objective and verification plan matching the quality policy.",
                "example_call": {
                    "tasks": [{
                        "title": "Implement feature X",
                        "objective": "Add feature X supporting use cases Y and Z.",
                        "use_cases": ["User requests X", "Server processes X"],
                        "verification_plan": "Write unit tests and run cargo test",
                        "required_skills": ["Rust", "SQLite"]
                    }]
                }
            },
            "co_force_submit_verification": {
                "description": "Submit verification evidence to move task from in_progress to verification.",
                "example_call": {
                    "task_id": "task-123",
                    "task_revision": 1,
                    "evidence": [{
                        "kind": "test",
                        "content": "cargo test results: 12 passed, 0 failed",
                        "exit_code": 0,
                        "metadata": { "commit_sha": "a1b2c3d4e5f6..." }
                    }]
                }
            },
            "co_force_submit_review": {
                "description": "Submit review verdict and findings for an assigned task.",
                "example_call": {
                    "task_id": "task-123",
                    "verdict": "approved",
                    "findings": [{
                        "severity": "medium",
                        "file_path": "src/lib.rs",
                        "line_number": 42,
                        "message": "Potential memory leak in connection pooling."
                    }]
                }
            }
        });

        let common_errors = serde_json::json!({
            "CHECK_IN_REQUIRED": {
                "explanation": "No active session registered.",
                "recovery_action": "Call co_force_check_in first."
            },
            "LOCK_CONFLICT": {
                "explanation": "Another agent holds locks on your target files.",
                "recovery_action": "Call co_force_check_conflicts and coordinate or wait."
            },
            "GATE_VIOLATION": {
                "explanation": "Invalid task transition, e.g. trying to complete without review, or self-review.",
                "recovery_action": "Follow task lifecycle; submit verification and request review from teammate."
            },
            "EVIDENCE_STALE": {
                "explanation": "Code has been modified since the last verification submission.",
                "recovery_action": "Re-run verification tests and call submit_verification again."
            }
        });

        let guide = serde_json::json!({
            "protocol_version": "1.0",
            "active_quality_policy": policy,
            "current_team": {
                "agents": active_agents,
                "locks": active_locks
            },
            "backlog": backlog,
            "standard_examples": examples,
            "common_errors": common_errors,
            "playbooks": {
                "developer": "check_in -> recall -> lock -> code -> submit_verification -> handle findings -> store_memory",
                "reviewer": "check_in -> wait_events loop -> receives review_request -> reads code -> runs tests -> submit_review",
                "critic": "receives critique_request -> submit_critique"
            }
        });

        CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&guide).unwrap_or_default(),
        )])
    }

    // -------------------------------------------------------------------------
    // TASK MANAGEMENT TOOLS
    // -------------------------------------------------------------------------

    #[tool(
        description = "Creates one or more tasks in the workspace. Tasks start in Draft status \
        and proceed through spec_review → awaiting_approval → approved before work begins."
    )]
    async fn co_force_create_tasks(&self, params: Parameters<CreateTasksParams>) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::types::TaskId;
        let tasks: Vec<NewTaskInput> = args
            .tasks
            .into_iter()
            .map(|t| NewTaskInput {
                title: t.title,
                objective: t.objective,
                use_cases: t.use_cases,
                prerequisites: t.prerequisites,
                verification_plan: t.verification_plan,
                required_skills: t.required_skills,
                impact_analysis: t.impact_analysis,
                priority: t.priority.unwrap_or(0),
                parent_task_id: t.parent_task_id.map(TaskId::from),
            })
            .collect();

        let req = CreateTasksRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            agent_id: AgentId::from(agent_id.as_str()),
            tasks,
        };

        match self.create_tasks_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "TASK_CREATE_FAILED",
                &format!("Failed to create tasks: {e}"),
                "Check task format and retry",
            ),
        }
    }

    #[tool(
        description = "Lists tasks in the workspace. Filter by status (draft|spec_review|awaiting_approval|\
        approved|in_progress|verification|code_review|rework|completed|blocked) or agent_id."
    )]
    async fn co_force_list_tasks(&self, params: Parameters<ListTasksParams>) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::types::TaskStatus;
        let status_filter = args
            .status_filter
            .as_deref()
            .and_then(TaskStatus::from_str_value);

        let agent_id_filter = args.agent_id_filter.as_deref().map(AgentId::from);

        let req = ListTasksRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            status_filter,
            agent_id_filter,
        };

        match self.list_tasks_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "INTERNAL_ERROR",
                &format!("Failed to list tasks: {e}"),
                "Retry listing tasks",
            ),
        }
    }

    #[tool(description = "Updates a task's status or adds a progress note. \
        GATE: Cannot set status=completed directly — must use co_force_submit_verification first.")]
    async fn co_force_update_task(&self, params: Parameters<UpdateTaskParams>) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::types::{TaskId, TaskStatus};
        let new_status = args
            .new_status
            .as_deref()
            .and_then(TaskStatus::from_str_value);

        let req = UpdateTaskRequest {
            task_id: TaskId::from(args.task_id.as_str()),
            agent_id: AgentId::from(agent_id.as_str()),
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            new_status,
            progress_note: args.progress_note,
        };

        match self.update_task_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "GATE_VIOLATION",
                &format!("{e}"),
                "Check state machine transitions or use co_force_submit_verification",
            ),
        }
    }

    #[tool(
        description = "Approves or rejects tasks that are in awaiting_approval status. \
        Only users/PM agents should call this. Rejected tasks return to Draft."
    )]
    async fn co_force_approve_tasks(
        &self,
        params: Parameters<ApproveTasksParams>,
    ) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::types::TaskId;
        let req = ApproveTasksRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            task_ids: args
                .task_ids
                .into_iter()
                .map(|s| TaskId::from(s.as_str()))
                .collect(),
            approver_agent_id: AgentId::from(agent_id.as_str()),
            reject: args.reject.unwrap_or(false),
            rejection_reason: args.rejection_reason,
        };

        match self.approve_tasks_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "INTERNAL_ERROR",
                &format!("Failed to approve tasks: {e}"),
                "Ensure tasks are in awaiting_approval status",
            ),
        }
    }

    #[tool(
        description = "Delegates a task to another agent. Useful when agent is rate-limited or \
        needs to hand off specific work without full handover."
    )]
    async fn co_force_delegate_task(
        &self,
        params: Parameters<DelegateTaskParams>,
    ) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::types::TaskId;
        let req = DelegateTaskRequest {
            task_id: TaskId::from(args.task_id.as_str()),
            from_agent_id: AgentId::from(agent_id.as_str()),
            to_agent_id: AgentId::from(args.to_agent_id.as_str()),
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            reason: args.reason,
        };

        match self.delegate_task_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "DELEGATE_FAILED",
                &format!("Failed to delegate task: {e}"),
                "Check target agent_id exists and is online",
            ),
        }
    }

    #[tool(
        description = "MANDATORY: Submits verification evidence before requesting code review. \
        Must include at least 1 step with kind='test' and exit_code=0 (real test output required)."
    )]
    async fn co_force_submit_verification(
        &self,
        params: Parameters<SubmitVerificationParams>,
    ) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::types::TaskId;
        let req = SubmitVerificationRequest {
            task_id: TaskId::from(args.task_id.as_str()),
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            agent_id: AgentId::from(agent_id.as_str()),
            commit_sha: args.commit_sha,
            steps: args.steps,
        };

        match self.submit_verification_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    Some("Verification submitted. Now request code review via co_force_send_message to a reviewer agent.".to_string()),
                )
                .await
            }
            Err(e) => make_error_response(
                "EVIDENCE_INVALID",
                &format!("{e}"),
                "Include real test output with exit_code=0",
            ),
        }
    }

    // -------------------------------------------------------------------------
    // FILE LOCK TOOLS
    // -------------------------------------------------------------------------

    #[tool(
        description = "MANDATORY: MUST be called before modifying any files. Requests exclusive locks. \
        Returns conflict info if another agent holds locks."
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
                    "co_force_check_in(workspace_path, agent_name, role)",
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
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "LOCK_CONFLICT",
                &format!("Failed to lock files: {e}"),
                "Use co_force_check_conflicts to see who holds the locks",
            ),
        }
    }

    #[tool(
        description = "Releases file locks held by you. Call after completing work on a task or \
        when you no longer need exclusive access to the files."
    )]
    async fn co_force_unlock_files(&self, params: Parameters<UnlockFilesParams>) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        let req = UnlockFilesRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            agent_id: AgentId::from(agent_id.as_str()),
            file_paths: args.file_paths,
        };

        match self.unlock_files_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "UNLOCK_FAILED",
                &format!("Failed to unlock files: {e}"),
                "Retry or check file paths",
            ),
        }
    }

    #[tool(
        description = "Checks which files are currently locked and by whom before claiming them. \
        Use before co_force_lock_files to avoid conflicts."
    )]
    async fn co_force_check_conflicts(
        &self,
        params: Parameters<CheckConflictsParams>,
    ) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        let req = CheckConflictsRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            file_paths: args.file_paths,
        };

        match self.check_conflicts_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "INTERNAL_ERROR",
                &format!("Failed to check conflicts: {e}"),
                "Retry",
            ),
        }
    }

    // -------------------------------------------------------------------------
    // AWARENESS TOOLS
    // -------------------------------------------------------------------------

    #[tool(
        description = "Lists all agents in the workspace. Shows their state, role, and assigned task."
    )]
    async fn co_force_list_agents(&self, params: Parameters<ListAgentsParams>) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        let req = ListAgentsRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            include_disconnected: args.include_disconnected.unwrap_or(false),
        };

        match self.list_agents_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "INTERNAL_ERROR",
                &format!("Failed to list agents: {e}"),
                "Retry",
            ),
        }
    }

    #[tool(
        description = "Get recent activity stream for the workspace — task updates, file edits, \
        context shares, handovers. Useful for PM agents monitoring team progress."
    )]
    async fn co_force_get_workspace_activity(
        &self,
        params: Parameters<GetWorkspaceActivityParams>,
    ) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        let req = GetWorkspaceActivityRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            limit: args.limit.unwrap_or(50),
        };

        match self.get_workspace_activity_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "INTERNAL_ERROR",
                &format!("Failed to get activity: {e}"),
                "Retry",
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
                    "co_force_check_in(workspace_path, agent_name, role)",
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
                    None,
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

    // -------------------------------------------------------------------------
    // MESSAGING TOOLS
    // -------------------------------------------------------------------------

    #[tool(
        description = "Sends a message to another agent or broadcasts to agents with a specific role. \
        Used for review_request, question, info, and critique_request."
    )]
    async fn co_force_send_message(&self, params: Parameters<SendMessageParams>) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::quality::messaging::SendMessageRequest;
        let req = SendMessageRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            from_agent_id: AgentId::from(agent_id.as_str()),
            to_agent_id: args.to_agent_id.as_deref().map(AgentId::from),
            role_filter: args.role_filter,
            kind: args.kind,
            payload: args.payload,
            correlation_id: args.correlation_id,
            requires_response: args.requires_response.unwrap_or(false),
        };

        match self.send_message_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "MESSAGE_FAILED",
                &format!("Failed to send message: {e}"),
                "Check target agent_id or role_filter",
            ),
        }
    }

    #[tool(
        description = "Long-polls for new messages and events (up to 55 seconds). \
        Returns immediately if there are pending messages. Use this instead of busy-polling."
    )]
    async fn co_force_wait_events(&self) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);

        match self
            .wait_events_usecase
            .execute(
                &AgentId::from(agent_id.as_str()),
                &WorkspaceId::from(workspace_id.as_str()),
            )
            .await
        {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => {
                make_error_response("WAIT_FAILED", &format!("Wait events failed: {e}"), "Retry")
            }
        }
    }

    #[tool(description = "Shares specific context blocks to another agent (lazy resolution).")]
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
                    "co_force_check_in(workspace_path, agent_name, role)",
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
                    None,
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

    // -------------------------------------------------------------------------
    // QUALITY TOOLS
    // -------------------------------------------------------------------------

    #[tool(
        description = "Submits a code review verdict for a task in code_review status. \
        GATE: Reviewer must differ from task author. Verdict: 'approved' | 'changes_requested'."
    )]
    async fn co_force_submit_review(
        &self,
        params: Parameters<SubmitReviewParams>,
    ) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::quality::review::SubmitReviewRequest;
        use co_force_core::types::TaskId;

        let req = SubmitReviewRequest {
            task_id: TaskId::from(args.task_id.as_str()),
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            reviewer_agent_id: AgentId::from(agent_id.as_str()),
            reviewer_provider: None, // TODO: load from session
            verdict: args.verdict,
            findings: args.findings,
            task_revision: args.task_revision,
        };

        match self.submit_review_usecase.execute(req).await {
            Ok(res) => {
                let next_action = res.next_action.clone();
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    Some(next_action),
                )
                .await
            }
            Err(e) => make_error_response(
                "REVIEW_GATE_VIOLATION",
                &format!("{e}"),
                "Ensure task is in code_review status and you are not the task author",
            ),
        }
    }

    // -------------------------------------------------------------------------
    // A2A HANDOVER TOOLS
    // -------------------------------------------------------------------------

    #[tool(
        description = "MANDATORY: MUST be called early upon rate limiting or context exhaustion."
    )]
    async fn co_force_handover(&self, params: Parameters<HandoverParams>) -> CallToolResult {
        let agent_id_opt = self.agent_id.lock().await.clone();
        let workspace_id_opt = self.workspace_id.lock().await.clone();

        let (agent_id, workspace_id) = match (agent_id_opt, workspace_id_opt) {
            (Some(aid), Some(wid)) => (aid, wid),
            _ => {
                return make_error_response(
                    "CHECK_IN_REQUIRED",
                    "Protocol Violation: You must check-in first.",
                    "co_force_check_in(workspace_path, agent_name, role)",
                );
            }
        };

        let args = params.0;
        let cooldown = args.provider_cooldown_until.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|d| d.with_timezone(&chrono::Utc))
        });

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
                    None,
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

    #[tool(description = "Spawns a new subagent (Lane 2 directive or Lane 3 local process).")]
    async fn co_force_spawn_agent(&self, params: Parameters<SpawnParams>) -> CallToolResult {
        let agent_id_opt = self.agent_id.lock().await.clone();
        let workspace_id_opt = self.workspace_id.lock().await.clone();

        let (agent_id, workspace_id) = match (agent_id_opt, workspace_id_opt) {
            (Some(aid), Some(wid)) => (aid, wid),
            _ => {
                return make_error_response(
                    "CHECK_IN_REQUIRED",
                    "Protocol Violation: You must check-in first.",
                    "co_force_check_in(workspace_path, agent_name, role)",
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
                    None,
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

    // -------------------------------------------------------------------------
    // RAG & SKILLS TOOLS
    // -------------------------------------------------------------------------

    #[tool(
        description = "Stores a new memory, rule, or skill context into the workspace memory. \
        Auto-classifies content into MEMORY | KNOWLEDGE | SKILL if entry_type is omitted."
    )]
    async fn co_force_store_memory(&self, params: Parameters<StoreMemoryParams>) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::llm::StoreMemoryRequest;

        let req = StoreMemoryRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            agent_id: agent_id.clone(),
            content: args.content,
            entry_type: args.entry_type,
            tags: args.tags,
            source: args.source,
        };

        match self.store_memory_usecase.execute(req).await {
            Ok(res) => {
                let status_msg = res.message.clone();
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    status_msg,
                )
                .await
            }
            Err(e) => make_error_response(
                "STORE_FAILED",
                &format!("Failed to store memory: {e}"),
                "Verify input format and retry",
            ),
        }
    }

    #[tool(
        description = "Semantic recall from the workspace knowledge base. Returns highly relevant \
        context snippets based on query similarity. Checks for index availability."
    )]
    async fn co_force_recall(&self, params: Parameters<RecallParams>) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::llm::RecallRequest;

        let req = RecallRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            query: args.query,
            top_k: args.top_k.unwrap_or(5),
            min_score: args.min_score.unwrap_or(0.7),
            type_filter: args.type_filter,
        };

        match self.recall_usecase.execute(req).await {
            Ok(res) => {
                let status_msg = res.index_status.clone();
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    status_msg,
                )
                .await
            }
            Err(e) => make_error_response(
                "RECALL_FAILED",
                &format!("{e}"),
                "Ensure Ollama is running and query is valid",
            ),
        }
    }

    #[tool(
        description = "Consolidates near-duplicate memories in the workspace by grouping entries \
        with similarity > 0.92, preserving the latest accessed ones."
    )]
    async fn co_force_consolidate_memory(&self) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);

        match self
            .consolidate_memory_usecase
            .execute(&WorkspaceId::from(workspace_id.as_str()))
            .await
        {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "CONSOLIDATION_FAILED",
                &format!("Consolidation failed: {e}"),
                "Retry consolidation",
            ),
        }
    }

    #[tool(
        description = "Manually registers a reified, step-by-step procedural skill in the database."
    )]
    async fn co_force_create_skill(&self, params: Parameters<CreateSkillParams>) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::llm::CreateSkillRequest;

        let req = CreateSkillRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            name: args.name,
            description: args.description,
            category: args.category,
            steps: args.steps,
            source_memories: args.source_memories,
        };

        match self.create_skill_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "SKILL_CREATE_FAILED",
                &format!("Failed to create skill: {e}"),
                "Verify inputs",
            ),
        }
    }

    #[tool(
        description = "Lists all registered skills in the workspace, optionally filtering by category."
    )]
    async fn co_force_list_skills(&self, params: Parameters<ListSkillsParams>) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::llm::ListSkillsRequest;

        let req = ListSkillsRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            category_filter: args.category_filter,
        };

        match self.list_skills_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "SKILL_LIST_FAILED",
                &format!("Failed to list skills: {e}"),
                "Retry listing",
            ),
        }
    }

    #[tool(description = "Retrieves step-by-step procedural details of a specific skill by ID.")]
    async fn co_force_get_skill(&self, params: Parameters<GetSkillParams>) -> CallToolResult {
        let (agent_id, workspace_id) = require_session!(self);
        let args = params.0;

        use co_force_core::llm::GetSkillRequest;

        let req = GetSkillRequest {
            workspace_id: WorkspaceId::from(workspace_id.as_str()),
            skill_id: args.skill_id,
        };

        match self.get_skill_usecase.execute(req).await {
            Ok(res) => {
                make_envelope_response(
                    &self.db_conn,
                    Some(&agent_id),
                    Some(&workspace_id),
                    Some(res),
                    None,
                )
                .await
            }
            Err(e) => make_error_response(
                "SKILL_GET_FAILED",
                &format!("Failed to retrieve skill: {e}"),
                "Ensure skill ID is correct",
            ),
        }
    }
}

use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
    middleware::Next,
    response::IntoResponse,
    Json,
};

#[derive(Debug, Deserialize)]
pub struct EnrollRequest {
    pub enroll_token: String,
    pub label: Option<String>,
    pub workspace_hint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EnrollResponse {
    pub agent_token: String,
    pub workspace_id: String,
    pub server_url: String,
    pub team_online: Vec<String>,
}

// 1. Health handler
async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

// 2. Setup script handler
async fn setup_handler(
    axum::extract::State(public_url): axum::extract::State<String>,
) -> impl IntoResponse {
    let script = format!(
        r#"#!/bin/sh
# Co-Force Client Automatic Setup Script

set -e

TOKEN=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --token)
            TOKEN="$2"
            shift 2
            ;;
        *)
            shift
            ;;
    esac
done

if [ -z "$TOKEN" ]; then
    echo "❌ Error: --token <enrollment_token> is required."
    exit 1
fi

echo "Connecting and enrolling machine with Co-Force Server..."
RESPONSE=$(curl -fsSL -X POST -H "Content-Type: application/json" \
  -d "{{\"enroll_token\":\"$TOKEN\",\"label\":\"$(hostname)\"}}" \
  "{public_url}/api/enroll")

AGENT_TOKEN=$(echo "$RESPONSE" | grep -o '"agent_token":"[^"]*' | grep -o '[^"]*$')
WORKSPACE_ID=$(echo "$RESPONSE" | grep -o '"workspace_id":"[^"]*' | grep -o '[^"]*$')

echo "✅ Enrolled successfully. Agent token received."

# Write ~/.claude.json
if command -v claude >/dev/null 2>&1; then
    echo "Configuring Claude Code..."
    claude mcp add -s local -t http co-force "{public_url}/mcp" --header "Authorization: Bearer $AGENT_TOKEN"
fi

# Cursor config
CURSOR_DIR="$HOME/.cursor"
if [ -d "$CURSOR_DIR" ]; then
    echo "Configuring Cursor..."
    mkdir -p "$CURSOR_DIR"
    echo "{{\"mcpServers\":{{\"co-force\":{{\"type\":\"http\",\"url\":\"{public_url}/mcp\",\"headers\":{{\"Authorization\":\"Bearer $AGENT_TOKEN\"}}}}}}}}" > "$CURSOR_DIR/mcp.json"
fi

# Codex config
CODEX_DIR="$HOME/.codex"
if command -v codex >/dev/null 2>&1 || [ -d "$CODEX_DIR" ]; then
    echo "Configuring Codex..."
    mkdir -p "$CODEX_DIR"
    cat << EOF > "$CODEX_DIR/config.toml"
[mcp_servers.co-force]
command = "npx"
args = ["-y", "mcp-remote-stdio-shim", "{public_url}/mcp", "Bearer $AGENT_TOKEN"]
EOF
fi

# Antigravity config
AGY_DIR="$HOME/.gemini/config"
if command -v agy >/dev/null 2>&1 || [ -d "$HOME/.gemini" ]; then
    echo "Configuring Antigravity..."
    mkdir -p "$AGY_DIR"
    echo "{{\"mcpServers\":{{\"co-force\":{{\"type\":\"http\",\"url\":\"{public_url}/mcp\",\"headers\":{{\"Authorization\":\"Bearer $AGENT_TOKEN\"}}}}}}}}" > "$AGY_DIR/mcp_config.json"
fi

# VSCode or generic local .mcp.json fallback
echo "Configuring generic fallback (.mcp.json)..."
echo "{{\"mcpServers\":{{\"co-force\":{{\"type\":\"http\",\"url\":\"{public_url}/mcp\",\"headers\":{{\"Authorization\":\"Bearer $AGENT_TOKEN\"}}}}}}}}" > .mcp.json
if ! grep -q ".mcp.json" .gitignore 2>/dev/null; then
    echo "\n.mcp.json" >> .gitignore
fi

# Rule injection into AGENTS.md
echo "Injecting rules block into AGENTS.md..."
cat << 'EOF' >> AGENTS.md

<!-- CO-FORCE RULES -->
# Co-Force Agent Protocol
- You MUST check-in using co_force_check_in before any action.
- You MUST lock files using co_force_lock_files before modifying them.
- You MUST submit verification tests with co_force_submit_verification.
<!-- END CO-FORCE RULES -->
EOF

echo "🎉 Co-Force Client configuration written. You are ready to go!"
"#
    );

    (
        [(axum::http::header::CONTENT_TYPE, "application/x-sh")],
        script,
    )
}

// 3. Enroll handler
async fn enroll_handler(
    axum::extract::State((server_db, public_url)): axum::extract::State<(
        co_force_core::db::ServerDatabase,
        String,
    )>,
    Json(payload): Json<EnrollRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    // Validate enrollment token
    let token_info = server_db
        .validate_token(&payload.enroll_token)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    if token_info.kind != "enrollment" {
        return Err(StatusCode::FORBIDDEN);
    }

    // Revoke enrollment token (single-use)
    let _ = server_db.revoke_token(&token_info.token_id).await;

    // Issue long-term agent token
    let label = payload.label.or(Some("Agent-Client".to_string()));
    let (agent_token, _agent_token_info) = server_db
        .issue_token(label, "agent", "*", None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Register workspace
    let ws_id = payload
        .workspace_hint
        .unwrap_or_else(|| "ws-default".to_string());
    let _ = server_db
        .register_workspace(&ws_id, "default-workspace", "/tmp/default")
        .await;

    Ok(Json(EnrollResponse {
        agent_token,
        workspace_id: ws_id,
        server_url: public_url,
        team_online: vec!["Agent-Alpha (reviewer)".to_string()],
    }))
}

// 4. Auth middleware
async fn auth_middleware(
    axum::extract::State(server_db): axum::extract::State<co_force_core::db::ServerDatabase>,
    req: Request<Body>,
    next: Next,
) -> Result<Response<Body>, StatusCode> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok());

    let token = match auth_header {
        Some(auth) if auth.starts_with("Bearer ") => &auth[7..],
        _ => return Err(StatusCode::UNAUTHORIZED),
    };

    match server_db.validate_token(token).await {
        Ok(api_token) => {
            let _ = server_db
                .log_audit(Some(api_token.token_id), None, "mcp_access", "success")
                .await;
            Ok(next.run(req).await)
        }
        Err(_) => {
            let _ = server_db
                .log_audit(None, None, "mcp_access", "failed_unauthorized")
                .await;
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

#[tool_handler]
impl ServerHandler for CoForceMcp {}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

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

    // Spawn reclaim daemon monitoring agent heartbeats & grace periods
    let bus_reclaim = bus.clone();
    let agent_repo_reclaim = agent_repo.clone();
    let task_repo_reclaim = task_repo.clone();
    let lock_repo_reclaim = lock_repo.clone();
    let activity_repo_reclaim = activity_repo.clone();
    let handover_repo_reclaim = handover_repo.clone();
    let db_conn_reclaim = db_conn.clone();
    tokio::spawn(async move {
        co_force_core::orchestration::reclaim::run_reclaim_daemon(
            bus_reclaim,
            agent_repo_reclaim,
            task_repo_reclaim,
            lock_repo_reclaim,
            activity_repo_reclaim,
            handover_repo_reclaim,
            db_conn_reclaim,
            co_force_core::types::WorkspaceId::from("default"),
            std::time::Duration::from_secs(30),  // disconnect_timeout
            std::time::Duration::from_secs(120), // reclaim_timeout
            std::time::Duration::from_secs(10),  // poll_interval
        )
        .await;
    });

    // === Instantiate Use Cases ===

    // Core
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

    // Task management
    let create_tasks = Arc::new(CreateTasksUseCase::new(
        task_repo.clone(),
        activity_repo.clone(),
        bus.clone(),
    ));
    let list_tasks = Arc::new(ListTasksUseCase::new(task_repo.clone()));
    let update_task = Arc::new(UpdateTaskUseCase::new(
        task_repo.clone(),
        activity_repo.clone(),
        db.conn().clone(),
        agent_repo.clone(),
        bus.clone(),
    ));
    let approve_tasks = Arc::new(ApproveTasksUseCase::new(
        task_repo.clone(),
        activity_repo.clone(),
        bus.clone(),
    ));
    let delegate_task = Arc::new(DelegateTaskUseCase::new(
        task_repo.clone(),
        agent_repo.clone(),
        activity_repo.clone(),
    ));
    let submit_verification = Arc::new(SubmitVerificationUseCase::new(
        task_repo.clone(),
        activity_repo.clone(),
        db.conn().clone(),
        bus.clone(),
    ));
    let unlock_files = Arc::new(UnlockFilesUseCase::new(
        lock_repo.clone(),
        activity_repo.clone(),
    ));
    let check_conflicts = Arc::new(CheckConflictsUseCase::new(lock_repo.clone()));
    let list_agents = Arc::new(ListAgentsUseCase::new(agent_repo.clone()));
    let get_workspace_activity = Arc::new(GetWorkspaceActivityUseCase::new(activity_repo.clone()));

    // Quality engine
    let send_message = Arc::new(SendMessageUseCase::new(
        db.conn().clone(),
        activity_repo.clone(),
        bus.clone(),
    ));
    let wait_events = Arc::new(WaitEventsUseCase::new(
        db.conn().clone(),
        bus.clone(),
        agent_repo.clone(),
    ));
    let submit_review = Arc::new(SubmitReviewUseCase::new(
        task_repo.clone(),
        db.conn().clone(),
        bus.clone(),
    ));

    // RAG and Skills
    let llm_provider = Arc::new(co_force_core::llm::OllamaProvider::new(
        "http://localhost:11434",
    ));
    let vector_search = Arc::new(co_force_core::llm::BruteForceCosine::new(db.conn().clone()));

    let store_memory = Arc::new(co_force_core::llm::StoreMemoryUseCase::new(
        db.conn().clone(),
        llm_provider.clone(),
    ));
    let recall = Arc::new(co_force_core::llm::RecallUseCase::new(
        vector_search.clone(),
        llm_provider.clone(),
        db.conn().clone(),
    ));
    let consolidate_memory = Arc::new(co_force_core::llm::ConsolidateMemoryUseCase::new(
        db.conn().clone(),
    ));
    let create_skill = Arc::new(co_force_core::llm::CreateSkillUseCase::new(
        db.conn().clone(),
    ));
    let list_skills = Arc::new(co_force_core::llm::ListSkillsUseCase::new(
        db.conn().clone(),
    ));
    let get_skill = Arc::new(co_force_core::llm::GetSkillUseCase::new(db.conn().clone()));

    let make_server = move || {
        Ok(CoForceMcp::new(
            check_in.clone(),
            lock_files.clone(),
            get_agent_context.clone(),
            share_context.clone(),
            handover.clone(),
            spawn.clone(),
            create_tasks.clone(),
            list_tasks.clone(),
            update_task.clone(),
            approve_tasks.clone(),
            delegate_task.clone(),
            submit_verification.clone(),
            unlock_files.clone(),
            check_conflicts.clone(),
            list_agents.clone(),
            get_workspace_activity.clone(),
            send_message.clone(),
            wait_events.clone(),
            submit_review.clone(),
            store_memory.clone(),
            recall.clone(),
            consolidate_memory.clone(),
            create_skill.clone(),
            list_skills.clone(),
            get_skill.clone(),
            db_conn.clone(),
        ))
    };

    match args.transport.as_str() {
        "stdio" => {
            let stdio_server = make_server()?;
            let transport = rmcp::transport::io::stdio();
            let running = serve_server(stdio_server, transport).await?;
            running.waiting().await?;
        }
        "http" => {
            let server_db = co_force_core::db::ServerDatabase::open(&args.server_db).await?;
            // Issue a default enrollment token on startup for developer convenience
            let (enroll_raw, _) = server_db
                .issue_token(
                    Some("Default Setup Token".to_string()),
                    "enrollment",
                    "*",
                    Some(24),
                )
                .await?;
            tracing::info!("🚀 Startup enrollment token (valid 24h): {}", enroll_raw);
            tracing::info!("👉 Onboarding script URL: {}/setup", args.public_url);

            let service = StreamableHttpService::new(
                make_server,
                LocalSessionManager::default().into(),
                Default::default(),
            );

            let mcp_router = axum::Router::new().nest_service("/", service).route_layer(
                axum::middleware::from_fn_with_state(server_db.clone(), auth_middleware),
            );

            let app = axum::Router::new()
                .nest("/mcp", mcp_router)
                .route("/healthz", axum::routing::get(health_handler))
                .route(
                    "/setup",
                    axum::routing::get(setup_handler).with_state(args.public_url.clone()),
                )
                .route(
                    "/api/enroll",
                    axum::routing::post(enroll_handler)
                        .with_state((server_db.clone(), args.public_url.clone())),
                );

            let listener = tokio::net::TcpListener::bind(&args.addr).await?;
            tracing::info!("Co-Force MCP Server listening on http://{}", args.addr);
            axum::serve(listener, app).await?;
        }
        other => {
            anyhow::bail!("Unsupported transport: {}", other);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use co_force_core::db::ServerDatabase;
    use serde_json::Value;

    #[tokio::test]
    async fn test_http_endpoints_and_auth_flow() {
        let server_db = ServerDatabase::open(":memory:").await.unwrap();

        let (enroll_raw, token_info) = server_db
            .issue_token(
                Some("Test Setup Token".to_string()),
                "enrollment",
                "*",
                Some(24),
            )
            .await
            .unwrap();

        let app = axum::Router::new()
            .route("/healthz", axum::routing::get(health_handler))
            .route(
                "/setup",
                axum::routing::get(setup_handler).with_state("https://mcp.test.com".to_string()),
            )
            .route(
                "/api/enroll",
                axum::routing::post(enroll_handler)
                    .with_state((server_db.clone(), "https://mcp.test.com".to_string())),
            );

        use axum::body::Body;
        use tower::util::ServiceExt;

        // Test healthz
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");

        // Test setup
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/setup")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap();
        let script_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(script_str.contains("https://mcp.test.com/api/enroll"));

        // Test enroll
        let enroll_payload = serde_json::json!({
            "enroll_token": enroll_raw,
            "label": "test-machine",
            "workspace_hint": "ws-test-project"
        });

        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/enroll")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_vec(&enroll_payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let enroll_resp: Value = serde_json::from_slice(&body).unwrap();

        let agent_token = enroll_resp["agent_token"].as_str().unwrap();
        assert!(agent_token.starts_with("cfk_agent_"));
        assert_eq!(enroll_resp["workspace_id"], "ws-test-project");

        let validated = server_db.validate_token(agent_token).await.unwrap();
        assert_eq!(validated.kind, "agent");

        let revoked_check = server_db.validate_token(&enroll_raw).await;
        assert!(revoked_check.is_err());
    }

    #[tokio::test]
    async fn test_mcp_guide_tool() {
        let db = co_force_core::db::Database::open_in_memory().await.unwrap();
        let bus = co_force_core::orchestration::bus::WorkspaceEventBus::new(16);

        // Repositories
        let agent_repo = Arc::new(SqliteAgentRepo::new(db.conn().clone()));
        let activity_repo = Arc::new(SqliteActivityRepo::new(db.conn().clone()));
        let task_repo = Arc::new(SqliteTaskRepo::new(db.conn().clone()));
        let lock_repo = Arc::new(SqliteLockRepo::new(db.conn().clone()));
        let context_repo = Arc::new(SqliteContextRepo::new(db.conn().clone()));
        let handover_repo = Arc::new(SqliteHandoverRepo::new(db.conn().clone()));

        // Use cases
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
        let create_tasks = Arc::new(CreateTasksUseCase::new(
            task_repo.clone(),
            activity_repo.clone(),
            bus.clone(),
        ));
        let list_tasks = Arc::new(ListTasksUseCase::new(task_repo.clone()));
        let update_task = Arc::new(UpdateTaskUseCase::new(
            task_repo.clone(),
            activity_repo.clone(),
            db.conn().clone(),
            agent_repo.clone(),
            bus.clone(),
        ));
        let approve_tasks = Arc::new(ApproveTasksUseCase::new(
            task_repo.clone(),
            activity_repo.clone(),
            bus.clone(),
        ));
        let delegate_task = Arc::new(DelegateTaskUseCase::new(
            task_repo.clone(),
            agent_repo.clone(),
            activity_repo.clone(),
        ));
        let submit_verification = Arc::new(SubmitVerificationUseCase::new(
            task_repo.clone(),
            activity_repo.clone(),
            db.conn().clone(),
            bus.clone(),
        ));
        let unlock_files = Arc::new(UnlockFilesUseCase::new(
            lock_repo.clone(),
            activity_repo.clone(),
        ));
        let check_conflicts = Arc::new(CheckConflictsUseCase::new(lock_repo.clone()));
        let list_agents = Arc::new(ListAgentsUseCase::new(agent_repo.clone()));
        let get_workspace_activity = Arc::new(GetWorkspaceActivityUseCase::new(activity_repo.clone()));
        let send_message = Arc::new(SendMessageUseCase::new(
            db.conn().clone(),
            activity_repo.clone(),
            bus.clone(),
        ));
        let wait_events = Arc::new(WaitEventsUseCase::new(
            db.conn().clone(),
            bus.clone(),
            agent_repo.clone(),
        ));
        let submit_review = Arc::new(SubmitReviewUseCase::new(
            task_repo.clone(),
            db.conn().clone(),
            bus.clone(),
        ));

        let llm_provider = Arc::new(co_force_core::llm::OllamaProvider::new("http://localhost:11434"));
        let vector_search = Arc::new(co_force_core::llm::BruteForceCosine::new(db.conn().clone()));

        let store_memory = Arc::new(co_force_core::llm::StoreMemoryUseCase::new(
            db.conn().clone(),
            llm_provider.clone(),
        ));
        let recall = Arc::new(co_force_core::llm::RecallUseCase::new(
            vector_search.clone(),
            llm_provider.clone(),
            db.conn().clone(),
        ));
        let consolidate_memory = Arc::new(co_force_core::llm::ConsolidateMemoryUseCase::new(
            db.conn().clone(),
        ));
        let create_skill = Arc::new(co_force_core::llm::CreateSkillUseCase::new(
            db.conn().clone(),
        ));
        let list_skills = Arc::new(co_force_core::llm::ListSkillsUseCase::new(
            db.conn().clone(),
        ));
        let get_skill = Arc::new(co_force_core::llm::GetSkillUseCase::new(db.conn().clone()));

        let mcp = CoForceMcp::new(
            check_in,
            lock_files,
            get_agent_context,
            share_context,
            handover,
            spawn,
            create_tasks,
            list_tasks,
            update_task,
            approve_tasks,
            delegate_task,
            submit_verification,
            unlock_files,
            check_conflicts,
            list_agents,
            get_workspace_activity,
            send_message,
            wait_events,
            submit_review,
            store_memory,
            recall,
            consolidate_memory,
            create_skill,
            list_skills,
            get_skill,
            db.conn().clone(),
        );

        // Verify that calling guide before check_in fails with CHECK_IN_REQUIRED
        let res = mcp.co_force_guide().await;
        assert_eq!(res.is_error, Some(true));
        let val: serde_json::Value = serde_json::from_str(&res.content[0].as_text().unwrap().text).unwrap();
        assert_eq!(val["status"], "error");
        assert_eq!(val["error"]["code"], "CHECK_IN_REQUIRED");

        // Perform check_in
        let check_in_params = CheckInParams {
            workspace_path: "/Users/trungtran/project-x".to_string(),
            agent_name: "dev-agent-1".to_string(),
            role: "developer".to_string(),
            agent_id: Some("agent-123".to_string()),
            provider: Some("claude-code".to_string()),
            machine_id: Some("machine-abc".to_string()),
        };
        let check_in_res = mcp.co_force_check_in(Parameters(check_in_params)).await;
        assert_ne!(check_in_res.is_error, Some(true));

        // Call guide again
        let res_guide = mcp.co_force_guide().await;
        assert_ne!(res_guide.is_error, Some(true));
        let guide_val: serde_json::Value = serde_json::from_str(&res_guide.content[0].as_text().unwrap().text).unwrap();
        assert_eq!(guide_val["protocol_version"], "1.0");
        assert!(guide_val.get("active_quality_policy").is_some());
        assert!(guide_val.get("standard_examples").is_some());
    }
}
