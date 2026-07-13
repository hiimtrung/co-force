use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::engine::ports::{ActivityRepository, LockRepository};
use crate::orchestration::bus::{WorkspaceEvent, WorkspaceEventBus};
use crate::types::{
    ActivityId, ActivityType, AgentActivity, AgentId, FileLock, TaskId, WorkspaceId,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFilesRequest {
    pub workspace_id: String,
    pub agent_id: String,
    pub file_paths: Vec<String>,
    pub machine_id: String,
    pub task_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFilesResponse {
    pub locked_files: Vec<String>,
}

pub struct LockFilesUseCase {
    lock_repo: Arc<dyn LockRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
    bus: WorkspaceEventBus,
}

impl LockFilesUseCase {
    pub fn new(
        lock_repo: Arc<dyn LockRepository>,
        activity_repo: Arc<dyn ActivityRepository>,
        bus: WorkspaceEventBus,
    ) -> Self {
        Self {
            lock_repo,
            activity_repo,
            bus,
        }
    }

    pub async fn execute(&self, req: LockFilesRequest) -> Result<LockFilesResponse> {
        let ws_id = WorkspaceId::from(req.workspace_id.clone());
        let agent_id = AgentId::from(req.agent_id.clone());
        let task_id = req.task_id.as_ref().map(|id| TaskId::from(id.clone()));

        let locks: Vec<FileLock> = req
            .file_paths
            .iter()
            .map(|path| FileLock {
                id: None,
                workspace_id: ws_id.clone(),
                file_path: path.clone(),
                agent_id: agent_id.clone(),
                machine_id: req.machine_id.clone(),
                task_id: task_id.clone(),
                reason: req.reason.clone(),
                locked_at: Some(Utc::now()),
                expires_at: None,
            })
            .collect();

        self.lock_repo.acquire_locks(&locks).await?;

        let activity = AgentActivity {
            activity_id: ActivityId::new(),
            workspace_id: ws_id,
            agent_id,
            activity_type: ActivityType::LockAcquired,
            content: Some(serde_json::json!({
                "summary": format!("Acquired locks on {} files", req.file_paths.len()),
                "reason": req.reason,
            })),
            related_task_id: task_id,
            related_files: Some(req.file_paths.clone()),
            version: 1,
            occurred_at: Utc::now(),
        };

        self.activity_repo.log_activity(&activity).await?;

        self.bus.send(WorkspaceEvent::FilesLocked {
            agent_id: req.agent_id,
            files: req.file_paths.clone(),
        });

        Ok(LockFilesResponse {
            locked_files: req.file_paths,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ports::{MockActivityRepository, MockLockRepository};
    use mockall::predicate::*;

    #[tokio::test]
    async fn test_lock_files_success() {
        let mut lock_repo = MockLockRepository::new();
        let mut activity_repo = MockActivityRepository::new();

        lock_repo
            .expect_acquire_locks()
            .with(always())
            .times(1)
            .returning(|_| Ok(()));

        activity_repo
            .expect_log_activity()
            .with(always())
            .times(1)
            .returning(|_| Ok(()));

        let usecase = LockFilesUseCase::new(
            Arc::new(lock_repo),
            Arc::new(activity_repo),
            WorkspaceEventBus::new(10),
        );

        let req = LockFilesRequest {
            workspace_id: "ws-1".to_string(),
            agent_id: "agent-1".to_string(),
            file_paths: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
            machine_id: "machine-1".to_string(),
            task_id: Some("task-1".to_string()),
            reason: Some("editing".to_string()),
        };

        let res = usecase.execute(req).await.unwrap();
        assert_eq!(res.locked_files.len(), 2);
        assert_eq!(res.locked_files[0], "src/main.rs");
    }

    #[tokio::test]
    async fn test_lock_files_conflict() {
        let mut lock_repo = MockLockRepository::new();
        let activity_repo = MockActivityRepository::new();

        // Simulate conflict error from repository
        lock_repo
            .expect_acquire_locks()
            .with(always())
            .times(1)
            .returning(|_| Err(anyhow::anyhow!("Lock conflict")));

        let usecase = LockFilesUseCase::new(
            Arc::new(lock_repo),
            Arc::new(activity_repo),
            WorkspaceEventBus::new(10),
        );

        let req = LockFilesRequest {
            workspace_id: "ws-1".to_string(),
            agent_id: "agent-1".to_string(),
            file_paths: vec!["src/main.rs".to_string()],
            machine_id: "machine-1".to_string(),
            task_id: None,
            reason: None,
        };

        let res = usecase.execute(req).await;
        assert!(res.is_err());
    }
}
