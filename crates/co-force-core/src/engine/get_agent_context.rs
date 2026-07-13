use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::engine::ports::{ActivityRepository, AgentRepository, ContextRepository};
use crate::types::{Agent, AgentActivity, AgentId, SharedContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetAgentContextRequest {
    pub agent_id: String,
    pub include_history: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetAgentContextResponse {
    pub agent: Option<Agent>,
    pub activities: Vec<AgentActivity>,
    pub unresolved_contexts: Vec<SharedContext>,
}

pub struct GetAgentContextUseCase {
    agent_repo: Arc<dyn AgentRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
    context_repo: Arc<dyn ContextRepository>,
}

impl GetAgentContextUseCase {
    pub fn new(
        agent_repo: Arc<dyn AgentRepository>,
        activity_repo: Arc<dyn ActivityRepository>,
        context_repo: Arc<dyn ContextRepository>,
    ) -> Self {
        Self {
            agent_repo,
            activity_repo,
            context_repo,
        }
    }

    pub async fn execute(&self, req: GetAgentContextRequest) -> Result<GetAgentContextResponse> {
        let agent_id = AgentId::from(req.agent_id.clone());

        let agent = self.agent_repo.find_by_id(&agent_id).await?;

        let activities = if req.include_history.unwrap_or(false) {
            self.activity_repo
                .get_agent_activities(&agent_id, 20)
                .await?
        } else {
            vec![]
        };

        let unresolved_contexts = self.context_repo.get_unresolved(&agent_id).await?;

        Ok(GetAgentContextResponse {
            agent,
            activities,
            unresolved_contexts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ports::{
        MockActivityRepository, MockAgentRepository, MockContextRepository,
    };
    use crate::types::{AgentState, WorkspaceId};
    use chrono::Utc;
    use mockall::predicate::*;

    #[tokio::test]
    async fn test_get_agent_context_success() {
        let mut agent_repo = MockAgentRepository::new();
        let mut activity_repo = MockActivityRepository::new();
        let mut context_repo = MockContextRepository::new();

        let agent_id = AgentId::from("agent-1");
        let ws_id = WorkspaceId::from("ws-1");

        let mock_agent = Agent {
            agent_id: agent_id.clone(),
            workspace_id: ws_id.clone(),
            name: "Beta".to_string(),
            role: "developer".to_string(),
            provider: Some("claude".to_string()),
            machine_id: "machine-1".to_string(),
            state: AgentState::Idle,
            current_task_id: None,
            last_seen: Some(Utc::now()),
            created_at: Some(Utc::now()),
        };

        let mock_agent_clone = mock_agent.clone();
        agent_repo
            .expect_find_by_id()
            .with(eq(agent_id.clone()))
            .times(1)
            .returning(move |_| Ok(Some(mock_agent_clone.clone())));

        // Since include_history is true, it should call get_agent_activities
        activity_repo
            .expect_get_agent_activities()
            .with(eq(agent_id.clone()), eq(20))
            .times(1)
            .returning(|_, _| Ok(vec![]));

        context_repo
            .expect_get_unresolved()
            .with(eq(agent_id.clone()))
            .times(1)
            .returning(|_| Ok(vec![]));

        let usecase = GetAgentContextUseCase::new(
            Arc::new(agent_repo),
            Arc::new(activity_repo),
            Arc::new(context_repo),
        );

        let req = GetAgentContextRequest {
            agent_id: "agent-1".to_string(),
            include_history: Some(true),
        };

        let res = usecase.execute(req).await.unwrap();
        assert!(res.agent.is_some());
        assert_eq!(res.agent.unwrap().name, "Beta");
        assert!(res.activities.is_empty());
        assert!(res.unresolved_contexts.is_empty());
    }
}
