use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::engine::ports::{ActivityRepository, AgentRepository, TaskRepository};
use crate::types::{
    ActivityId, ActivityType, Agent, AgentActivity, AgentId, AgentState, Task, WorkspaceId,
};
use crate::orchestration::bus::{WorkspaceEvent, WorkspaceEventBus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckInRequest {
    pub workspace_path: String,
    pub agent_name: String,
    pub role: String,
    pub agent_id: Option<String>,
    pub provider: Option<String>,
    pub machine_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckInResponse {
    pub agent_id: String,
    pub workspace_id: String,
    pub onboarding_required: bool,
    pub pending_tasks: Vec<Task>,
}

pub struct CheckInUseCase {
    agent_repo: Arc<dyn AgentRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
    task_repo: Arc<dyn TaskRepository>,
    bus: WorkspaceEventBus,
}

impl CheckInUseCase {
    pub fn new(
        agent_repo: Arc<dyn AgentRepository>,
        activity_repo: Arc<dyn ActivityRepository>,
        task_repo: Arc<dyn TaskRepository>,
        bus: WorkspaceEventBus,
    ) -> Self {
        Self {
            agent_repo,
            activity_repo,
            task_repo,
            bus,
        }
    }

    pub async fn execute(&self, req: CheckInRequest) -> Result<CheckInResponse> {
        let workspace_id = derive_workspace_id(&req.workspace_path);
        let machine_id = req
            .machine_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let (agent_id, onboarding_required) = match &req.agent_id {
            Some(id_str) => {
                let id = AgentId::from(id_str.clone());
                let existing = self.agent_repo.find_by_id(&id).await?;
                (id, existing.is_none())
            }
            None => (AgentId::new(), true),
        };

        let agent = Agent {
            agent_id: agent_id.clone(),
            workspace_id: workspace_id.clone(),
            name: req.agent_name.clone(),
            role: req.role.clone(),
            provider: req.provider.clone(),
            machine_id,
            state: AgentState::Idle,
            current_task_id: None,
            last_seen: Some(Utc::now()),
            created_at: Some(Utc::now()),
        };

        self.agent_repo.upsert(&agent).await?;

        let activity = AgentActivity {
            activity_id: ActivityId::new(),
            workspace_id: workspace_id.clone(),
            agent_id: agent_id.clone(),
            activity_type: ActivityType::CheckIn,
            content: Some(serde_json::json!({
                "summary": format!("Agent {} ({}) checked in", agent.name, agent.role),
                "provider": agent.provider,
                "machine_id": agent.machine_id,
            })),
            related_task_id: None,
            related_files: None,
            version: 1,
            occurred_at: Utc::now(),
        };

        self.activity_repo.log_activity(&activity).await?;

        let pending_tasks = self.task_repo.list_by_agent(&agent_id).await?;

        self.bus.send(WorkspaceEvent::AgentCheckedIn {
            agent_id: agent_id.to_string(),
            workspace_id: workspace_id.to_string(),
        });

        Ok(CheckInResponse {
            agent_id: agent_id.to_string(),
            workspace_id: workspace_id.to_string(),
            onboarding_required,
            pending_tasks,
        })
    }
}

pub fn derive_workspace_id(path: &str) -> WorkspaceId {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    let result = hasher.finalize();
    WorkspaceId::from(format!("{:x}", result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ports::{MockActivityRepository, MockAgentRepository, MockTaskRepository};
    use crate::types::TaskId;
    use mockall::predicate::*;

    #[tokio::test]
    async fn test_check_in_new_agent() {
        let mut agent_repo = MockAgentRepository::new();
        let mut activity_repo = MockActivityRepository::new();
        let mut task_repo = MockTaskRepository::new();

        agent_repo
            .expect_upsert()
            .with(always())
            .times(1)
            .returning(|_| Ok(()));

        activity_repo
            .expect_log_activity()
            .with(always())
            .times(1)
            .returning(|_| Ok(()));

        task_repo
            .expect_list_by_agent()
            .with(always())
            .times(1)
            .returning(|_| Ok(vec![]));

        let usecase = CheckInUseCase::new(
            Arc::new(agent_repo),
            Arc::new(activity_repo),
            Arc::new(task_repo),
            WorkspaceEventBus::new(10),
        );

        let req = CheckInRequest {
            workspace_path: "/Users/test/workspace".to_string(),
            agent_name: "TestAgent".to_string(),
            role: "developer".to_string(),
            agent_id: None,
            provider: Some("claude".to_string()),
            machine_id: Some("machine-1".to_string()),
        };

        let res = usecase.execute(req).await.unwrap();

        assert!(!res.agent_id.is_empty());
        assert_eq!(
            res.workspace_id,
            derive_workspace_id("/Users/test/workspace").to_string()
        );
        assert!(res.onboarding_required);
        assert!(res.pending_tasks.is_empty());
    }

    #[tokio::test]
    async fn test_check_in_existing_agent() {
        let mut agent_repo = MockAgentRepository::new();
        let mut activity_repo = MockActivityRepository::new();
        let mut task_repo = MockTaskRepository::new();

        let existing_agent_id = AgentId::from("existing-id");
        let ws_id = derive_workspace_id("/Users/test/workspace");

        let existing_agent = Agent {
            agent_id: existing_agent_id.clone(),
            workspace_id: ws_id.clone(),
            name: "TestAgent".to_string(),
            role: "developer".to_string(),
            provider: Some("claude".to_string()),
            machine_id: "machine-1".to_string(),
            state: AgentState::Disconnected,
            current_task_id: None,
            last_seen: Some(Utc::now()),
            created_at: Some(Utc::now()),
        };

        let existing_agent_clone = existing_agent.clone();
        agent_repo
            .expect_find_by_id()
            .with(eq(existing_agent_id.clone()))
            .times(1)
            .returning(move |_| Ok(Some(existing_agent_clone.clone())));

        agent_repo
            .expect_upsert()
            .with(always())
            .times(1)
            .returning(|_| Ok(()));

        activity_repo
            .expect_log_activity()
            .with(always())
            .times(1)
            .returning(|_| Ok(()));

        let mock_task = Task {
            task_id: TaskId::from("task-123"),
            workspace_id: ws_id.clone(),
            title: "Task Title".to_string(),
            objective: None,
            status: crate::types::TaskStatus::InProgress,
            revision: 1,
            rework_cycle: 0,
            assigned_agent_id: Some(existing_agent_id.clone()),
            delegated_from_agent_id: None,
            parent_task_id: None,
            use_cases: None,
            prerequisites: None,
            verification_plan: None,
            required_skills: None,
            locked_files: None,
            impact_analysis: None,
            priority: 0,
            created_at: None,
            updated_at: None,
            completed_at: None,
        };

        let mock_task_clone = mock_task.clone();
        task_repo
            .expect_list_by_agent()
            .with(eq(existing_agent_id.clone()))
            .times(1)
            .returning(move |_| Ok(vec![mock_task_clone.clone()]));

        let usecase = CheckInUseCase::new(
            Arc::new(agent_repo),
            Arc::new(activity_repo),
            Arc::new(task_repo),
            WorkspaceEventBus::new(10),
        );

        let req = CheckInRequest {
            workspace_path: "/Users/test/workspace".to_string(),
            agent_name: "TestAgent".to_string(),
            role: "developer".to_string(),
            agent_id: Some("existing-id".to_string()),
            provider: Some("claude".to_string()),
            machine_id: Some("machine-1".to_string()),
        };

        let res = usecase.execute(req).await.unwrap();

        assert_eq!(res.agent_id, "existing-id");
        assert_eq!(res.workspace_id, ws_id.to_string());
        assert!(!res.onboarding_required);
        assert_eq!(res.pending_tasks.len(), 1);
        assert_eq!(res.pending_tasks[0].task_id, TaskId::from("task-123"));
    }
}
