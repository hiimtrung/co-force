//! SQLite implementation of `ActivityRepository`.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio_rusqlite::Connection;

use crate::engine::ports::ActivityRepository;
use crate::types::{ActivityId, ActivityType, AgentActivity, AgentId, TaskId, WorkspaceId};

/// Concrete SQLite-backed activity repository.
#[derive(Clone)]
pub struct SqliteActivityRepo {
    conn: Connection,
}

impl SqliteActivityRepo {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl ActivityRepository for SqliteActivityRepo {
    async fn log_activity(&self, activity: &AgentActivity) -> Result<()> {
        let activity = activity.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO agent_activities \
                     (activity_id, workspace_id, agent_id, activity_type, content, \
                      related_task_id, related_files, version, occurred_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        activity.activity_id.as_ref(),
                        activity.workspace_id.as_ref(),
                        activity.agent_id.as_ref(),
                        activity.activity_type.to_string(),
                        activity.content.as_ref().map(|v| v.to_string()),
                        activity
                            .related_task_id
                            .as_ref()
                            .map(|t| t.as_ref().to_string()),
                        activity
                            .related_files
                            .as_ref()
                            .map(|f| serde_json::to_string(f).unwrap_or_default()),
                        activity.version,
                        activity.occurred_at.to_rfc3339(),
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("log_activity failed: {e}"))
    }

    async fn get_workspace_stream(
        &self,
        workspace_id: &WorkspaceId,
        limit: usize,
    ) -> Result<Vec<AgentActivity>> {
        let ws_id = workspace_id.clone();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT activity_id, workspace_id, agent_id, activity_type, content, \
                     related_task_id, related_files, version, occurred_at \
                     FROM agent_activities \
                     WHERE workspace_id = ?1 \
                     ORDER BY occurred_at DESC \
                     LIMIT ?2",
                )?;

                let activities = stmt
                    .query_map(
                        rusqlite::params![ws_id.as_ref(), limit as i64],
                        row_to_activity,
                    )?
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(activities)
            })
            .await
            .map_err(|e| anyhow::anyhow!("get_workspace_stream failed: {e}"))
    }

    async fn get_agent_activities(
        &self,
        agent_id: &AgentId,
        limit: usize,
    ) -> Result<Vec<AgentActivity>> {
        let agent_id = agent_id.clone();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT activity_id, workspace_id, agent_id, activity_type, content, \
                     related_task_id, related_files, version, occurred_at \
                     FROM agent_activities \
                     WHERE agent_id = ?1 \
                     ORDER BY occurred_at DESC \
                     LIMIT ?2",
                )?;

                let activities = stmt
                    .query_map(
                        rusqlite::params![agent_id.as_ref(), limit as i64],
                        row_to_activity,
                    )?
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(activities)
            })
            .await
            .map_err(|e| anyhow::anyhow!("get_agent_activities failed: {e}"))
    }
}

