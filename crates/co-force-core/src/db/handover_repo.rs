//! SQLite implementation of `HandoverRepository` and `ProviderStatusRepository`.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio_rusqlite::Connection;

use crate::db::helpers::get_optional_datetime;
use crate::engine::ports::{HandoverRepository, ProviderStatusRepository};
use crate::types::{AgentId, Handover, TaskId};

/// Concrete SQLite-backed handover repository.
#[derive(Clone)]
pub struct SqliteHandoverRepo {
    conn: Connection,
}

impl SqliteHandoverRepo {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }
}

/// Helper to build a `Handover` from a rusqlite row.
fn row_to_handover(row: &rusqlite::Row<'_>) -> Result<Handover, rusqlite::Error> {
    let package_str: String = row.get(5)?;
    let package: serde_json::Value = serde_json::from_str(&package_str).unwrap_or_default();

    Ok(Handover {
        handover_id: row.get(0)?,
        task_id: TaskId::from(row.get::<_, String>(1)?),
        from_agent_id: AgentId::from(row.get::<_, String>(2)?),
        to_agent_id: row.get::<_, Option<String>>(3)?.map(AgentId::from),
        reason: row.get(4)?,
        package,
        provider_cooldown_until: get_optional_datetime(row, 6).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
        })?,
        created_at: get_optional_datetime(row, 7).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e))
        })?,
        accepted_at: get_optional_datetime(row, 8).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
        })?,
    })
}

const SELECT_HANDOVER_COLS: &str = "SELECT handover_id, task_id, from_agent_id, to_agent_id, reason, \
     package, provider_cooldown_until, created_at, accepted_at FROM handovers";

#[async_trait]
impl HandoverRepository for SqliteHandoverRepo {
    async fn insert_handover(&self, handover: &Handover) -> Result<()> {
        let h = handover.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO handovers (handover_id, task_id, from_agent_id, to_agent_id, reason, \
                     package, provider_cooldown_until, created_at, accepted_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, COALESCE(?7, CURRENT_TIMESTAMP), CURRENT_TIMESTAMP, ?8)",
                    rusqlite::params![
                        h.handover_id,
                        h.task_id.as_ref(),
                        h.from_agent_id.as_ref(),
                        h.to_agent_id.as_ref().map(|a| a.as_ref().to_string()),
                        h.reason,
                        h.package.to_string(),
                        h.provider_cooldown_until.map(|dt| dt.to_rfc3339()),
                        h.accepted_at.map(|dt| dt.to_rfc3339()),
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("insert_handover failed: {e}"))
    }

    async fn find_handover(&self, handover_id: &str) -> Result<Option<Handover>> {
        let hid = handover_id.to_string();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!("{SELECT_HANDOVER_COLS} WHERE handover_id = ?1"))?;
                let handover = stmt.query_row([hid], row_to_handover).optional()?;
                Ok(handover)
            })
            .await
            .map_err(|e| anyhow::anyhow!("find_handover failed: {e}"))
    }

    async fn find_pending_for_task(&self, task_id: &TaskId) -> Result<Option<Handover>> {
        let tid = task_id.clone();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "{SELECT_HANDOVER_COLS} WHERE task_id = ?1 AND accepted_at IS NULL"
                ))?;
                let handover = stmt.query_row([tid.as_ref()], row_to_handover).optional()?;
                Ok(handover)
            })
            .await
            .map_err(|e| anyhow::anyhow!("find_pending_for_task failed: {e}"))
    }

    async fn update_handover(&self, handover: &Handover) -> Result<()> {
        let h = handover.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "UPDATE handovers SET \
                     to_agent_id = ?2, \
                     accepted_at = ?3 \
                     WHERE handover_id = ?1",
                    rusqlite::params![
                        h.handover_id,
                        h.to_agent_id.as_ref().map(|a| a.as_ref().to_string()),
                        h.accepted_at.map(|dt| dt.to_rfc3339()),
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("update_handover failed: {e}"))
    }
}

/// Concrete SQLite-backed provider status repository.
#[derive(Clone)]
pub struct SqliteProviderStatusRepo {
    conn: Connection,
}

impl SqliteProviderStatusRepo {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl ProviderStatusRepository for SqliteProviderStatusRepo {
    async fn set_cooldown(
        &self,
        machine_id: &str,
        provider: &str,
        until: DateTime<Utc>,
        error: Option<String>,
    ) -> Result<()> {
        let mid = machine_id.to_string();
        let prov = provider.to_string();
        let err = error.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO provider_status (machine_id, provider, rate_limited_until, last_error) \
                     VALUES (?1, ?2, ?3, ?4) \
                     ON CONFLICT(machine_id, provider) DO UPDATE SET \
                     rate_limited_until = excluded.rate_limited_until, \
                     last_error = excluded.last_error",
                    rusqlite::params![mid, prov, until.to_rfc3339(), err],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("set_cooldown failed: {e}"))
    }

