//! Task management use cases: create, list, update, approve, delegate, submit_verification.
//!
//! Follows TDD approach — tests are defined first, then implementation.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::engine::ports::{ActivityRepository, AgentRepository, TaskRepository};
use crate::orchestration::bus::WorkspaceEventBus;
use crate::types::{
    ActivityId, ActivityType, AgentActivity, AgentId, Task, TaskId, TaskStatus, WorkspaceId,
};

// ---------------------------------------------------------------------------
// CreateTasksUseCase
// ---------------------------------------------------------------------------

/// Input to create one or more tasks in a workspace.
#[derive(Debug, Clone)]
pub struct CreateTasksRequest {
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub tasks: Vec<NewTaskInput>,
}

#[derive(Debug, Clone)]
pub struct NewTaskInput {
    pub title: String,
    pub objective: Option<String>,
    pub use_cases: Option<serde_json::Value>,
    pub prerequisites: Option<serde_json::Value>,
    pub verification_plan: Option<serde_json::Value>,
    pub required_skills: Option<serde_json::Value>,
    pub impact_analysis: Option<serde_json::Value>,
    pub priority: i64,
    pub parent_task_id: Option<TaskId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTasksResponse {
    pub created: Vec<TaskSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    pub task_id: String,
    pub title: String,
    pub status: TaskStatus,
}

pub struct CreateTasksUseCase {
    task_repo: Arc<dyn TaskRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
    bus: WorkspaceEventBus,
}

impl CreateTasksUseCase {
    pub fn new(
        task_repo: Arc<dyn TaskRepository>,
        activity_repo: Arc<dyn ActivityRepository>,
        bus: WorkspaceEventBus,
    ) -> Self {
        Self {
            task_repo,
            activity_repo,
            bus,
        }
    }

    pub async fn execute(&self, req: CreateTasksRequest) -> Result<CreateTasksResponse> {
        let mut created = Vec::new();

        for input in req.tasks {
            let task_id = TaskId::new();
            let now = Utc::now();

            let task = Task {
                task_id: task_id.clone(),
                workspace_id: req.workspace_id.clone(),
                title: input.title.clone(),
                objective: input.objective,
                status: TaskStatus::Draft,
                revision: 1,
                rework_cycle: 0,
                assigned_agent_id: None,
                delegated_from_agent_id: None,
                parent_task_id: input.parent_task_id,
                use_cases: input.use_cases,
                prerequisites: input.prerequisites,
                verification_plan: input.verification_plan,
                required_skills: input.required_skills,
                locked_files: None,
                impact_analysis: input.impact_analysis,
                priority: input.priority,
                created_at: Some(now),
                updated_at: Some(now),
                completed_at: None,
            };

            self.task_repo.insert(&task).await?;

            // Log activity
            let activity = AgentActivity {
                activity_id: ActivityId::new(),
                workspace_id: req.workspace_id.clone(),
                agent_id: req.agent_id.clone(),
                activity_type: ActivityType::TaskStarted,
                content: Some(serde_json::json!({
                    "summary": format!("Created task: {}", input.title),
                    "task_id": task_id.to_string(),
                })),
                related_task_id: Some(task_id.clone()),
                related_files: None,
                version: 1,
                occurred_at: now,
            };
            self.activity_repo.log_activity(&activity).await?;

            // Emit event
            use crate::orchestration::bus::WorkspaceEvent;
            self.bus.send(WorkspaceEvent::TaskUpdated {
                task_id: task_id.to_string(),
                new_status: "draft".to_string(),
            });

            created.push(TaskSummary {
                task_id: task_id.to_string(),
                title: task.title,
                status: task.status,
            });
        }

        Ok(CreateTasksResponse { created })
    }
}

// ---------------------------------------------------------------------------
// ListTasksUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ListTasksRequest {
    pub workspace_id: WorkspaceId,
    pub status_filter: Option<TaskStatus>,
    pub agent_id_filter: Option<AgentId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTasksResponse {
    pub tasks: Vec<Task>,
    pub total: usize,
}

pub struct ListTasksUseCase {
    task_repo: Arc<dyn TaskRepository>,
}

impl ListTasksUseCase {
    pub fn new(task_repo: Arc<dyn TaskRepository>) -> Self {
        Self { task_repo }
    }