/// Helper to convert a rusqlite row into an `AgentActivity`.
fn row_to_activity(row: &rusqlite::Row<'_>) -> Result<AgentActivity, rusqlite::Error> {
    let occurred_at_str: String = row.get(8)?;
    let occurred_at = DateTime::parse_from_rfc3339(&occurred_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    let related_files: Option<Vec<String>> = row
        .get::<_, Option<String>>(6)?
        .and_then(|s| serde_json::from_str(&s).ok());

    Ok(AgentActivity {
        activity_id: ActivityId::from(row.get::<_, String>(0)?),
        workspace_id: WorkspaceId::from(row.get::<_, String>(1)?),
        agent_id: AgentId::from(row.get::<_, String>(2)?),
        activity_type: ActivityType::from_str_value(&row.get::<_, String>(3)?)
            .unwrap_or(ActivityType::CheckIn),
        content: row
            .get::<_, Option<String>>(4)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        related_task_id: row.get::<_, Option<String>>(5)?.map(TaskId::from),
        related_files,
        version: row.get(7)?,
        occurred_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::agent_repo::SqliteAgentRepo;
    use crate::db::Database;
    use crate::engine::ports::AgentRepository;
    use crate::types::AgentState;

    async fn setup() -> (Database, SqliteActivityRepo) {
        let db = Database::open_in_memory().await.unwrap();
        let repo = SqliteActivityRepo::new(db.conn().clone());
        (db, repo)
    }

    async fn ensure_agent(db: &Database, agent_id: &str, ws: &str) {
        let agent_repo = SqliteAgentRepo::new(db.conn().clone());
        agent_repo
            .upsert(&crate::types::Agent {
                agent_id: AgentId::from(agent_id),
                workspace_id: WorkspaceId::from(ws),
                name: format!("Agent-{agent_id}"),
                role: "developer".to_string(),
                provider: None,
                machine_id: "test-machine".to_string(),
                state: AgentState::Idle,
                current_task_id: None,
                last_seen: None,
                created_at: None,
            })
            .await
            .unwrap();
    }

    fn sample_activity(id: &str, ws: &str, agent: &str) -> AgentActivity {
        AgentActivity {
            activity_id: ActivityId::from(id),
            workspace_id: WorkspaceId::from(ws),
            agent_id: AgentId::from(agent),
            activity_type: ActivityType::CheckIn,
            content: Some(serde_json::json!({"summary": "Agent checked in"})),
            related_task_id: None,
            related_files: Some(vec!["src/main.rs".to_string()]),
            version: 1,
            occurred_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_log_and_retrieve_activity() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "agent-1", "ws-1").await;

        repo.log_activity(&sample_activity("act-1", "ws-1", "agent-1"))
            .await
            .unwrap();

        let stream = repo
            .get_workspace_stream(&WorkspaceId::from("ws-1"), 10)
            .await
            .unwrap();

        assert_eq!(stream.len(), 1);
        assert_eq!(stream[0].activity_id, ActivityId::from("act-1"));
        assert_eq!(stream[0].activity_type, ActivityType::CheckIn);
    }

    #[tokio::test]
    async fn test_workspace_stream_ordering_and_limit() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "agent-1", "ws-1").await;

        for i in 0..5 {
            let mut activity = sample_activity(&format!("act-{i}"), "ws-1", "agent-1");
            activity.activity_type = ActivityType::TaskStarted;
            activity.occurred_at = Utc::now() + chrono::Duration::milliseconds(i * 100);
            repo.log_activity(&activity).await.unwrap();
        }

        let stream = repo
            .get_workspace_stream(&WorkspaceId::from("ws-1"), 3)
            .await
            .unwrap();

        assert_eq!(stream.len(), 3);
        assert!(stream[0].occurred_at >= stream[1].occurred_at);
    }

    #[tokio::test]
    async fn test_get_agent_activities() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "agent-a", "ws-1").await;
        ensure_agent(&db, "agent-b", "ws-1").await;

        repo.log_activity(&sample_activity("act-1", "ws-1", "agent-a"))
            .await
            .unwrap();
        repo.log_activity(&sample_activity("act-2", "ws-1", "agent-b"))
            .await
            .unwrap();
        repo.log_activity(&sample_activity("act-3", "ws-1", "agent-a"))
            .await
            .unwrap();

        let agent_a_acts = repo
            .get_agent_activities(&AgentId::from("agent-a"), 10)
            .await
            .unwrap();

        assert_eq!(agent_a_acts.len(), 2);
        assert!(agent_a_acts
            .iter()
            .all(|a| a.agent_id == AgentId::from("agent-a")));
    }

    #[tokio::test]
    async fn test_workspace_isolation() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "agent-1", "ws-1").await;
        ensure_agent(&db, "agent-2", "ws-2").await;

        repo.log_activity(&sample_activity("act-1", "ws-1", "agent-1"))
            .await
            .unwrap();
        repo.log_activity(&sample_activity("act-2", "ws-2", "agent-2"))
            .await
            .unwrap();

        let ws1 = repo
            .get_workspace_stream(&WorkspaceId::from("ws-1"), 10)
            .await
            .unwrap();
        assert_eq!(ws1.len(), 1);
        assert_eq!(ws1[0].workspace_id, WorkspaceId::from("ws-1"));
    }
}