    async fn get_cooldown(
        &self,
        machine_id: &str,
        provider: &str,
    ) -> Result<Option<DateTime<Utc>>> {
        let mid = machine_id.to_string();
        let prov = provider.to_string();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT rate_limited_until FROM provider_status WHERE machine_id = ?1 AND provider = ?2"
                )?;
                let res: Option<String> = stmt.query_row(rusqlite::params![mid, prov], |row| row.get(0)).optional()?;
                let dt = res.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc)));
                Ok(dt)
            })
            .await
            .map_err(|e| anyhow::anyhow!("get_cooldown failed: {e}"))
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

    #[tokio::test]
    async fn test_handover_repo() {
        use crate::db::agent_repo::SqliteAgentRepo;
        use crate::db::task_repo::SqliteTaskRepo;
        use crate::engine::ports::{AgentRepository, TaskRepository};
        use crate::types::{Agent, AgentState, Task, TaskStatus, WorkspaceId};

        let db = Database::open_in_memory().await.unwrap();
        let repo = SqliteHandoverRepo::new(db.conn().clone());
        let agent_repo = SqliteAgentRepo::new(db.conn().clone());
        let task_repo = SqliteTaskRepo::new(db.conn().clone());

        agent_repo.upsert(&Agent {
            agent_id: AgentId::from("a-1"),
            workspace_id: WorkspaceId::from("ws-1"),
            name: "Agent 1".to_string(),
            role: "developer".to_string(),
            provider: None,
            machine_id: "test-machine".to_string(),
            state: AgentState::Idle,
            current_task_id: None,
            last_seen: None,
            created_at: None,
        }).await.unwrap();

        agent_repo.upsert(&Agent {
            agent_id: AgentId::from("a-2"),
            workspace_id: WorkspaceId::from("ws-1"),
            name: "Agent 2".to_string(),
            role: "developer".to_string(),
            provider: None,
            machine_id: "test-machine".to_string(),
            state: AgentState::Idle,
            current_task_id: None,
            last_seen: None,
            created_at: None,
        }).await.unwrap();

        task_repo.insert(&Task {
            task_id: TaskId::from("t-1"),
            workspace_id: WorkspaceId::from("ws-1"),
            title: "Task 1".to_string(),
            objective: None,
            status: TaskStatus::InProgress,
            revision: 1,
            rework_cycle: 0,
            assigned_agent_id: Some(AgentId::from("a-1")),
            delegated_from_agent_id: None,
            parent_task_id: None,
            use_cases: None,
            prerequisites: None,
            verification_plan: None,
            required_skills: None,
            locked_files: None,
            impact_analysis: None,
            priority: 1,
            created_at: None,
            updated_at: None,
            completed_at: None,
        }).await.unwrap();

        let handover = Handover {
            handover_id: "h-1".to_string(),
            task_id: TaskId::from("t-1"),
            from_agent_id: AgentId::from("a-1"),
            to_agent_id: None,
            reason: "rate_limit".to_string(),
            package: serde_json::json!({"next_steps": ["step 1"]}),
            provider_cooldown_until: Some(Utc::now() + chrono::Duration::hours(1)),
            created_at: None,
            accepted_at: None,
        };

        repo.insert_handover(&handover).await.unwrap();

        let found = repo.find_handover("h-1").await.unwrap().unwrap();
        assert_eq!(found.reason, "rate_limit");
        assert_eq!(found.package.get("next_steps").unwrap().as_array().unwrap().len(), 1);

        let mut updated = found;
        updated.to_agent_id = Some(AgentId::from("a-2"));
        updated.accepted_at = Some(Utc::now());
        repo.update_handover(&updated).await.unwrap();

        let found2 = repo.find_handover("h-1").await.unwrap().unwrap();
        assert_eq!(found2.to_agent_id, Some(AgentId::from("a-2")));
    }

    #[tokio::test]
    async fn test_provider_status_repo() {
        let db = Database::open_in_memory().await.unwrap();
        let repo = SqliteProviderStatusRepo::new(db.conn().clone());

        let until = Utc::now() + chrono::Duration::minutes(30);
        repo.set_cooldown("mach-1", "claude", until, Some("rate limit".to_string())).await.unwrap();

        let cool = repo.get_cooldown("mach-1", "claude").await.unwrap().unwrap();
        // compare timestamps approximately within a second due to serialization roundtrip
        assert!((cool - until).num_seconds().abs() < 2);
    }
}
