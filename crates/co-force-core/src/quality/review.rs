//! Review and Critique use cases (Plan 07 §5-6).
//!
//! Key invariants:
//! - Reviewer MUST differ from author (enforced via state_machine.check_reviewer_policy)
//! - Code review results are stored in `reviews` table
//! - Critiques fan out to multiple critics and are consolidated by the server

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::engine::ports::TaskRepository;
use crate::orchestration::bus::{WorkspaceEvent, WorkspaceEventBus};
use crate::quality::state_machine::{
    EvidenceSummary, QualityPolicy, ReviewSummary, TransitionContext,
};
use crate::types::{AgentId, TaskId, TaskStatus, WorkspaceId};

// ---------------------------------------------------------------------------
// SubmitReviewUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SubmitReviewRequest {
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub reviewer_agent_id: AgentId,
    pub reviewer_provider: Option<String>,
    /// "approved" | "changes_requested"
    pub verdict: String,
    /// JSON array of findings: [{file, line, severity, issue, suggestion}]
    pub findings: Option<serde_json::Value>,
    pub task_revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitReviewResponse {
    pub review_id: String,
    pub verdict: String,
    pub next_action: String,
}

pub struct SubmitReviewUseCase {
    task_repo: Arc<dyn TaskRepository>,
    conn: tokio_rusqlite::Connection,
    bus: WorkspaceEventBus,
}

impl SubmitReviewUseCase {
    pub fn new(
        task_repo: Arc<dyn TaskRepository>,
        conn: tokio_rusqlite::Connection,
        bus: WorkspaceEventBus,
    ) -> Self {
        Self {
            task_repo,
            conn,
            bus,
        }
    }

    pub async fn execute(&self, req: SubmitReviewRequest) -> Result<SubmitReviewResponse> {
        // 1. Load task — must be in CodeReview
        let mut task = self
            .task_repo
            .find_by_id(&req.task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", req.task_id))?;

        if !matches!(task.status, TaskStatus::CodeReview) {
            anyhow::bail!(
                "GATE_VIOLATION: submit_review requires task to be in code_review status. \
                 Current status: {:?}",
                task.status
            );
        }

        // 2. Validate verdict value
        if !matches!(req.verdict.as_str(), "approved" | "changes_requested") {
            anyhow::bail!(
                "Invalid verdict '{}'. Must be 'approved' or 'changes_requested'.",
                req.verdict
            );
        }

        // 3. Store review
        let review_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let rid = review_id.clone();
        let ws_id = req.workspace_id.to_string();
        let task_id_str = req.task_id.to_string();
        let reviewer_str = req.reviewer_agent_id.to_string();
        let verdict = req.verdict.clone();
        let findings_str = req
            .findings
            .as_ref()
            .and_then(|f| serde_json::to_string(f).ok());
        let revision = req.task_revision;

        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO reviews \
                     (review_id, task_id, workspace_id, task_revision, reviewer_agent_id, \
                      verdict, findings, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        rid,
                        task_id_str,
                        ws_id,
                        revision,
                        reviewer_str,
                        verdict,
                        findings_str,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to store review: {e}"))?;

        // 4. Determine next action
        let (next_action, new_status) = if req.verdict == "approved" {
            // Check if we have enough reviews via a quick count
            let count = self
                .count_approved_reviews(&req.task_id, req.task_revision)
                .await?;
            let policy = QualityPolicy::default(); // TODO: load from DB

            if count >= policy.reviews_required as i64 {
                // Enough reviews → task moves to Completed
                (
                    "Task passed code review. Use co_force_update_task to mark completed."
                        .to_string(),
                    Some(TaskStatus::Completed),
                )
            } else {
                (
                    format!(
                        "Review recorded. Need {} more approval(s).",
                        policy.reviews_required as i64 - count
                    ),
                    None,
                )
            }
        } else {
            // changes_requested → Rework
            task.status = TaskStatus::Rework;
            task.rework_cycle += 1;
            task.updated_at = Some(Utc::now());
            self.task_repo.update(&task).await?;

            self.bus.send(WorkspaceEvent::TaskUpdated {
                task_id: req.task_id.to_string(),
                new_status: "rework".to_string(),
            });

            (
                "Task requires rework. Developer must address findings and resubmit.".to_string(),
                None,
            )
        };

        if let Some(status) = new_status {
            task.status = status;
            task.updated_at = Some(Utc::now());
            self.task_repo.update(&task).await?;

            self.bus.send(WorkspaceEvent::TaskUpdated {
                task_id: req.task_id.to_string(),
                new_status: format!("{:?}", task.status).to_lowercase(),
            });
        }

        Ok(SubmitReviewResponse {
            review_id,
            verdict: req.verdict,
            next_action,
        })
    }

    async fn count_approved_reviews(&self, task_id: &TaskId, revision: i64) -> Result<i64> {
        let tid = task_id.to_string();
        let count = self
            .conn
            .call(move |conn| {
                conn.query_row(
                    "SELECT count(*) FROM reviews \
                     WHERE task_id = ?1 AND task_revision = ?2 AND verdict = 'approved'",
                    rusqlite::params![tid, revision],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0);
                Ok(0i64)
            })
            .await
            .unwrap_or(0);

        Ok(count)
    }
}

// ---------------------------------------------------------------------------
// Unit Tests (TDD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ports::MockTaskRepository;
    use crate::orchestration::bus::WorkspaceEventBus;
    use crate::types::{Task, TaskId, WorkspaceId};

    fn make_task_in_code_review() -> Task {
        Task {
            task_id: TaskId::from("task-1"),
            workspace_id: WorkspaceId::from("ws-1"),
            title: "Auth implementation".to_string(),
            objective: None,
            status: TaskStatus::CodeReview,
            revision: 2,
            rework_cycle: 0,
            assigned_agent_id: Some(AgentId::from("dev-1")),
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
        }
    }

    #[tokio::test]
    async fn test_submit_review_requires_code_review_status() {
        let db = crate::db::Database::open_in_memory().await.unwrap();
        let bus = WorkspaceEventBus::new(16);

        let mut mock_task = MockTaskRepository::new();
        mock_task.expect_find_by_id().returning(|_| {
            let mut t = make_task_in_code_review();
            t.status = TaskStatus::InProgress; // Wrong status!
            Ok(Some(t))
        });

        let uc = SubmitReviewUseCase::new(Arc::new(mock_task), db.conn().clone(), bus);

        let req = SubmitReviewRequest {
            task_id: TaskId::from("task-1"),
            workspace_id: WorkspaceId::from("ws-1"),
            reviewer_agent_id: AgentId::from("reviewer-1"),
            reviewer_provider: None,
            verdict: "approved".to_string(),
            findings: None,
            task_revision: 2,
        };

        let result = uc.execute(req).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("GATE_VIOLATION"));
    }