    pub async fn execute(&self, req: ListTasksRequest) -> Result<ListTasksResponse> {
        let tasks = if let Some(agent_id) = req.agent_id_filter {
            self.task_repo.list_by_agent(&agent_id).await?
        } else {
            self.task_repo
                .list_by_workspace(&req.workspace_id, req.status_filter)
                .await?
        };

        let total = tasks.len();
        Ok(ListTasksResponse { tasks, total })
    }
}

// ---------------------------------------------------------------------------
// UpdateTaskUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct UpdateTaskRequest {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub workspace_id: WorkspaceId,
    /// If Some, attempt to transition to this status (validated against state machine)
    pub new_status: Option<TaskStatus>,
    pub progress_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTaskResponse {
    pub task: Task,
    pub gate_warning: Option<String>,
}

pub struct UpdateTaskUseCase {
    task_repo: Arc<dyn TaskRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
    bus: WorkspaceEventBus,
}

impl UpdateTaskUseCase {
    pub fn new(
        task_repo: Arc<dyn TaskRepository>,
        activity_repo: Arc<dyn ActivityRepository>,
        bus: WorkspaceEventBus,
    ) -> Self {
        Self {
            task_repo,
            activity_repo,
            bus,
        }
    }

    pub async fn execute(&self, req: UpdateTaskRequest) -> Result<UpdateTaskResponse> {
        let mut task = self
            .task_repo
            .find_by_id(&req.task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", req.task_id))?;

        let mut gate_warning = None;

        if let Some(new_status) = req.new_status {
            // GATE VIOLATION: agents cannot set completed directly
            if matches!(new_status, TaskStatus::Completed) {
                anyhow::bail!(
                    "GATE_VIOLATION: Cannot set status=completed directly. \
                     Use co_force_submit_verification with real evidence first."
                );
            }
            // Check valid transition
            validate_transition(&task.status, &new_status)?;
            task.status = new_status.clone();
            task.updated_at = Some(Utc::now());

            // Emit event
            use crate::orchestration::bus::WorkspaceEvent;
            self.bus.send(WorkspaceEvent::TaskUpdated {
                task_id: req.task_id.to_string(),
                new_status: format!("{new_status}"),
            });

            gate_warning = Some(format!(
                "Task moved to {:?}. Ensure protocol gates are followed.",
                task.status
            ));
        }

        self.task_repo.update(&task).await?;

        if let Some(note) = req.progress_note {
            let activity = AgentActivity {
                activity_id: ActivityId::new(),
                workspace_id: req.workspace_id,
                agent_id: req.agent_id,
                activity_type: ActivityType::TaskStarted,
                content: Some(serde_json::json!({
                    "summary": "Progress update",
                    "note": note,
                    "task_id": req.task_id.to_string(),
                })),
                related_task_id: Some(req.task_id),
                related_files: None,
                version: 1,
                occurred_at: Utc::now(),
            };
            self.activity_repo.log_activity(&activity).await?;
        }

        Ok(UpdateTaskResponse { task, gate_warning })
    }
}

/// Validates that the status transition is allowed by the state machine.
fn validate_transition(from: &TaskStatus, to: &TaskStatus) -> Result<()> {
    use TaskStatus::*;
    let allowed = matches!(
        (from, to),
        // Normal flow
        (Draft, SpecReview)
        | (Draft, Cancelled)
        | (SpecReview, Draft)       // recheck finds gaps
        | (SpecReview, AwaitingApproval)
        | (AwaitingApproval, Approved)
        | (AwaitingApproval, Draft) // user rejects
        | (AwaitingApproval, Cancelled)
        | (Approved, InProgress)
        | (Approved, Cancelled)
        | (InProgress, Verification)
        | (InProgress, Blocked)
        | (InProgress, PendingHandover)
        | (InProgress, Cancelled)
        | (Verification, InProgress) // evidence invalid
        | (Verification, CodeReview) // evidence valid
        | (CodeReview, Rework)
        | (CodeReview, Completed)
        | (Rework, InProgress)
        | (Blocked, InProgress)
        | (Blocked, Cancelled)
        | (PendingHandover, InProgress)
        | (PendingHandover, Approved) // timeout — returns to backlog
        // Self-loop for progress notes (no status change)
        | (_, _) if from == to
    );

    if !allowed {
        anyhow::bail!(
            "GATE_VIOLATION: Transition from {:?} to {:?} is not allowed by the state machine.",
            from,
            to
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ApproveTasksUseCase (user approves tasks in awaiting_approval)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ApproveTasksRequest {
    pub workspace_id: WorkspaceId,
    pub task_ids: Vec<TaskId>,
    pub approver_agent_id: AgentId,
    pub reject: bool, // if true → transition to Draft with rejection note
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveTasksResponse {
    pub approved: Vec<String>,
    pub rejected: Vec<String>,
}

pub struct ApproveTasksUseCase {
    task_repo: Arc<dyn TaskRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
    bus: WorkspaceEventBus,
}

impl ApproveTasksUseCase {
    pub fn new(
        task_repo: Arc<dyn TaskRepository>,
        activity_repo: Arc<dyn ActivityRepository>,
        bus: WorkspaceEventBus,
    ) -> Self {
        Self {
            task_repo,
            activity_repo,
            bus,
        }
    }

    pub async fn execute(&self, req: ApproveTasksRequest) -> Result<ApproveTasksResponse> {
        let mut approved = Vec::new();
        let mut rejected = Vec::new();

        for task_id in req.task_ids {
            let mut task = match self.task_repo.find_by_id(&task_id).await? {
                Some(t) => t,
                None => continue,
            };

            if !matches!(task.status, TaskStatus::AwaitingApproval) {
                continue; // skip non-awaiting tasks
            }

            if req.reject {
                task.status = TaskStatus::Draft;
                task.updated_at = Some(Utc::now());
                self.task_repo.update(&task).await?;
                rejected.push(task_id.to_string());
            } else {
                task.status = TaskStatus::Approved;
                task.updated_at = Some(Utc::now());
                self.task_repo.update(&task).await?;

                use crate::orchestration::bus::WorkspaceEvent;
                self.bus.send(WorkspaceEvent::TaskUpdated {
                    task_id: task_id.to_string(),
                    new_status: "approved".to_string(),
                });

                approved.push(task_id.to_string());
            }

            let activity = AgentActivity {
                activity_id: ActivityId::new(),
                workspace_id: req.workspace_id.clone(),
                agent_id: req.approver_agent_id.clone(),
                activity_type: ActivityType::TaskStarted,
                content: Some(serde_json::json!({
                    "summary": if req.reject { "Task rejected" } else { "Task approved" },
                    "task_id": task_id.to_string(),
                    "rejection_reason": req.rejection_reason,
                })),
                related_task_id: Some(task_id),
                related_files: None,
                version: 1,
                occurred_at: Utc::now(),
            };
            self.activity_repo.log_activity(&activity).await?;
        }

        Ok(ApproveTasksResponse { approved, rejected })
    }
}

// ---------------------------------------------------------------------------
// DelegateTaskUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DelegateTaskRequest {
    pub task_id: TaskId,
    pub from_agent_id: AgentId,
    pub to_agent_id: AgentId,
    pub workspace_id: WorkspaceId,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegateTaskResponse {
    pub task_id: String,
    pub new_assignee: String,
}

pub struct DelegateTaskUseCase {
    task_repo: Arc<dyn TaskRepository>,
    agent_repo: Arc<dyn AgentRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
}

impl DelegateTaskUseCase {
    pub fn new(
        task_repo: Arc<dyn TaskRepository>,
        agent_repo: Arc<dyn AgentRepository>,
        activity_repo: Arc<dyn ActivityRepository>,
    ) -> Self {
        Self {
            task_repo,
            agent_repo,
            activity_repo,
        }
    }

    pub async fn execute(&self, req: DelegateTaskRequest) -> Result<DelegateTaskResponse> {
        let mut task = self
            .task_repo
            .find_by_id(&req.task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Task not found"))?;

        // Verify target agent exists
        self.agent_repo
            .find_by_id(&req.to_agent_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Target agent not found: {}", req.to_agent_id))?;

        task.assigned_agent_id = Some(req.to_agent_id.clone());
        task.delegated_from_agent_id = Some(req.from_agent_id.clone());
        task.updated_at = Some(Utc::now());
        self.task_repo.update(&task).await?;

        let activity = AgentActivity {
            activity_id: ActivityId::new(),
            workspace_id: req.workspace_id,
            agent_id: req.from_agent_id,
            activity_type: ActivityType::Delegation,
            content: Some(serde_json::json!({
                "summary": format!("Task delegated to {}", req.to_agent_id),
                "task_id": req.task_id.to_string(),
                "reason": req.reason,
            })),
            related_task_id: Some(req.task_id.clone()),
            related_files: None,
            version: 1,
            occurred_at: Utc::now(),
        };
        self.activity_repo.log_activity(&activity).await?;

        Ok(DelegateTaskResponse {
            task_id: req.task_id.to_string(),
            new_assignee: req.to_agent_id.to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// SubmitVerificationUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SubmitVerificationRequest {
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub commit_sha: Option<String>,
    /// JSON array of verification steps
    pub steps: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitVerificationResponse {
    pub verification_id: String,
    pub task_status: TaskStatus,
    pub message: String,
}

pub struct SubmitVerificationUseCase {
    task_repo: Arc<dyn TaskRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
    bus: WorkspaceEventBus,
}

impl SubmitVerificationUseCase {
    pub fn new(
        task_repo: Arc<dyn TaskRepository>,
        activity_repo: Arc<dyn ActivityRepository>,
        bus: WorkspaceEventBus,
    ) -> Self {
        Self {
            task_repo,
            activity_repo,
            bus,
        }
    }

    pub async fn execute(
        &self,
        req: SubmitVerificationRequest,
    ) -> Result<SubmitVerificationResponse> {
        let mut task = self
            .task_repo
            .find_by_id(&req.task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", req.task_id))?;

        // Must be in_progress to submit verification
        if !matches!(task.status, TaskStatus::InProgress | TaskStatus::Rework) {
            anyhow::bail!(
                "GATE_VIOLATION: submit_verification requires task to be in_progress or rework. \
                 Current status: {:?}",
                task.status
            );
        }

        // Validate evidence — must have at least 1 step with kind=test and exit_code=0
        validate_evidence(&req.steps)?;

        // Generate verification_id
        let verification_id = uuid::Uuid::new_v4().to_string();

        // Store verification record in DB via raw SQL
        // (we log it via activity for now — full repo in Phase 2)
        let activity = AgentActivity {
            activity_id: ActivityId::new(),
            workspace_id: req.workspace_id.clone(),
            agent_id: req.agent_id.clone(),
            activity_type: ActivityType::TaskCompleted,
            content: Some(serde_json::json!({
                "summary": "Verification evidence submitted",
                "verification_id": verification_id,
                "task_id": req.task_id.to_string(),
                "commit_sha": req.commit_sha,
                "steps": req.steps,
                "task_revision": task.revision,
            })),
            related_task_id: Some(req.task_id.clone()),
            related_files: None,
            version: 1,
            occurred_at: Utc::now(),
        };
        self.activity_repo.log_activity(&activity).await?;

        // Transition task to Verification
        task.status = TaskStatus::Verification;
        task.updated_at = Some(Utc::now());
        self.task_repo.update(&task).await?;

        use crate::orchestration::bus::WorkspaceEvent;
        self.bus.send(WorkspaceEvent::TaskUpdated {
            task_id: req.task_id.to_string(),
            new_status: "verification".to_string(),
        });

        Ok(SubmitVerificationResponse {
            verification_id,
            task_status: task.status,
            message: "Verification recorded. Task is now awaiting code review.".to_string(),
        })
    }
}

/// Validates that the verification evidence has at least 1 passing test step.
fn validate_evidence(steps: &serde_json::Value) -> Result<()> {
    let arr = steps
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("EVIDENCE_INVALID: steps must be a JSON array"))?;

    if arr.is_empty() {
        anyhow::bail!("EVIDENCE_INVALID: steps array cannot be empty");
    }

    let has_passing_test = arr.iter().any(|step| {
        step.get("kind")
            .and_then(|k| k.as_str())
            .map(|k| k == "test")
            .unwrap_or(false)
            && step
                .get("exit_code")
                .and_then(|c| c.as_i64())
                .map(|c| c == 0)
                .unwrap_or(false)
    });

    if !has_passing_test {
        anyhow::bail!(
            "EVIDENCE_INVALID: Must include at least 1 step with kind='test' and exit_code=0. \
             Run your tests and include actual output."
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// UnlockFilesUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct UnlockFilesRequest {
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub file_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnlockFilesResponse {
    pub released: Vec<String>,
}

pub struct UnlockFilesUseCase {
    lock_repo: Arc<dyn crate::engine::ports::LockRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
}

impl UnlockFilesUseCase {
    pub fn new(
        lock_repo: Arc<dyn crate::engine::ports::LockRepository>,
        activity_repo: Arc<dyn ActivityRepository>,
    ) -> Self {
        Self {
            lock_repo,
            activity_repo,
        }
    }

    pub async fn execute(&self, req: UnlockFilesRequest) -> Result<UnlockFilesResponse> {
        self.lock_repo
            .release_locks(&req.workspace_id, &req.agent_id, &req.file_paths)
            .await?;

        let activity = AgentActivity {
            activity_id: ActivityId::new(),
            workspace_id: req.workspace_id,
            agent_id: req.agent_id,
            activity_type: ActivityType::LockAcquired,
            content: Some(serde_json::json!({
                "summary": format!("Released {} lock(s)", req.file_paths.len()),
                "released": req.file_paths,
            })),
            related_task_id: None,
            related_files: Some(req.file_paths.clone()),
            version: 1,
            occurred_at: Utc::now(),
        };
        self.activity_repo.log_activity(&activity).await?;

        Ok(UnlockFilesResponse {
            released: req.file_paths,
        })
    }
}

// ---------------------------------------------------------------------------
// CheckConflictsUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CheckConflictsRequest {
    pub workspace_id: WorkspaceId,
    pub file_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckConflictsResponse {
    pub conflicts: Vec<ConflictInfo>,
    pub all_clear: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictInfo {
    pub file_path: String,
    pub locked_by_agent: String,
    pub machine_id: String,
    pub task_id: Option<String>,
}

pub struct CheckConflictsUseCase {
    lock_repo: Arc<dyn crate::engine::ports::LockRepository>,
}

impl CheckConflictsUseCase {
    pub fn new(lock_repo: Arc<dyn crate::engine::ports::LockRepository>) -> Self {
        Self { lock_repo }
    }

    pub async fn execute(&self, req: CheckConflictsRequest) -> Result<CheckConflictsResponse> {
        let all_locks = self.lock_repo.list_locks(&req.workspace_id).await?;

        let conflicts: Vec<ConflictInfo> = all_locks
            .into_iter()
            .filter(|lock| req.file_paths.contains(&lock.file_path))
            .map(|lock| ConflictInfo {
                file_path: lock.file_path,
                locked_by_agent: lock.agent_id.to_string(),
                machine_id: lock.machine_id,
                task_id: lock.task_id.map(|t| t.to_string()),
            })
            .collect();

        let all_clear = conflicts.is_empty();
        Ok(CheckConflictsResponse {
            conflicts,
            all_clear,
        })
    }
}

// ---------------------------------------------------------------------------
// ListAgentsUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ListAgentsRequest {
    pub workspace_id: WorkspaceId,
    pub include_disconnected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListAgentsResponse {
    pub agents: Vec<crate::types::Agent>,
    pub online_count: usize,
}

pub struct ListAgentsUseCase {
    agent_repo: Arc<dyn AgentRepository>,
}

impl ListAgentsUseCase {
    pub fn new(agent_repo: Arc<dyn AgentRepository>) -> Self {
        Self { agent_repo }
    }

    pub async fn execute(&self, req: ListAgentsRequest) -> Result<ListAgentsResponse> {
        let agents = if req.include_disconnected {
            self.agent_repo.list_all(&req.workspace_id).await?
        } else {
            self.agent_repo.list_active(&req.workspace_id).await?
        };

        let online_count = agents
            .iter()
            .filter(|a| !matches!(a.state, crate::types::AgentState::Disconnected))
            .count();

        Ok(ListAgentsResponse {
            agents,
            online_count,
        })
    }
}

// ---------------------------------------------------------------------------
// GetWorkspaceActivityUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GetWorkspaceActivityRequest {
    pub workspace_id: WorkspaceId,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetWorkspaceActivityResponse {
    pub activities: Vec<AgentActivity>,
}

pub struct GetWorkspaceActivityUseCase {
    activity_repo: Arc<dyn ActivityRepository>,
}

impl GetWorkspaceActivityUseCase {
    pub fn new(activity_repo: Arc<dyn ActivityRepository>) -> Self {
        Self { activity_repo }
    }

    pub async fn execute(
        &self,
        req: GetWorkspaceActivityRequest,
    ) -> Result<GetWorkspaceActivityResponse> {
        let activities = self
            .activity_repo
            .get_workspace_stream(&req.workspace_id, req.limit)
            .await?;

        Ok(GetWorkspaceActivityResponse { activities })
    }
}

// ---------------------------------------------------------------------------
// Unit Tests (TDD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ports::{MockActivityRepository, MockAgentRepository, MockTaskRepository};
    use crate::orchestration::bus::WorkspaceEventBus;
    use crate::types::{TaskId, WorkspaceId};
    use mockall::predicate::*;

    fn make_bus() -> WorkspaceEventBus {
        WorkspaceEventBus::new(32)
    }

    // -- CreateTasksUseCase tests --

    #[tokio::test]
    async fn test_create_tasks_success() {
        let mut mock_task = MockTaskRepository::new();
        mock_task.expect_insert().times(2).returning(|_| Ok(()));

        let mut mock_activity = MockActivityRepository::new();
        mock_activity
            .expect_log_activity()
            .times(2)
            .returning(|_| Ok(()));

        let uc = CreateTasksUseCase::new(Arc::new(mock_task), Arc::new(mock_activity), make_bus());

        let req = CreateTasksRequest {
            workspace_id: WorkspaceId::from("ws-1"),
            agent_id: AgentId::new(),
            tasks: vec![
                NewTaskInput {
                    title: "Task A".to_string(),
                    objective: Some("Implement feature X".to_string()),
                    use_cases: None,
                    prerequisites: None,
                    verification_plan: None,
                    required_skills: None,
                    impact_analysis: None,
                    priority: 1,
                    parent_task_id: None,
                },
                NewTaskInput {
                    title: "Task B".to_string(),
                    objective: None,
                    use_cases: None,
                    prerequisites: None,
                    verification_plan: None,
                    required_skills: None,
                    impact_analysis: None,
                    priority: 0,
                    parent_task_id: None,
                },
            ],
        };

        let res = uc.execute(req).await.unwrap();
        assert_eq!(res.created.len(), 2);
        assert_eq!(res.created[0].title, "Task A");
        assert!(matches!(res.created[0].status, TaskStatus::Draft));
    }

    // -- UpdateTaskUseCase tests --

    #[tokio::test]
    async fn test_update_task_gate_violation_completed() {
        let mut mock_task = MockTaskRepository::new();
        mock_task.expect_find_by_id().returning(|_| {
            Ok(Some(Task {
                task_id: TaskId::new(),
                workspace_id: WorkspaceId::from("ws-1"),
                title: "T1".to_string(),
                objective: None,
                status: TaskStatus::InProgress,
                revision: 1,
                rework_cycle: 0,
                assigned_agent_id: None,
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
            }))
        });

        let mock_activity = MockActivityRepository::new();
        let uc = UpdateTaskUseCase::new(Arc::new(mock_task), Arc::new(mock_activity), make_bus());

        let req = UpdateTaskRequest {
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            workspace_id: WorkspaceId::from("ws-1"),
            new_status: Some(TaskStatus::Completed),
            progress_note: None,
        };

        let result = uc.execute(req).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("GATE_VIOLATION"));
        assert!(err.contains("submit_verification"));
    }

    #[tokio::test]
    async fn test_update_task_invalid_transition() {
        let mut mock_task = MockTaskRepository::new();
        mock_task.expect_find_by_id().returning(|_| {
            Ok(Some(Task {
                task_id: TaskId::new(),
                workspace_id: WorkspaceId::from("ws-1"),
                title: "T1".to_string(),
                objective: None,
                status: TaskStatus::Draft,
                revision: 1,
                rework_cycle: 0,
                assigned_agent_id: None,
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
            }))
        });

        let mock_activity = MockActivityRepository::new();
        let uc = UpdateTaskUseCase::new(Arc::new(mock_task), Arc::new(mock_activity), make_bus());

        let req = UpdateTaskRequest {
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            workspace_id: WorkspaceId::from("ws-1"),
            new_status: Some(TaskStatus::CodeReview), // invalid from Draft
            progress_note: None,
        };

        let result = uc.execute(req).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("GATE_VIOLATION"));
    }

    // -- SubmitVerificationUseCase tests --

    #[tokio::test]
    async fn test_submit_verification_requires_test_step() {
        let mut mock_task = MockTaskRepository::new();
        mock_task.expect_find_by_id().returning(|_| {
            Ok(Some(Task {
                task_id: TaskId::new(),
                workspace_id: WorkspaceId::from("ws-1"),
                title: "T1".to_string(),
                objective: None,
                status: TaskStatus::InProgress,
                revision: 1,
                rework_cycle: 0,
                assigned_agent_id: None,
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
            }))
        });

        let mock_activity = MockActivityRepository::new();
        let uc = SubmitVerificationUseCase::new(
            Arc::new(mock_task),
            Arc::new(mock_activity),
            make_bus(),
        );

        // Missing passing test step
        let req = SubmitVerificationRequest {
            task_id: TaskId::new(),
            workspace_id: WorkspaceId::from("ws-1"),
            agent_id: AgentId::new(),
            commit_sha: None,
            steps: serde_json::json!([
                {"kind": "lint", "command": "cargo clippy", "exit_code": 0}
            ]),
        };

        let result = uc.execute(req).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("EVIDENCE_INVALID"));
    }

    #[tokio::test]
    async fn test_submit_verification_success() {
        let mut mock_task = MockTaskRepository::new();
        mock_task.expect_find_by_id().returning(|_| {
            Ok(Some(Task {
                task_id: TaskId::new(),
                workspace_id: WorkspaceId::from("ws-1"),
                title: "T1".to_string(),
                objective: None,
                status: TaskStatus::InProgress,
                revision: 1,
                rework_cycle: 0,
                assigned_agent_id: None,
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
            }))
        });
        mock_task.expect_update().returning(|_| Ok(()));

        let mut mock_activity = MockActivityRepository::new();
        mock_activity.expect_log_activity().returning(|_| Ok(()));

        let uc = SubmitVerificationUseCase::new(
            Arc::new(mock_task),
            Arc::new(mock_activity),
            make_bus(),
        );

        let req = SubmitVerificationRequest {
            task_id: TaskId::new(),
            workspace_id: WorkspaceId::from("ws-1"),
            agent_id: AgentId::new(),
            commit_sha: Some("abc123".to_string()),
            steps: serde_json::json!([
                {"kind": "test", "command": "cargo test", "exit_code": 0, "summary": "42 passed"}
            ]),
        };

        let res = uc.execute(req).await.unwrap();
        assert!(matches!(res.task_status, TaskStatus::Verification));
    }

    // -- CheckConflictsUseCase tests --

    #[tokio::test]
    async fn test_check_conflicts_returns_conflicts() {
        use crate::engine::ports::MockLockRepository;
        use crate::types::FileLock;

        let mut mock_lock = MockLockRepository::new();
        mock_lock.expect_list_locks().returning(|_| {
            Ok(vec![FileLock {
                id: Some(1),
                workspace_id: WorkspaceId::from("ws-1"),
                file_path: "src/main.rs".to_string(),
                agent_id: AgentId::new(),
                machine_id: "machine-1".to_string(),
                task_id: None,
                reason: None,
                locked_at: None,
                expires_at: None,
            }])
        });

        let uc = CheckConflictsUseCase::new(Arc::new(mock_lock));
        let req = CheckConflictsRequest {
            workspace_id: WorkspaceId::from("ws-1"),
            file_paths: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
        };

        let res = uc.execute(req).await.unwrap();
        assert!(!res.all_clear);
        assert_eq!(res.conflicts.len(), 1);
        assert_eq!(res.conflicts[0].file_path, "src/main.rs");
    }

    #[tokio::test]
    async fn test_check_conflicts_all_clear() {
        use crate::engine::ports::MockLockRepository;

        let mut mock_lock = MockLockRepository::new();
        mock_lock.expect_list_locks().returning(|_| Ok(Vec::new()));

        let uc = CheckConflictsUseCase::new(Arc::new(mock_lock));
        let req = CheckConflictsRequest {
            workspace_id: WorkspaceId::from("ws-1"),
            file_paths: vec!["src/lib.rs".to_string()],
        };

        let res = uc.execute(req).await.unwrap();
        assert!(res.all_clear);
        assert!(res.conflicts.is_empty());
    }
}
