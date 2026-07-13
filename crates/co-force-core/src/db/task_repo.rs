//! SQLite implementation of `TaskRepository`.

use anyhow::Result;
use async_trait::async_trait;
use tokio_rusqlite::Connection;

use crate::db::helpers::get_optional_datetime;
use crate::engine::ports::TaskRepository;
use crate::types::{AgentId, Task, TaskId, TaskStatus, WorkspaceId};

/// Concrete SQLite-backed task repository.
#[derive(Clone)]
pub struct SqliteTaskRepo {
    conn: Connection,
}

impl SqliteTaskRepo {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }
}

/// Helper to build a `Task` from a rusqlite row.
fn row_to_task(row: &rusqlite::Row<'_>) -> Result<Task, rusqlite::Error> {
    let use_cases: Option<String> = row.get(10)?;
    let prerequisites: Option<String> = row.get(11)?;
    let verification_plan: Option<String> = row.get(12)?;
    let required_skills: Option<String> = row.get(13)?;
    let locked_files: Option<String> = row.get(14)?;
    let impact_analysis: Option<String> = row.get(15)?;

    Ok(Task {
        task_id: TaskId::from(row.get::<_, String>(0)?),
        workspace_id: WorkspaceId::from(row.get::<_, String>(1)?),
        title: row.get(2)?,
        objective: row.get(3)?,
        status: TaskStatus::from_str_value(&row.get::<_, String>(4)?).unwrap_or(TaskStatus::Draft),
        revision: row.get(5)?,
        rework_cycle: row.get(6)?,
        assigned_agent_id: row.get::<_, Option<String>>(7)?.map(AgentId::from),
        delegated_from_agent_id: row.get::<_, Option<String>>(8)?.map(AgentId::from),
        parent_task_id: row.get::<_, Option<String>>(9)?.map(TaskId::from),
        use_cases: use_cases.and_then(|s| serde_json::from_str(&s).ok()),
        prerequisites: prerequisites.and_then(|s| serde_json::from_str(&s).ok()),
        verification_plan: verification_plan.and_then(|s| serde_json::from_str(&s).ok()),
        required_skills: required_skills.and_then(|s| serde_json::from_str(&s).ok()),
        locked_files: locked_files.and_then(|s| serde_json::from_str(&s).ok()),
        impact_analysis: impact_analysis.and_then(|s| serde_json::from_str(&s).ok()),
        priority: row.get(16)?,
        created_at: get_optional_datetime(row, 17).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(17, rusqlite::types::Type::Text, Box::new(e))
        })?,
        updated_at: get_optional_datetime(row, 18).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(18, rusqlite::types::Type::Text, Box::new(e))
        })?,
        completed_at: get_optional_datetime(row, 19).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(19, rusqlite::types::Type::Text, Box::new(e))
        })?,
    })
}

const SELECT_TASK_COLS: &str = "SELECT task_id, workspace_id, title, objective, status, revision, \
     rework_cycle, assigned_agent_id, delegated_from_agent_id, parent_task_id, use_cases, \
     prerequisites, verification_plan, required_skills, locked_files, impact_analysis, priority, \
     created_at, updated_at, completed_at FROM tasks";

