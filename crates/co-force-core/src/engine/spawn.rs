//! Spawn agent use case implementation.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::orchestration::process_mgr::{ProcessManager, SpawnDirective};

/// Spawn request payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnRequest {
    pub provider: String,
    pub task_id: String,
    pub placement: String, // "local" or "server"
    pub workspace_path: String,
}

/// Spawn response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnResponse {
    pub directive: Option<SpawnDirective>,
    pub pid: Option<u32>,
}

/// Use case that orchestrates the spawning of new subagents.
pub struct SpawnUseCase;

impl SpawnUseCase {
    pub fn new() -> Self {
        Self
    }

    pub async fn execute(&self, req: SpawnRequest) -> Result<SpawnResponse> {
        // Generate a child agent enrollment token
        let token = format!("tok-spawn-{}", uuid::Uuid::new_v4());

        let directive = ProcessManager::build_directive(
            &req.provider,
            &req.task_id,
            &token,
            &req.workspace_path,
        )?;

        if req.placement == "server" {
            let pid = ProcessManager::spawn_local(&directive).await?;
            Ok(SpawnResponse {
                directive: None,
                pid: Some(pid),
            })
        } else {
            Ok(SpawnResponse {
                directive: Some(directive),
                pid: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_local_directive() {
        let usecase = SpawnUseCase::new();
        let req = SpawnRequest {
            provider: "claude".to_string(),
            task_id: "t-1".to_string(),
            placement: "local".to_string(),
            workspace_path: "/tmp".to_string(),
        };
        let res = usecase.execute(req).await.unwrap();
        assert!(res.directive.is_some());
        assert_eq!(res.directive.unwrap().command, "claude");
    }
}
