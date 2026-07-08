//! SQLite implementation of `AgentRepository`.

use anyhow::Result;
use async_trait::async_trait;
use tokio_rusqlite::Connection;

use crate::db::helpers::get_optional_datetime;
use crate::engine::ports::AgentRepository;
use crate::types::{Agent, AgentId, AgentState, TaskId, WorkspaceId};

/// Concrete SQLite-backed agent repository.
#[derive(Clone)]
pub struct SqliteAgentRepo {
    conn: Connection,
}

impl SqliteAgentRepo {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }
}

/// Helper to build an `Agent` from a rusqlite row.
fn row_to_agent(row: &rusqlite::Row<'_>) -> Result<Agent, rusqlite::Error> {
    Ok(Agent {
        agent_id: AgentId::from(row.get::<_, String>(0)?),
        workspace_id: WorkspaceId::from(row.get::<_, String>(1)?),
        name: row.get(2)?,
        role: row.get(3)?,
        provider: row.get(4)?,
        machine_id: row.get(5)?,
        state: AgentState::from_str_value(&row.get::<_, String>(6)?).unwrap_or(AgentState::Idle),
        current_task_id: row.get::<_, Option<String>>(7)?.map(TaskId::from),
        last_seen: get_optional_datetime(row, 8).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
        })?,
        created_at: get_optional_datetime(row, 9).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(e))
        })?,
    })
}

const SELECT_AGENT_COLS: &str = "SELECT agent_id, workspace_id, name, role, provider, machine_id, \
     state, current_task_id, last_seen, created_at FROM agents";

#[async_trait]
impl AgentRepository for SqliteAgentRepo {
    async fn find_by_id(&self, id: &AgentId) -> Result<Option<Agent>> {
        let id = id.clone();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!("{SELECT_AGENT_COLS} WHERE agent_id = ?1"))?;

                let agent = stmt
                    .query_row([id.as_ref()], row_to_agent)
                    .optional()?;

                Ok(agent)
            })
            .await
            .map_err(|e| anyhow::anyhow!("find_by_id failed: {e}"))
    }

    async fn upsert(&self, agent: &Agent) -> Result<()> {
        let agent = agent.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO agents (agent_id, workspace_id, name, role, provider, \
                     machine_id, state, current_task_id, last_seen) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, CURRENT_TIMESTAMP) \
                     ON CONFLICT(agent_id) DO UPDATE SET \
                     name = excluded.name, \
                     role = excluded.role, \
                     provider = excluded.provider, \
                     machine_id = excluded.machine_id, \
                     state = excluded.state, \
                     current_task_id = excluded.current_task_id, \
                     last_seen = CURRENT_TIMESTAMP",
                    rusqlite::params![
                        agent.agent_id.as_ref(),
                        agent.workspace_id.as_ref(),
                        agent.name,
                        agent.role,
                        agent.provider,
                        agent.machine_id,
                        agent.state.to_string(),
                        agent
                            .current_task_id
                            .as_ref()
                            .map(|t| t.as_ref().to_string()),
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("upsert failed: {e}"))
    }

    async fn list_active(&self, workspace_id: &WorkspaceId) -> Result<Vec<Agent>> {
        let ws_id = workspace_id.clone();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "{SELECT_AGENT_COLS} WHERE workspace_id = ?1 AND state != 'disconnected'"
                ))?;

                let agents = stmt
                    .query_map([ws_id.as_ref()], row_to_agent)?
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(agents)
            })
            .await
            .map_err(|e| anyhow::anyhow!("list_active failed: {e}"))
    }

    async fn list_all(&self, workspace_id: &WorkspaceId) -> Result<Vec<Agent>> {
        let ws_id = workspace_id.clone();
        self.conn
            .call(move |conn| {
                let mut stmt =
                    conn.prepare(&format!("{SELECT_AGENT_COLS} WHERE workspace_id = ?1"))?;

                let agents = stmt
                    .query_map([ws_id.as_ref()], row_to_agent)?
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(agents)
            })
            .await
            .map_err(|e| anyhow::anyhow!("list_all failed: {e}"))
    }
}

