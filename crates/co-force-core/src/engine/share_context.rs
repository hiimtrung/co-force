use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::engine::ports::{ActivityRepository, ContextRepository};
use crate::types::{
    ActivityId, ActivityType, AgentActivity, AgentId, ContextId, SharedContext, WorkspaceId,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareContextRequest {
    pub workspace_id: String,
    pub source_agent_id: String,
    pub target_agent_id: Option<String>,
    pub context_type: String,
    pub content: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareContextResponse {
    pub context_id: String,
}

pub struct ShareContextUseCase {
    context_repo: Arc<dyn ContextRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
}

impl ShareContextUseCase {
    pub fn new(
        context_repo: Arc<dyn ContextRepository>,
        activity_repo: Arc<dyn ActivityRepository>,
    ) -> Self {
        Self {
            context_repo,
            activity_repo,
        }
    }

    pub async fn execute(&self, req: ShareContextRequest) -> Result<ShareContextResponse> {
        let context_id = ContextId::new();
        let ws_id = WorkspaceId::from(req.workspace_id.clone());
        let source_agent_id = AgentId::from(req.source_agent_id.clone());
        let target_agent_id = req
            .target_agent_id
            .as_ref()
            .map(|id| AgentId::from(id.clone()));

        let ctx = SharedContext {
            context_id: context_id.clone(),
            workspace_id: ws_id.clone(),
            source_agent_id: source_agent_id.clone(),
            target_agent_id: target_agent_id.clone(),
            context_type: req.context_type.clone(),
            content: req.content.clone(),
            resolved: false,
            created_at: Some(Utc::now()),
            resolved_at: None,
        };

        self.context_repo.share_context(&ctx).await?;

        let activity = AgentActivity {
            activity_id: ActivityId::new(),
            workspace_id: ws_id,
            agent_id: source_agent_id,
            activity_type: ActivityType::ContextShared,
            content: Some(serde_json::json!({
                "summary": format!("Shared context of type '{}'", req.context_type),
                "target_agent_id": req.target_agent_id,
            })),
            related_task_id: None,
            related_files: None,
            version: 1,
            occurred_at: Utc::now(),
        };

        self.activity_repo.log_activity(&activity).await?;

        Ok(ShareContextResponse {
            context_id: context_id.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ports::{MockActivityRepository, MockContextRepository};
    use mockall::predicate::*;

    #[tokio::test]
    async fn test_share_context_success() {
        let mut context_repo = MockContextRepository::new();
        let mut activity_repo = MockActivityRepository::new();

        context_repo
            .expect_share_context()
            .with(always())
            .times(1)
            .returning(|_| Ok(()));

        activity_repo
            .expect_log_activity()
            .with(always())
            .times(1)
            .returning(|_| Ok(()));

        let usecase = ShareContextUseCase::new(Arc::new(context_repo), Arc::new(activity_repo));

        let req = ShareContextRequest {
            workspace_id: "ws-1".to_string(),
            source_agent_id: "agent-a".to_string(),
            target_agent_id: Some("agent-b".to_string()),
            context_type: "handover".to_string(),
            content: serde_json::json!({"notes": "please build module A"}),
        };

        let res = usecase.execute(req).await.unwrap();
        assert!(!res.context_id.is_empty());
    }
}
