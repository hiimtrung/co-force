//! Engine module — use cases and port definitions.

pub mod check_in;
pub mod get_agent_context;
pub mod handover;
pub mod lock_files;
pub mod ports;
pub mod share_context;
pub mod spawn;
pub mod tasks;

pub use check_in::{derive_workspace_id, CheckInRequest, CheckInResponse, CheckInUseCase};
pub use get_agent_context::{
    GetAgentContextRequest, GetAgentContextResponse, GetAgentContextUseCase,
};
pub use handover::{HandoverRequest, HandoverResponse, HandoverUseCase};
pub use lock_files::{LockFilesRequest, LockFilesResponse, LockFilesUseCase};
pub use share_context::{ShareContextRequest, ShareContextResponse, ShareContextUseCase};
pub use spawn::{SpawnRequest, SpawnResponse, SpawnUseCase};
pub use tasks::{
    ApproveTasksRequest, ApproveTasksResponse, ApproveTasksUseCase, CheckConflictsRequest,
    CheckConflictsResponse, CheckConflictsUseCase, CreateTasksRequest, CreateTasksResponse,
    CreateTasksUseCase, DelegateTaskRequest, DelegateTaskResponse, DelegateTaskUseCase,
    GetWorkspaceActivityRequest, GetWorkspaceActivityResponse, GetWorkspaceActivityUseCase,
    ListAgentsRequest, ListAgentsResponse, ListAgentsUseCase, ListTasksRequest, ListTasksResponse,
    ListTasksUseCase, NewTaskInput, SubmitVerificationRequest, SubmitVerificationResponse,
    SubmitVerificationUseCase, UnlockFilesRequest, UnlockFilesResponse, UnlockFilesUseCase,
    UpdateTaskRequest, UpdateTaskResponse, UpdateTaskUseCase,
};
