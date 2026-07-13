//! Skills management use cases: create, list, and read skills (Plan 04 §6).
//!
//! Reified skills are derived from memory entries or registered manually.
//! Stored in the SQLite database 'skills' table.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::types::{SkillId, WorkspaceId};

// ---------------------------------------------------------------------------
// CreateSkillUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CreateSkillRequest {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub steps: Vec<String>,
    pub source_memories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSkillResponse {
    pub skill_id: String,
    pub name: String,
}

pub struct CreateSkillUseCase {
    conn: tokio_rusqlite::Connection,
}

impl CreateSkillUseCase {
    pub fn new(conn: tokio_rusqlite::Connection) -> Self {
        Self { conn }
    }

    pub async fn execute(&self, req: CreateSkillRequest) -> Result<CreateSkillResponse> {
        let skill_id = SkillId::new().to_string();
        let now = Utc::now().to_rfc3339();

        let steps_json = serde_json::to_string(&req.steps)?;
        let source_memories_json = serde_json::to_string(&req.source_memories)?;

        let sid = skill_id.clone();
        let ws_id = req.workspace_id.to_string();
        let name = req.name.clone();
        let description = req.description.clone();
        let category = req.category.clone();

        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO skills \
                     (skill_id, workspace_id, name, description, category, steps, source_memories, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    rusqlite::params![
                        sid,
                        ws_id,
                        name,
                        description,
                        category,
                        steps_json,
                        source_memories_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create skill: {e}"))?;

        Ok(CreateSkillResponse {
            skill_id,
            name: req.name,
        })
    }
}

// ---------------------------------------------------------------------------
// ListSkillsUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ListSkillsRequest {
    pub workspace_id: WorkspaceId,
    pub category_filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub skill_id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub usage_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSkillsResponse {
    pub skills: Vec<SkillSummary>,
}

pub struct ListSkillsUseCase {
    conn: tokio_rusqlite::Connection,
}

impl ListSkillsUseCase {
    pub fn new(conn: tokio_rusqlite::Connection) -> Self {
        Self { conn }
    }

    pub async fn execute(&self, req: ListSkillsRequest) -> Result<ListSkillsResponse> {
        let ws_id = req.workspace_id.to_string();
        let category = req.category_filter.clone();

        let skills = self
            .conn
            .call(move |conn| {
                let mut query = "SELECT skill_id, name, description, category, usage_count \
                                 FROM skills WHERE workspace_id = ?1"
                    .to_string();
                let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(ws_id)];

                if let Some(cat) = category {
                    query.push_str(" AND category = ?2");
                    params.push(Box::new(cat));
                }

                let mut stmt = conn.prepare(&query)?;
                let rows = stmt.query_map(
                    rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                    |row| {
                        Ok(SkillSummary {
                            skill_id: row.get(0)?,
                            name: row.get(1)?,
                            description: row.get(2)?,
                            category: row.get(3)?,
                            usage_count: row.get(4)?,
                        })
                    },
                )?;

                let mut result = Vec::new();
                for r in rows {
                    result.push(r?);
                }
                Ok(result)
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list skills: {e}"))?;

        Ok(ListSkillsResponse { skills })
    }
}

// ---------------------------------------------------------------------------
// GetSkillUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GetSkillRequest {
    pub workspace_id: WorkspaceId,
    pub skill_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSkillResponse {
    pub skill_id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub steps: Vec<String>,
    pub source_memories: Vec<String>,
    pub usage_count: i64,
    pub created_at: String,
}

pub struct GetSkillUseCase {
    conn: tokio_rusqlite::Connection,
}

impl GetSkillUseCase {
    pub fn new(conn: tokio_rusqlite::Connection) -> Self {
        Self { conn }
    }

    pub async fn execute(&self, req: GetSkillRequest) -> Result<GetSkillResponse> {
        let sid = req.skill_id.clone();
        let ws_id = req.workspace_id.to_string();

        let res = self
            .conn
            .call(move |conn| {
                let row_res = conn.query_row(
                    "SELECT skill_id, name, description, category, steps, source_memories, usage_count, created_at \
                     FROM skills WHERE workspace_id = ?1 AND skill_id = ?2",
                    rusqlite::params![ws_id, sid],
                    |row| {
                        let steps_str: String = row.get(4)?;
                        let source_str: String = row.get(5)?;

                        let steps: Vec<String> = serde_json::from_str(&steps_str).unwrap_or_default();
                        let source_memories: Vec<String> = serde_json::from_str(&source_str).unwrap_or_default();

                        Ok(GetSkillResponse {
                            skill_id: row.get(0)?,
                            name: row.get(1)?,
                            description: row.get(2)?,
                            category: row.get(3)?,
                            steps,
                            source_memories,
                            usage_count: row.get(6)?,
                            created_at: row.get(7)?,
                        })
                    },
                )?;
                Ok(row_res)
            })
            .await
            .map_err(|e| anyhow::anyhow!("Skill not found or error occurred: {e}"))?;

        // Increment usage count asynchronously in background
        let sid2 = req.skill_id.clone();
        let ws_id2 = req.workspace_id.to_string();
        let conn_clone = self.conn.clone();
        tokio::spawn(async move {
            let _ = conn_clone
                .call(move |conn| {
                    let updated = conn.execute(
                        "UPDATE skills SET usage_count = usage_count + 1 WHERE workspace_id = ?1 AND skill_id = ?2",
                        rusqlite::params![ws_id2, sid2],
                    )?;
                    Ok(updated)
                })
                .await;
        });

        Ok(res)
    }
}

// ---------------------------------------------------------------------------
// Unit Tests (TDD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[tokio::test]
    async fn test_create_list_and_get_skill() {
        let db = Database::open_in_memory().await.unwrap();

        let create_uc = CreateSkillUseCase::new(db.conn().clone());
        let list_uc = ListSkillsUseCase::new(db.conn().clone());
        let get_uc = GetSkillUseCase::new(db.conn().clone());

        let ws = WorkspaceId::from("ws-1");

        // 1. Create a skill
        let res = create_uc
            .execute(CreateSkillRequest {
                workspace_id: ws.clone(),
                name: "Docker build".to_string(),
                description: Some("How to build Docker images".to_string()),
                category: Some("devops".to_string()),
                steps: vec![
                    "Write Dockerfile".to_string(),
                    "Run docker build".to_string(),
                ],
                source_memories: vec!["mem-1".to_string()],
            })
            .await
            .unwrap();

        assert!(!res.skill_id.is_empty());
        assert_eq!(res.name, "Docker build");

        // 2. List skills
        let list_res = list_uc
            .execute(ListSkillsRequest {
                workspace_id: ws.clone(),
                category_filter: Some("devops".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(list_res.skills.len(), 1);
        assert_eq!(list_res.skills[0].name, "Docker build");

        // 3. Get skill
        let get_res = get_uc
            .execute(GetSkillRequest {
                workspace_id: ws.clone(),
                skill_id: res.skill_id.clone(),
            })
            .await
            .unwrap();

        assert_eq!(get_res.name, "Docker build");
        assert_eq!(get_res.steps.len(), 2);
        assert_eq!(get_res.steps[0], "Write Dockerfile");
        assert_eq!(get_res.source_memories.len(), 1);
        assert_eq!(get_res.source_memories[0], "mem-1");
    }
}
