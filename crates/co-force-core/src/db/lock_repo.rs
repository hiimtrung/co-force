//! SQLite implementation of `LockRepository`.

use anyhow::Result;
use async_trait::async_trait;
use tokio_rusqlite::Connection;

use crate::db::helpers::get_optional_datetime;
use crate::engine::ports::LockRepository;
use crate::types::{AgentId, FileLock, TaskId, WorkspaceId};

/// Concrete SQLite-backed lock repository.
#[derive(Clone)]
pub struct SqliteLockRepo {
    conn: Connection,
}

impl SqliteLockRepo {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }
}

/// Helper to build a `FileLock` from a rusqlite row.
fn row_to_lock(row: &rusqlite::Row<'_>) -> Result<FileLock, rusqlite::Error> {
    Ok(FileLock {
        id: Some(row.get(0)?),
        workspace_id: WorkspaceId::from(row.get::<_, String>(1)?),
        file_path: row.get(2)?,
        agent_id: AgentId::from(row.get::<_, String>(3)?),
        machine_id: row.get(4)?,
        task_id: row.get::<_, Option<String>>(5)?.map(TaskId::from),
        reason: row.get(6)?,
        locked_at: get_optional_datetime(row, 7).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e))
        })?,
        expires_at: get_optional_datetime(row, 8).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
        })?,
    })
}

#[async_trait]
impl LockRepository for SqliteLockRepo {
    async fn acquire_locks(&self, locks: &[FileLock]) -> Result<()> {
        let locks = locks.to_vec();
        self.conn
            .call(move |conn| {
                let tx = conn.transaction()?;
                for lock in &locks {
                    tx.execute(
                        "INSERT INTO file_locks (workspace_id, file_path, agent_id, machine_id, task_id, reason, expires_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        rusqlite::params![
                            lock.workspace_id.as_ref(),
                            lock.file_path,
                            lock.agent_id.as_ref(),
                            lock.machine_id,
                            lock.task_id.as_ref().map(|t| t.as_ref().to_string()),
                            lock.reason,
                            lock.expires_at.map(|dt| dt.to_rfc3339()),
                        ],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("acquire_locks failed: {e}"))
    }

    async fn release_locks(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        paths: &[String],
    ) -> Result<()> {
        let ws_id = workspace_id.clone();
        let agent_id = agent_id.clone();
        let paths = paths.to_vec();
        self.conn
            .call(move |conn| {
                let tx = conn.transaction()?;
                for path in &paths {
                    tx.execute(
                        "DELETE FROM file_locks WHERE workspace_id = ?1 AND agent_id = ?2 AND file_path = ?3",
                        rusqlite::params![ws_id.as_ref(), agent_id.as_ref(), path],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("release_locks failed: {e}"))
    }

    async fn list_locks(&self, workspace_id: &WorkspaceId) -> Result<Vec<FileLock>> {
        let ws_id = workspace_id.clone();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, workspace_id, file_path, agent_id, machine_id, task_id, reason, locked_at, expires_at \
                     FROM file_locks WHERE workspace_id = ?1"
                )?;
                let locks = stmt
                    .query_map([ws_id.as_ref()], row_to_lock)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(locks)
            })
            .await
            .map_err(|e| anyhow::anyhow!("list_locks failed: {e}"))
    }

    async fn release_all_for_agent(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Result<()> {
        let ws_id = workspace_id.clone();
        let agent_id = agent_id.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM file_locks WHERE workspace_id = ?1 AND agent_id = ?2",
                    rusqlite::params![ws_id.as_ref(), agent_id.as_ref()],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("release_all_for_agent failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::agent_repo::SqliteAgentRepo;
    use crate::db::Database;
    use crate::engine::ports::AgentRepository;
    use crate::types::AgentState;

    async fn setup() -> (Database, SqliteLockRepo) {
        let db = Database::open_in_memory().await.unwrap();
        let repo = SqliteLockRepo::new(db.conn().clone());
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

    fn sample_lock(ws: &str, file: &str, agent: &str) -> FileLock {
        FileLock {
            id: None,
            workspace_id: WorkspaceId::from(ws),
            file_path: file.to_string(),
            agent_id: AgentId::from(agent),
            machine_id: "machine-1".to_string(),
            task_id: None,
            reason: Some("editing".to_string()),
            locked_at: None,
            expires_at: None,
        }
    }

    #[tokio::test]
    async fn test_acquire_and_list_locks() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "agent-1", "ws-1").await;
        let lock = sample_lock("ws-1", "src/main.rs", "agent-1");

        repo.acquire_locks(&[lock]).await.unwrap();
        let locks = repo.list_locks(&WorkspaceId::from("ws-1")).await.unwrap();

        assert_eq!(locks.len(), 1);
        assert_eq!(locks[0].file_path, "src/main.rs");
    }

    #[tokio::test]
    async fn test_acquire_conflict() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "agent-1", "ws-1").await;
        ensure_agent(&db, "agent-2", "ws-1").await;

        let lock1 = sample_lock("ws-1", "src/main.rs", "agent-1");
        let lock2 = sample_lock("ws-1", "src/main.rs", "agent-2");

        repo.acquire_locks(&[lock1]).await.unwrap();
        let result = repo.acquire_locks(&[lock2]).await;
        assert!(result.is_err()); // unique constraint violation
    }

    #[tokio::test]
    async fn test_release_locks() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "agent-1", "ws-1").await;
        let lock = sample_lock("ws-1", "src/main.rs", "agent-1");

        repo.acquire_locks(&[lock]).await.unwrap();
        repo.release_locks(
            &WorkspaceId::from("ws-1"),
            &AgentId::from("agent-1"),
            &["src/main.rs".to_string()],
        )
        .await
        .unwrap();

        let locks = repo.list_locks(&WorkspaceId::from("ws-1")).await.unwrap();
        assert_eq!(locks.len(), 0);
    }
}
