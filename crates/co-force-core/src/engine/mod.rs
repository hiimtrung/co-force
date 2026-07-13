//! Engine module — use cases and port definitions.

pub mod check_in;
pub mod get_agent_context;
pub mod lock_files;
pub mod ports;
pub mod share_context;
pub mod handover;
pub mod spawn;

pub use check_in::{derive_workspace_id, CheckInRequest, CheckInResponse, CheckInUseCase};
pub use get_agent_context::{
    GetAgentContextRequest, GetAgentContextResponse, GetAgentContextUseCase,
};
pub use lock_files::{LockFilesRequest, LockFilesResponse, LockFilesUseCase};
pub use share_context::{ShareContextRequest, ShareContextResponse, ShareContextUseCase};
pub use handover::{HandoverRequest, HandoverResponse, HandoverUseCase};
pub use spawn::{SpawnRequest, SpawnResponse, SpawnUseCase};
