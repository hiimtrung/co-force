//! Repository trait definitions (Ports layer).
//!
//! These traits define the interface between the Use Case Engine and
//! the persistence layer. All concrete implementations live in `db/`.
//!
//! Traits are annotated with `#[cfg_attr(test, mockall::automock)]`
//! to auto-generate mock implementations for unit testing of use cases.

use anyhow::Result;
use async_trait::async_trait;

use crate::types::{
    Agent, AgentActivity, AgentId, ContextId, FileLock, Handover, SharedContext, Task, TaskId,
    TaskStatus, WorkspaceId,
};

// ---------------------------------------------------------------------------
// AgentRepository
// ---------------------------------------------------------------------------

/// Persistence operations for `Agent` entities.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait AgentRepository: Send + Sync {
    /// Finds an agent by its ID. Returns `None` if not found.
    async fn find_by_id(&self, id: &AgentId) -> Result<Option<Agent>>;

    /// Inserts or updates an agent record (upsert by `agent_id`).
    async fn upsert(&self, agent: &Agent) -> Result<()>;

    /// Lists all agents in a workspace that are not `Disconnected`.
    async fn list_active(&self, workspace_id: &WorkspaceId) -> Result<Vec<Agent>>;

    /// Lists all agents in a workspace (including disconnected).
    async fn list_all(&self, workspace_id: &WorkspaceId) -> Result<Vec<Agent>>;
}

// ---------------------------------------------------------------------------
// ActivityRepository
// ---------------------------------------------------------------------------

/// Persistence operations for `AgentActivity` log entries.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ActivityRepository: Send + Sync {
    /// Appends an activity to the log (insert-only).
    async fn log_activity(&self, activity: &AgentActivity) -> Result<()>;

    /// Retrieves the most recent activities for a workspace, newest first.
    async fn get_workspace_stream(
        &self,
        workspace_id: &WorkspaceId,
        limit: usize,
    ) -> Result<Vec<AgentActivity>>;

    /// Retrieves activities for a specific agent.
    async fn get_agent_activities(
        &self,
        agent_id: &AgentId,
        limit: usize,
    ) -> Result<Vec<AgentActivity>>;
}

// ---------------------------------------------------------------------------
// ContextRepository
// ---------------------------------------------------------------------------

/// Persistence operations for `SharedContext` blocks.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ContextRepository: Send + Sync {
    /// Stores a new shared context block.
    async fn share_context(&self, ctx: &SharedContext) -> Result<()>;

    /// Retrieves all unresolved contexts targeted at the given agent.
    async fn get_unresolved(&self, target_agent: &AgentId) -> Result<Vec<SharedContext>>;

    /// Marks a context as resolved.
    async fn mark_resolved(&self, context_id: &ContextId) -> Result<()>;
}

// ---------------------------------------------------------------------------
// TaskRepository
// ---------------------------------------------------------------------------

/// Persistence operations for `Task` entities.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskRepository: Send + Sync {
    /// Finds a task by its ID.
    async fn find_by_id(&self, id: &TaskId) -> Result<Option<Task>>;

    /// Inserts a new task.
    async fn insert(&self, task: &Task) -> Result<()>;

    /// Updates an existing task.
    async fn update(&self, task: &Task) -> Result<()>;

    /// Lists tasks in a workspace, optionally filtered by status.
    async fn list_by_workspace(
        &self,
        workspace_id: &WorkspaceId,
        status_filter: Option<TaskStatus>,
    ) -> Result<Vec<Task>>;

    /// Lists tasks assigned to a specific agent.
    async fn list_by_agent(&self, agent_id: &AgentId) -> Result<Vec<Task>>;
}

// ---------------------------------------------------------------------------
// LockRepository
// ---------------------------------------------------------------------------

/// Persistence operations for `FileLock` entities.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait LockRepository: Send + Sync {
    /// Acquires locks on the given file paths. Returns an error if any
    /// path is already locked by a different agent.
    async fn acquire_locks(&self, locks: &[FileLock]) -> Result<()>;

    /// Releases locks held by the agent on the given paths.
    async fn release_locks(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        paths: &[String],
    ) -> Result<()>;

    /// Lists all active locks in a workspace.
    async fn list_locks(&self, workspace_id: &WorkspaceId) -> Result<Vec<FileLock>>;

    /// Releases all locks held by a specific agent (used during reclaim).
    async fn release_all_for_agent(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Result<()>;
}

// ---------------------------------------------------------------------------
// HandoverRepository
// ---------------------------------------------------------------------------

/// Operations on agent handover records.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait HandoverRepository: Send + Sync {
    async fn insert_handover(&self, handover: &Handover) -> Result<()>;
    async fn find_handover(&self, handover_id: &str) -> Result<Option<Handover>>;
    async fn find_pending_for_task(&self, task_id: &TaskId) -> Result<Option<Handover>>;
    async fn update_handover(&self, handover: &Handover) -> Result<()>;
}

// ---------------------------------------------------------------------------
// ProviderStatusRepository
// ---------------------------------------------------------------------------

/// Operations on machine/provider limits and cooldown tracking.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ProviderStatusRepository: Send + Sync {
    async fn set_cooldown(
        &self,
        machine_id: &str,
        provider: &str,
        until: chrono::DateTime<chrono::Utc>,
        error: Option<String>,
    ) -> Result<()>;
    async fn get_cooldown(
        &self,
        machine_id: &str,
        provider: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>>;
}
