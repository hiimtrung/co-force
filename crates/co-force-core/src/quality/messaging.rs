//! A2A Messaging use cases — SendMessage, WaitEvents (Plan 07 §4).
//!
//! Messaging is built on top of the `shared_contexts` table (extended inbox).
//! Full agent_messages table is used for typed protocol messages (reviews, critiques).

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use crate::engine::ports::{ActivityRepository, AgentRepository};
use crate::orchestration::bus::{WorkspaceEvent, WorkspaceEventBus};
use crate::types::{ActivityId, ActivityType, AgentActivity, AgentId, WorkspaceId};

// ---------------------------------------------------------------------------
// SendMessageUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SendMessageRequest {
    pub workspace_id: WorkspaceId,
    pub from_agent_id: AgentId,
    /// None = broadcast to all agents with role_filter
    pub to_agent_id: Option<AgentId>,
    /// Filter by role if to_agent_id is None
    pub role_filter: Option<String>,
    /// Message kind: info | question | review_request | critique_request | answer
    pub kind: String,
    pub payload: serde_json::Value,
    pub correlation_id: Option<String>,
    pub requires_response: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageResponse {
    pub message_id: String,
    pub delivered_to: usize,
}

pub struct SendMessageUseCase {
    conn: tokio_rusqlite::Connection,
    activity_repo: Arc<dyn ActivityRepository>,
    bus: WorkspaceEventBus,
}

impl SendMessageUseCase {
    pub fn new(
        conn: tokio_rusqlite::Connection,
        activity_repo: Arc<dyn ActivityRepository>,
        bus: WorkspaceEventBus,
    ) -> Self {
        Self {
            conn,
            activity_repo,
            bus,
        }
    }

