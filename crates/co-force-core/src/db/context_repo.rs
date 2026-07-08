//! SQLite implementation of `ContextRepository`.

use anyhow::Result;
use async_trait::async_trait;
use tokio_rusqlite::Connection;

use crate::db::helpers::get_optional_datetime;
use crate::engine::ports::ContextRepository;
use crate::types::{AgentId, ContextId, SharedContext, WorkspaceId};

/// Concrete SQLite-backed context repository.
#[derive(Clone)]
pub struct SqliteContextRepo {
    conn: Connection,
}

impl SqliteContextRepo {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl ContextRepository for SqliteContextRepo {
    async fn share_context(&self, ctx: &SharedContext) -> Result<()> {
        let ctx = ctx.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO shared_contexts \
                     (context_id, workspace_id, source_agent_id, target_agent_id, \
                      context_type, content, resolved) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        ctx.context_id.as_ref(),
                        ctx.workspace_id.as_ref(),
                        ctx.source_agent_id.as_ref(),
                        ctx.target_agent_id.as_ref().map(|a| a.as_ref().to_string()),
                        ctx.context_type,
                        ctx.content.to_string(),
                        ctx.resolved,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("share_context failed: {e}"))
    }

    async fn get_unresolved(&self, target_agent: &AgentId) -> Result<Vec<SharedContext>> {
        let target = target_agent.clone();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT context_id, workspace_id, source_agent_id, target_agent_id, \
                     context_type, content, resolved, created_at, resolved_at \
                     FROM shared_contexts \
                     WHERE (target_agent_id = ?1 OR target_agent_id IS NULL) \
                     AND resolved = FALSE \
                     ORDER BY created_at ASC",
                )?;

                let contexts = stmt
                    .query_map([target.as_ref()], row_to_context)?
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(contexts)
            })
            .await
            .map_err(|e| anyhow::anyhow!("get_unresolved failed: {e}"))
    }

    async fn mark_resolved(&self, context_id: &ContextId) -> Result<()> {
        let ctx_id = context_id.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "UPDATE shared_contexts \
                     SET resolved = TRUE, resolved_at = CURRENT_TIMESTAMP \
                     WHERE context_id = ?1",
                    [ctx_id.as_ref()],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("mark_resolved failed: {e}"))
    }
}

fn row_to_context(row: &rusqlite::Row<'_>) -> Result<SharedContext, rusqlite::Error> {
    Ok(SharedContext {
        context_id: ContextId::from(row.get::<_, String>(0)?),
        workspace_id: WorkspaceId::from(row.get::<_, String>(1)?),
        source_agent_id: AgentId::from(row.get::<_, String>(2)?),
        target_agent_id: row.get::<_, Option<String>>(3)?.map(AgentId::from),
        context_type: row.get(4)?,
        content: row
            .get::<_, String>(5)
            .map(|s| serde_json::from_str(&s).unwrap_or(serde_json::Value::Null))?,
        resolved: row.get(6)?,
        created_at: get_optional_datetime(row, 7).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e))
        })?,
        resolved_at: get_optional_datetime(row, 8).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
        })?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::agent_repo::SqliteAgentRepo;
    use crate::db::Database;
    use crate::engine::ports::AgentRepository;
    use crate::types::AgentState;

    async fn setup() -> (Database, SqliteContextRepo) {
        let db = Database::open_in_memory().await.unwrap();
        let repo = SqliteContextRepo::new(db.conn().clone());
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

    fn sample_context(id: &str, source: &str, target: Option<&str>) -> SharedContext {
        SharedContext {
            context_id: ContextId::from(id),
            workspace_id: WorkspaceId::from("ws-1"),
            source_agent_id: AgentId::from(source),
            target_agent_id: target.map(AgentId::from),
            context_type: "handover".to_string(),
            content: serde_json::json!({"notes": "context data"}),
            resolved: false,
            created_at: None,
            resolved_at: None,
        }
    }

    #[tokio::test]
    async fn test_share_and_retrieve_context() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "agent-a", "ws-1").await;

        let ctx = sample_context("ctx-1", "agent-a", Some("agent-b"));
        repo.share_context(&ctx).await.unwrap();

        let unresolved = repo
            .get_unresolved(&AgentId::from("agent-b"))
            .await
            .unwrap();

        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].context_id, ContextId::from("ctx-1"));
        assert_eq!(unresolved[0].context_type, "handover");
        assert!(!unresolved[0].resolved);
    }

    #[tokio::test]
    async fn test_mark_resolved() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "agent-a", "ws-1").await;

        let ctx = sample_context("ctx-1", "agent-a", Some("agent-b"));
        repo.share_context(&ctx).await.unwrap();
        repo.mark_resolved(&ContextId::from("ctx-1")).await.unwrap();

        let unresolved = repo
            .get_unresolved(&AgentId::from("agent-b"))
            .await
            .unwrap();

        assert_eq!(unresolved.len(), 0);
    }

    #[tokio::test]
    async fn test_broadcast_context_included_for_any_agent() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "agent-a", "ws-1").await;

        let ctx = sample_context("ctx-broadcast", "agent-a", None);
        repo.share_context(&ctx).await.unwrap();

        let unresolved = repo
            .get_unresolved(&AgentId::from("agent-x"))
            .await
            .unwrap();

        assert_eq!(unresolved.len(), 1);
    }

    #[tokio::test]
    async fn test_targeted_context_not_visible_to_other_agents() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "agent-a", "ws-1").await;

        let ctx = sample_context("ctx-1", "agent-a", Some("agent-b"));
        repo.share_context(&ctx).await.unwrap();

        let unresolved = repo
            .get_unresolved(&AgentId::from("agent-c"))
            .await
            .unwrap();

        assert_eq!(unresolved.len(), 0);
    }

    #[tokio::test]
    async fn test_multiple_contexts_ordering() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "agent-a", "ws-1").await;

        for i in 0..3 {
            let ctx = sample_context(&format!("ctx-{i}"), "agent-a", Some("agent-b"));
            repo.share_context(&ctx).await.unwrap();
        }

        let unresolved = repo
            .get_unresolved(&AgentId::from("agent-b"))
            .await
            .unwrap();

        assert_eq!(unresolved.len(), 3);
    }
}