/// Extension trait for rusqlite results to handle optional (not found) rows.
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn setup() -> (Database, SqliteAgentRepo) {
        let db = Database::open_in_memory().await.unwrap();
        let repo = SqliteAgentRepo::new(db.conn().clone());
        (db, repo)
    }

    fn sample_agent(id: &str, ws: &str) -> Agent {
        Agent {
            agent_id: AgentId::from(id),
            workspace_id: WorkspaceId::from(ws),
            name: format!("Agent-{id}"),
            role: "developer".to_string(),
            provider: Some("claude".to_string()),
            machine_id: "machine-1".to_string(),
            state: AgentState::Idle,
            current_task_id: None,
            last_seen: None,
            created_at: None,
        }
    }

    #[tokio::test]
    async fn test_upsert_and_find_by_id() {
        let (_db, repo) = setup().await;
        let agent = sample_agent("a-1", "ws-1");

        repo.upsert(&agent).await.unwrap();
        let found = repo.find_by_id(&AgentId::from("a-1")).await.unwrap();

        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.agent_id, AgentId::from("a-1"));
        assert_eq!(found.name, "Agent-a-1");
        assert_eq!(found.state, AgentState::Idle);
        // last_seen should be populated by the DB
        assert!(found.last_seen.is_some());
    }

    #[tokio::test]
    async fn test_find_by_id_not_found() {
        let (_db, repo) = setup().await;
        let found = repo
            .find_by_id(&AgentId::from("nonexistent"))
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_upsert_updates_existing() {
        let (_db, repo) = setup().await;
        let mut agent = sample_agent("a-1", "ws-1");

        repo.upsert(&agent).await.unwrap();

        agent.state = AgentState::Working;
        agent.current_task_id = Some(TaskId::from("task-1"));
        repo.upsert(&agent).await.unwrap();

        let found = repo
            .find_by_id(&AgentId::from("a-1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.state, AgentState::Working);
        assert_eq!(found.current_task_id, Some(TaskId::from("task-1")));
    }

    #[tokio::test]
    async fn test_list_active_excludes_disconnected() {
        let (_db, repo) = setup().await;

        let mut a1 = sample_agent("a-1", "ws-1");
        a1.state = AgentState::Idle;
        repo.upsert(&a1).await.unwrap();

        let mut a2 = sample_agent("a-2", "ws-1");
        a2.state = AgentState::Disconnected;
        repo.upsert(&a2).await.unwrap();

        let mut a3 = sample_agent("a-3", "ws-1");
        a3.state = AgentState::Working;
        repo.upsert(&a3).await.unwrap();

        let active = repo.list_active(&WorkspaceId::from("ws-1")).await.unwrap();
        assert_eq!(active.len(), 2);
        let ids: Vec<_> = active.iter().map(|a| a.agent_id.as_ref()).collect();
        assert!(ids.contains(&"a-1"));
        assert!(ids.contains(&"a-3"));
        assert!(!ids.contains(&"a-2"));
    }

    #[tokio::test]
    async fn test_list_all_includes_disconnected() {
        let (_db, repo) = setup().await;

        let mut a1 = sample_agent("a-1", "ws-1");
        a1.state = AgentState::Idle;
        repo.upsert(&a1).await.unwrap();

        let mut a2 = sample_agent("a-2", "ws-1");
        a2.state = AgentState::Disconnected;
        repo.upsert(&a2).await.unwrap();

        let all = repo.list_all(&WorkspaceId::from("ws-1")).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_list_active_workspace_isolation() {
        let (_db, repo) = setup().await;

        repo.upsert(&sample_agent("a-1", "ws-1")).await.unwrap();
        repo.upsert(&sample_agent("a-2", "ws-2")).await.unwrap();

        let ws1_agents = repo.list_active(&WorkspaceId::from("ws-1")).await.unwrap();
        assert_eq!(ws1_agents.len(), 1);
        assert_eq!(ws1_agents[0].agent_id, AgentId::from("a-1"));
    }
}
