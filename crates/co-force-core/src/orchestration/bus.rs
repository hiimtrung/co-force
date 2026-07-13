//! Global in-memory Event Bus for Workspace events.

use tokio::sync::broadcast;

/// Core events emitted by the platform and monitored by orchestration tasks.
#[derive(Debug, Clone)]
pub enum WorkspaceEvent {
    AgentCheckedIn {
        agent_id: String,
        workspace_id: String,
    },
    FilesLocked {
        agent_id: String,
        files: Vec<String>,
    },
    TaskUpdated {
        task_id: String,
        new_status: String,
    },
    ActivityLogged {
        activity_id: String,
    },
    ContextShared {
        context_id: String,
    },
    HandoverRequested {
        old_agent_id: String,
        task_id: String,
        next_provider: String,
    },
}

/// A wrapper around a tokio broadcast channel to decouple use cases and orchestration tasks.
#[derive(Clone)]
pub struct WorkspaceEventBus {
    tx: broadcast::Sender<WorkspaceEvent>,
}

impl WorkspaceEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Subscribe to the event bus.
    pub fn subscribe(&self) -> broadcast::Receiver<WorkspaceEvent> {
        self.tx.subscribe()
    }

    /// Publish an event to the bus.
    pub fn send(&self, event: WorkspaceEvent) {
        let _ = self.tx.send(event);
    }
}
