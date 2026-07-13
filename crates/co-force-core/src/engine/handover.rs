//! Handover use case implementation.

use std::sync::Arc;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::engine::ports::{ActivityRepository, HandoverRepository, TaskRepository};
use crate::orchestration::bus::{WorkspaceEvent, WorkspaceEventBus};
use crate::types::{ActivityType, AgentActivity, AgentId, Handover, TaskId, TaskStatus};

/// Handover request payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoverRequest {
    pub task_id: String,
    pub from_agent_id: String,
    pub reason: String,
    pub target_provider: String,
    pub package: serde_json::Value,
    pub provider_cooldown_until: Option<DateTime<Utc>>,
}

/// Handover response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoverResponse {
    pub handover_id: String,
    pub safe_to_exit: bool,
}

/// Use case that orchestrates agent-to-agent task handover.
pub struct HandoverUseCase {
    handover_repo: Arc<dyn HandoverRepository>,
    task_repo: Arc<dyn TaskRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
    bus: WorkspaceEventBus,
}

impl HandoverUseCase {
    pub fn new(
        handover_repo: Arc<dyn HandoverRepository>,
        task_repo: Arc<dyn TaskRepository>,
        activity_repo: Arc<dyn ActivityRepository>,
        bus: WorkspaceEventBus,
    ) -> Self {
        Self {
            handover_repo,
            task_repo,
            activity_repo,
            bus,
        }
    }

    pub async fn execute(&self, req: HandoverRequest) -> Result<HandoverResponse> {
        let task_id = TaskId::from(req.task_id.clone());
        let agent_id = AgentId::from(req.from_agent_id.clone());

        // 1. Fetch and validate task
        let mut task = self
            .task_repo
            .find_by_id(&task_id)
            .await?
            .context("Task not found")?;

        if task.assigned_agent_id.as_ref() != Some(&agent_id) {
            bail!("Task is not assigned to the requesting agent");
        }

        // 2. Validate handover package
        let package = &req.package;
        let has_next_steps = package.get("next_steps").and_then(|v| v.as_array()).map(|a| !a.is_empty()).unwrap_or(false);
        let has_remaining = package.get("progress").and_then(|p| p.get("remaining")).and_then(|v| v.as_array()).map(|a| !a.is_empty()).unwrap_or(false);

        if !has_next_steps || !has_remaining {
            bail!("HANDOVER_INCOMPLETE: Handover package is missing progress.remaining or next_steps");
        }

        // 3. Generate handover record
        let handover_id = uuid::Uuid::new_v4().to_string();
        let handover = Handover {
            handover_id: handover_id.clone(),
            task_id: task_id.clone(),
            from_agent_id: agent_id.clone(),
            to_agent_id: None,
            reason: req.reason.clone(),
            package: req.package.clone(),
            provider_cooldown_until: req.provider_cooldown_until,
            created_at: Some(Utc::now()),
            accepted_at: None,
        };

        self.handover_repo.insert_handover(&handover).await?;

        // 4. Update task status to PendingHandover
        task.status = TaskStatus::PendingHandover;
        self.task_repo.update(&task).await?;

        // 5. Append activity log
        let activity_id = crate::types::ids::ActivityId::new();
        self.activity_repo
            .log_activity(&AgentActivity {
                activity_id,
                workspace_id: task.workspace_id.clone(),
                agent_id: agent_id.clone(),
                activity_type: ActivityType::ContextShared,
                content: Some(serde_json::json!({
                    "summary": format!("Handover requested for task {}", task_id.as_ref()),
                    "details": format!("Reason: {}", req.reason),
                })),
                related_task_id: Some(task_id.clone()),
                related_files: None,
                version: 1,
                occurred_at: Utc::now(),
            })
            .await?;

        // 6. Broadcast handover request event
        self.bus.send(WorkspaceEvent::HandoverRequested {
            old_agent_id: agent_id.as_ref().to_string(),
            task_id: task_id.as_ref().to_string(),
            next_provider: req.target_provider.clone(),
        });

        // 7. Emit general task update event
        self.bus.send(WorkspaceEvent::TaskUpdated {
            task_id: task_id.as_ref().to_string(),
            new_status: TaskStatus::PendingHandover.to_string(),
        });

        Ok(HandoverResponse {
            handover_id,
            safe_to_exit: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ports::{MockActivityRepository, MockHandoverRepository, MockTaskRepository};
    use crate::types::{Task, WorkspaceId};

    #[tokio::test]
    async fn test_handover_success() {
        let mut handover_repo = MockHandoverRepository::new();
        let mut task_repo = MockTaskRepository::new();
        let mut activity_repo = MockActivityRepository::new();
        let bus = WorkspaceEventBus::new(10);

        let _task_id = TaskId::from("t-1");
        let _agent_id = AgentId::from("a-1");

        // Mock task repo
        task_repo.expect_find_by_id().returning(move |_| {
            Ok(Some(Task {
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
            }))
        });

        task_repo.expect_update().returning(|_| Ok(()));

        // Mock handover insert
        handover_repo.expect_insert_handover().returning(|_| Ok(()));

        // Mock activity repo
        activity_repo.expect_log_activity().returning(|_| Ok(()));

        let usecase = HandoverUseCase::new(
            Arc::new(handover_repo),
            Arc::new(task_repo),
            Arc::new(activity_repo),
            bus.clone(),
        );

        let mut rx = bus.subscribe();

        let req = HandoverRequest {
            task_id: "t-1".to_string(),
            from_agent_id: "a-1".to_string(),
            reason: "rate_limit".to_string(),
            target_provider: "agy".to_string(),
            package: serde_json::json!({
                "progress": {
                    "done": ["step 1"],
                    "remaining": ["step 2"]
                },
                "next_steps": ["step 2"]
            }),
            provider_cooldown_until: None,
        };

        let res = usecase.execute(req).await.unwrap();
        assert!(res.safe_to_exit);

        // Verify broadcast events
        let event1 = rx.recv().await.unwrap();
        if let WorkspaceEvent::HandoverRequested { old_agent_id, task_id, next_provider } = event1 {
            assert_eq!(old_agent_id, "a-1");
            assert_eq!(task_id, "t-1");
            assert_eq!(next_provider, "agy");
        } else {
            panic!("Expected HandoverRequested event");
        }
    }
}