    #[tokio::test]
    async fn test_submit_review_changes_requested_moves_to_rework() {
        let db = crate::db::Database::open_in_memory().await.unwrap();

        // Insert an agent first (foreign key constraint)
        db.conn()
            .call(|conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO agents (agent_id, workspace_id, name, machine_id) \
                     VALUES ('dev-1', 'ws-1', 'Dev', 'machine-1')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let bus = WorkspaceEventBus::new(16);
        let mut mock_task = MockTaskRepository::new();
        mock_task
            .expect_find_by_id()
            .returning(|_| Ok(Some(make_task_in_code_review())));
        mock_task.expect_update().returning(|_| Ok(()));

        let uc = SubmitReviewUseCase::new(Arc::new(mock_task), db.conn().clone(), bus);

        let req = SubmitReviewRequest {
            task_id: TaskId::from("task-1"),
            workspace_id: WorkspaceId::from("ws-1"),
            reviewer_agent_id: AgentId::from("reviewer-1"),
            reviewer_provider: Some("claude".to_string()),
            verdict: "changes_requested".to_string(),
            findings: Some(serde_json::json!([{
                "file": "src/auth.rs",
                "line": 42,
                "severity": "major",
                "issue": "Null pointer dereference",
                "suggestion": "Add Option handling"
            }])),
            task_revision: 2,
        };

        let res = uc.execute(req).await.unwrap();
        assert_eq!(res.verdict, "changes_requested");
        assert!(res.next_action.contains("rework"));
    }

    #[tokio::test]
    async fn test_submit_review_invalid_verdict_rejected() {
        let db = crate::db::Database::open_in_memory().await.unwrap();
        let bus = WorkspaceEventBus::new(16);

        let mut mock_task = MockTaskRepository::new();
        mock_task
            .expect_find_by_id()
            .returning(|_| Ok(Some(make_task_in_code_review())));

        let uc = SubmitReviewUseCase::new(Arc::new(mock_task), db.conn().clone(), bus);

        let req = SubmitReviewRequest {
            task_id: TaskId::from("task-1"),
            workspace_id: WorkspaceId::from("ws-1"),
            reviewer_agent_id: AgentId::from("reviewer-1"),
            reviewer_provider: None,
            verdict: "maybe".to_string(), // Invalid!
            findings: None,
            task_revision: 2,
        };

        let result = uc.execute(req).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid verdict"));
    }
}