#[async_trait]
impl TaskRepository for SqliteTaskRepo {
    async fn find_by_id(&self, id: &TaskId) -> Result<Option<Task>> {
        let id = id.clone();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!("{SELECT_TASK_COLS} WHERE task_id = ?1"))?;
                let task = stmt.query_row([id.as_ref()], row_to_task).optional()?;
                Ok(task)
            })
            .await
            .map_err(|e| anyhow::anyhow!("find_by_id failed: {e}"))
    }

    async fn insert(&self, task: &Task) -> Result<()> {
        let task = task.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO tasks (task_id, workspace_id, title, objective, status, revision, \
                     rework_cycle, assigned_agent_id, delegated_from_agent_id, parent_task_id, \
                     use_cases, prerequisites, verification_plan, required_skills, locked_files, \
                     impact_analysis, priority, created_at, updated_at, completed_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
                     ?17, \
                     COALESCE(?18, CURRENT_TIMESTAMP), \
                     COALESCE(?19, CURRENT_TIMESTAMP), \
                     ?20)",
                    rusqlite::params![
                        task.task_id.as_ref(),
                        task.workspace_id.as_ref(),
                        task.title,
                        task.objective,
                        task.status.to_string(),
                        task.revision,
                        task.rework_cycle,
                        task.assigned_agent_id.as_ref().map(|a| a.as_ref().to_string()),
                        task.delegated_from_agent_id.as_ref().map(|a| a.as_ref().to_string()),
                        task.parent_task_id.as_ref().map(|t| t.as_ref().to_string()),
                        task.use_cases.as_ref().map(|v| v.to_string()),
                        task.prerequisites.as_ref().map(|v| v.to_string()),
                        task.verification_plan.as_ref().map(|v| v.to_string()),
                        task.required_skills.as_ref().map(|v| v.to_string()),
                        task.locked_files.as_ref().map(|v| v.to_string()),
                        task.impact_analysis.as_ref().map(|v| v.to_string()),
                        task.priority,
                        task.created_at.map(|dt| dt.to_rfc3339()),
                        task.updated_at.map(|dt| dt.to_rfc3339()),
                        task.completed_at.map(|dt| dt.to_rfc3339()),
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("insert failed: {e}"))
    }

    async fn update(&self, task: &Task) -> Result<()> {
        let task = task.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "UPDATE tasks SET \
                     workspace_id = ?2, \
                     title = ?3, \
                     objective = ?4, \
                     status = ?5, \
                     revision = ?6, \
                     rework_cycle = ?7, \
                     assigned_agent_id = ?8, \
                     delegated_from_agent_id = ?9, \
                     parent_task_id = ?10, \
                     use_cases = ?11, \
                     prerequisites = ?12, \
                     verification_plan = ?13, \
                     required_skills = ?14, \
                     locked_files = ?15, \
                     impact_analysis = ?16, \
                     priority = ?17, \
                     updated_at = CURRENT_TIMESTAMP, \
                     completed_at = ?18 \
                     WHERE task_id = ?1",
                    rusqlite::params![
                        task.task_id.as_ref(),
                        task.workspace_id.as_ref(),
                        task.title,
                        task.objective,
                        task.status.to_string(),
                        task.revision,
                        task.rework_cycle,
                        task.assigned_agent_id
                            .as_ref()
                            .map(|a| a.as_ref().to_string()),
                        task.delegated_from_agent_id
                            .as_ref()
                            .map(|a| a.as_ref().to_string()),
                        task.parent_task_id.as_ref().map(|t| t.as_ref().to_string()),
                        task.use_cases.as_ref().map(|v| v.to_string()),
                        task.prerequisites.as_ref().map(|v| v.to_string()),
                        task.verification_plan.as_ref().map(|v| v.to_string()),
                        task.required_skills.as_ref().map(|v| v.to_string()),
                        task.locked_files.as_ref().map(|v| v.to_string()),
                        task.impact_analysis.as_ref().map(|v| v.to_string()),
                        task.priority,
                        task.completed_at.map(|dt| dt.to_rfc3339()),
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("update failed: {e}"))
    }

    async fn list_by_workspace(
        &self,
        workspace_id: &WorkspaceId,
        status_filter: Option<TaskStatus>,
    ) -> Result<Vec<Task>> {
        let ws_id = workspace_id.clone();
        self.conn
            .call(move |conn| {
                let tasks = if let Some(status) = status_filter {
                    let mut stmt = conn.prepare(&format!(
                        "{SELECT_TASK_COLS} WHERE workspace_id = ?1 AND status = ?2"
                    ))?;
                    let mapped = stmt.query_map(
                        rusqlite::params![ws_id.as_ref(), status.to_string()],
                        row_to_task,
                    )?;
                    mapped.collect::<Result<Vec<_>, _>>()?
                } else {
                    let mut stmt =
                        conn.prepare(&format!("{SELECT_TASK_COLS} WHERE workspace_id = ?1"))?;
                    let mapped = stmt.query_map([ws_id.as_ref()], row_to_task)?;
                    mapped.collect::<Result<Vec<_>, _>>()?
                };
                Ok(tasks)
            })
            .await
            .map_err(|e| anyhow::anyhow!("list_by_workspace failed: {e}"))
    }

    async fn list_by_agent(&self, agent_id: &AgentId) -> Result<Vec<Task>> {
        let agent_id = agent_id.clone();
        self.conn
            .call(move |conn| {
                let mut stmt =
                    conn.prepare(&format!("{SELECT_TASK_COLS} WHERE assigned_agent_id = ?1"))?;
                let tasks = stmt
                    .query_map([agent_id.as_ref()], row_to_task)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(tasks)
            })
            .await
            .map_err(|e| anyhow::anyhow!("list_by_agent failed: {e}"))
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
    use crate::db::agent_repo::SqliteAgentRepo;
    use crate::db::Database;
    use crate::engine::ports::AgentRepository;
    use crate::types::AgentState;

    async fn setup() -> (Database, SqliteTaskRepo) {
        let db = Database::open_in_memory().await.unwrap();
        let repo = SqliteTaskRepo::new(db.conn().clone());
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

    fn sample_task(id: &str, ws: &str, agent: Option<&str>) -> Task {
        Task {
            task_id: TaskId::from(id),
            workspace_id: WorkspaceId::from(ws),
            title: format!("Task {id}"),
            objective: Some("Objective".to_string()),
            status: TaskStatus::Draft,
            revision: 1,
            rework_cycle: 0,
            assigned_agent_id: agent.map(AgentId::from),
            delegated_from_agent_id: None,
            parent_task_id: None,
            use_cases: Some(serde_json::json!(["case 1"])),
            prerequisites: None,
            verification_plan: None,
            required_skills: None,
            locked_files: None,
            impact_analysis: None,
            priority: 1,
            created_at: None,
            updated_at: None,
            completed_at: None,
        }
    }

    #[tokio::test]
    async fn test_insert_and_find_by_id() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "a-1", "ws-1").await;
        let task = sample_task("t-1", "ws-1", Some("a-1"));

        repo.insert(&task).await.unwrap();
        let found = repo
            .find_by_id(&TaskId::from("t-1"))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(found.title, "Task t-1");
        assert_eq!(found.assigned_agent_id, Some(AgentId::from("a-1")));
        assert!(found.use_cases.is_some());
    }

    #[tokio::test]
    async fn test_update_task() {
        let (db, repo) = setup().await;
        ensure_agent(&db, "a-1", "ws-1").await;
        let mut task = sample_task("t-1", "ws-1", Some("a-1"));

        repo.insert(&task).await.unwrap();

        task.status = TaskStatus::InProgress;
        task.title = "Updated Title".to_string();
        repo.update(&task).await.unwrap();

        let found = repo
            .find_by_id(&TaskId::from("t-1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.title, "Updated Title");
        assert_eq!(found.status, TaskStatus::InProgress);
    }
}