    pub async fn execute(&self, req: SendMessageRequest) -> Result<SendMessageResponse> {
        let message_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let payload_str = serde_json::to_string(&req.payload)?;

        let mid = message_id.clone();
        let ws_id = req.workspace_id.to_string();
        let from_id = req.from_agent_id.to_string();
        let to_id = req.to_agent_id.as_ref().map(|i| i.to_string());
        let role_filter = req.role_filter.clone();
        let kind = req.kind.clone();
        let correlation_id = req.correlation_id.clone();
        let requires_response = req.requires_response;

        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO agent_messages \
                     (message_id, workspace_id, from_agent_id, to_agent_id, role_filter, \
                      kind, payload, correlation_id, requires_response, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        mid,
                        ws_id,
                        from_id,
                        to_id,
                        role_filter,
                        kind,
                        payload_str,
                        correlation_id,
                        requires_response as i32,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send message: {e}"))?;

        // Log activity
        let activity = AgentActivity {
            activity_id: ActivityId::new(),
            workspace_id: req.workspace_id.clone(),
            agent_id: req.from_agent_id.clone(),
            activity_type: ActivityType::ContextShared,
            content: Some(serde_json::json!({
                "summary": format!("Sent {} message", req.kind),
                "message_id": message_id,
                "to": req.to_agent_id.as_ref().map(|i| i.to_string()),
                "kind": req.kind,
            })),
            related_task_id: None,
            related_files: None,
            version: 1,
            occurred_at: Utc::now(),
        };
        self.activity_repo.log_activity(&activity).await?;

        // Emit event to wake up any waiting agents
        self.bus.send(WorkspaceEvent::ContextShared {
            context_id: message_id.clone(),
        });

        Ok(SendMessageResponse {
            message_id,
            delivered_to: 1,
        })
    }
}

// ---------------------------------------------------------------------------
// WaitEventsUseCase — long-poll (55 second timeout, MCP max)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMessage {
    pub message_id: String,
    pub from_agent_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub correlation_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitEventsResponse {
    pub messages: Vec<PendingMessage>,
    pub pulse: EventPulse,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPulse {
    pub tasks_at_gates: usize,
    pub agents_online: usize,
    pub pending_reviews: usize,
}

pub struct WaitEventsUseCase {
    conn: tokio_rusqlite::Connection,
    bus: WorkspaceEventBus,
    agent_repo: Arc<dyn AgentRepository>,
}

impl WaitEventsUseCase {
    pub fn new(
        conn: tokio_rusqlite::Connection,
        bus: WorkspaceEventBus,
        agent_repo: Arc<dyn AgentRepository>,
    ) -> Self {
        Self {
            conn,
            bus,
            agent_repo,
        }
    }

    /// Waits for new messages for up to 55 seconds (MCP request timeout is 60s).
    /// Returns immediately if messages are already pending.
    pub async fn execute(
        &self,
        agent_id: &AgentId,
        workspace_id: &WorkspaceId,
    ) -> Result<WaitEventsResponse> {
        // Check for immediate messages first
        let messages = self.fetch_pending_messages(agent_id, workspace_id).await?;
        if !messages.is_empty() {
            let pulse = self.fetch_pulse(workspace_id).await;
            return Ok(WaitEventsResponse {
                messages,
                pulse,
                timed_out: false,
            });
        }

        // Subscribe to bus events and wait up to 55 seconds
        let mut rx = self.bus.subscribe();
        let wait_duration = Duration::from_secs(55);

        let received = timeout(wait_duration, async {
            loop {
                match rx.recv().await {
                    Ok(_) => break true,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break false,
                }
            }
        })
        .await;

        let timed_out = received.unwrap_or(false) == false;

        // Fetch messages again after wakeup or timeout
        let messages = self.fetch_pending_messages(agent_id, workspace_id).await?;
        let pulse = self.fetch_pulse(workspace_id).await;

        Ok(WaitEventsResponse {
            messages,
            pulse,
            timed_out,
        })
    }

    async fn fetch_pending_messages(
        &self,
        agent_id: &AgentId,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<PendingMessage>> {
        let aid = agent_id.to_string();
        let ws_id = workspace_id.to_string();

        let messages = self
            .conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT message_id, from_agent_id, kind, payload, correlation_id, created_at \
                     FROM agent_messages \
                     WHERE workspace_id = ?1 \
                       AND (to_agent_id = ?2 OR to_agent_id IS NULL) \
                       AND delivered_at IS NULL \
                     ORDER BY created_at ASC \
                     LIMIT 20",
                )?;

                let rows = stmt.query_map(rusqlite::params![ws_id, aid], |row| {
                    let payload_str: String = row.get(3)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        payload_str,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })?;

                let mut messages = Vec::new();
                for row in rows {
                    let (message_id, from_id, kind, payload_str, correlation_id, created_at) = row?;
                    let payload: serde_json::Value =
                        serde_json::from_str(&payload_str).unwrap_or_default();
                    messages.push((
                        message_id,
                        from_id,
                        kind,
                        payload,
                        correlation_id,
                        created_at,
                    ));
                }
                Ok(messages)
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch messages: {e}"))?;

        // Mark messages as delivered
        if !messages.is_empty() {
            let now = Utc::now().to_rfc3339();
            let ids: Vec<String> = messages.iter().map(|(id, ..)| id.clone()).collect();
            let now_clone = now.clone();
            self.conn
                .call(move |conn| {
                    for id in &ids {
                        conn.execute(
                            "UPDATE agent_messages SET delivered_at = ?1 WHERE message_id = ?2",
                            rusqlite::params![now_clone, id],
                        )?;
                    }
                    Ok(())
                })
                .await
                .map_err(|e| anyhow::anyhow!("Failed to mark messages delivered: {e}"))?;
        }

        Ok(messages
            .into_iter()
            .map(
                |(message_id, from_agent_id, kind, payload, correlation_id, created_at)| {
                    PendingMessage {
                        message_id,
                        from_agent_id,
                        kind,
                        payload,
                        correlation_id,
                        created_at,
                    }
                },
            )
            .collect())
    }

    async fn fetch_pulse(&self, workspace_id: &WorkspaceId) -> EventPulse {
        let ws_id = workspace_id.to_string();
        let ws_id2 = workspace_id.to_string();

        let tasks_at_gates = self
            .conn
            .call(move |conn| {
                let count = conn.query_row(
                    "SELECT count(*) FROM tasks WHERE workspace_id = ?1 \
                     AND status IN ('spec_review', 'awaiting_approval', 'verification', 'code_review')",
                    [ws_id],
                    |row| row.get::<_, i64>(0),
                ).map(|c| c as usize).unwrap_or(0);
                Ok(count)
            })
            .await
            .unwrap_or(0);

        let agents_online = self
            .agent_repo
            .list_active(workspace_id)
            .await
            .map(|v| v.len())
            .unwrap_or(0);

        let pending_reviews = self
            .conn
            .call(move |conn| {
                let count: usize = conn
                    .query_row(
                        "SELECT count(*) FROM agent_messages \
                         WHERE workspace_id = ?1 AND kind = 'review_request' \
                         AND delivered_at IS NULL",
                        [ws_id2],
                        |row| row.get::<_, i64>(0),
                    )
                    .map(|c| c as usize)
                    .unwrap_or(0);
                Ok(count)
            })
            .await
            .unwrap_or(0);

        EventPulse {
            tasks_at_gates,
            agents_online,
            pending_reviews,
        }
    }
}

// ---------------------------------------------------------------------------
// Unit Tests (TDD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ports::MockActivityRepository;
    use crate::engine::ports::MockAgentRepository;
    use crate::orchestration::bus::WorkspaceEventBus;
    use crate::types::{AgentId, WorkspaceId};

    #[tokio::test]
    async fn test_send_message_stores_and_returns_id() {
        let db = crate::db::Database::open_in_memory().await.unwrap();
        let bus = WorkspaceEventBus::new(16);

        let mut mock_activity = MockActivityRepository::new();
        mock_activity.expect_log_activity().returning(|_| Ok(()));

        let uc = SendMessageUseCase::new(db.conn().clone(), Arc::new(mock_activity), bus);

        let req = SendMessageRequest {
            workspace_id: WorkspaceId::from("ws-1"),
            from_agent_id: AgentId::from("agent-pm"),
            to_agent_id: Some(AgentId::from("agent-dev")),
            role_filter: None,
            kind: "info".to_string(),
            payload: serde_json::json!({"text": "Please start task T1"}),
            correlation_id: None,
            requires_response: false,
        };

        let res = uc.execute(req).await.unwrap();
        assert!(!res.message_id.is_empty());
        assert_eq!(res.delivered_to, 1);
    }

    #[tokio::test]
    async fn test_wait_events_returns_pending_messages() {
        let db = crate::db::Database::open_in_memory().await.unwrap();
        let bus = WorkspaceEventBus::new(16);

        let mut mock_activity = MockActivityRepository::new();
        mock_activity.expect_log_activity().returning(|_| Ok(()));

        let mut mock_agent_repo = MockAgentRepository::new();
        mock_agent_repo
            .expect_list_active()
            .returning(|_| Ok(Vec::new()));

        // First send a message
        let send_uc =
            SendMessageUseCase::new(db.conn().clone(), Arc::new(mock_activity), bus.clone());
        send_uc
            .execute(SendMessageRequest {
                workspace_id: WorkspaceId::from("ws-1"),
                from_agent_id: AgentId::from("agent-pm"),
                to_agent_id: Some(AgentId::from("agent-dev")),
                role_filter: None,
                kind: "review_request".to_string(),
                payload: serde_json::json!({"task_id": "t1"}),
                correlation_id: Some("review-123".to_string()),
                requires_response: true,
            })
            .await
            .unwrap();

        // Then wait — should return immediately
        let wait_uc = WaitEventsUseCase::new(db.conn().clone(), bus, Arc::new(mock_agent_repo));
        let res = wait_uc
            .execute(&AgentId::from("agent-dev"), &WorkspaceId::from("ws-1"))
            .await
            .unwrap();

        assert!(!res.timed_out);
        assert_eq!(res.messages.len(), 1);
        assert_eq!(res.messages[0].kind, "review_request");
        assert_eq!(
            res.messages[0].correlation_id.as_deref(),
            Some("review-123")
        );
    }
}
