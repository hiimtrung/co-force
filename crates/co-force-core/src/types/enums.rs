//! Core domain enums for agent states, task statuses, and activity types.
//!
//! These enums are serialized with `snake_case` to match the SQLite schema
//! column values.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Represents the current lifecycle state of an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Agent is checked in but not actively working on a task.
    Idle,
    /// Agent is actively working on a task.
    Working,
    /// Agent's work is paused (e.g., awaiting review).
    Paused,
    /// Agent session has dropped; grace period may be active.
    Disconnected,
}

impl fmt::Display for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Working => write!(f, "working"),
            Self::Paused => write!(f, "paused"),
            Self::Disconnected => write!(f, "disconnected"),
        }
    }
}

impl AgentState {
    /// Parses a state from its string representation (as stored in SQLite).
    pub fn from_str_value(s: &str) -> Option<Self> {
        match s {
            "idle" => Some(Self::Idle),
            "working" => Some(Self::Working),
            "paused" => Some(Self::Paused),
            "disconnected" => Some(Self::Disconnected),
            _ => None,
        }
    }
}

/// Task lifecycle states per the state machine in Plan 07 §3.
///
/// Flow: `Draft → SpecReview → AwaitingApproval → Approved → InProgress
///        → Verification → CodeReview → Completed`
///
/// Rework loop: `CodeReview → Rework → InProgress`
/// Escalation: `Blocked`, `PendingHandover`, `Cancelled` are exit states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task has been created but not yet reviewed.
    Draft,
    /// Server/reasoner is reviewing the task spec.
    SpecReview,
    /// Task spec approved by server, awaiting user approval.
    AwaitingApproval,
    /// User has approved — ready to be assigned and worked on.
    Approved,
    /// An agent is actively working on this task.
    InProgress,
    /// Agent has submitted verification evidence.
    Verification,
    /// Under code review by a reviewer agent.
    CodeReview,
    /// Reviewer requested changes — back to development.
    Rework,
    /// All gates passed, task is done.
    Completed,
    /// Task is blocked by external factors.
    Blocked,
    /// Agent is handing over to another (context limit / rate limit).
    PendingHandover,
    /// Task was cancelled.
    Cancelled,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| format!("{self:?}"));
        write!(f, "{s}")
    }
}

impl TaskStatus {
    /// Parses a status from its string representation (as stored in SQLite).
    pub fn from_str_value(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }

    /// Returns `true` if this status represents a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

/// Types of agent activities logged in the `agent_activities` table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType {
    /// Agent checked in to the workspace.
    CheckIn,
    /// Agent started working on a task.
    TaskStarted,
    /// Agent completed a task.
    TaskCompleted,
    /// Agent edited a file.
    FileEdited,
    /// Agent stored a memory entry.
    MemoryStored,
    /// Agent delegated a task to another agent.
    Delegation,
    /// Agent acquired a file lock.
    LockAcquired,
    /// Agent shared context with another agent.
    ContextShared,
}

impl fmt::Display for ActivityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| format!("{self:?}"));
        write!(f, "{s}")
    }
}

impl ActivityType {
    /// Parses from its string representation.
    pub fn from_str_value(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- AgentState tests --

    #[test]
    fn test_agent_state_serialize_snake_case() {
        let json = serde_json::to_string(&AgentState::Idle).unwrap();
        assert_eq!(json, "\"idle\"");

        let json = serde_json::to_string(&AgentState::Working).unwrap();
        assert_eq!(json, "\"working\"");

        let json = serde_json::to_string(&AgentState::Disconnected).unwrap();
        assert_eq!(json, "\"disconnected\"");
    }

    #[test]
    fn test_agent_state_deserialize() {
        let state: AgentState = serde_json::from_str("\"paused\"").unwrap();
        assert_eq!(state, AgentState::Paused);
    }

    #[test]
    fn test_agent_state_display() {
        assert_eq!(AgentState::Idle.to_string(), "idle");
        assert_eq!(AgentState::Working.to_string(), "working");
    }

    #[test]
    fn test_agent_state_from_str_value() {
        assert_eq!(AgentState::from_str_value("idle"), Some(AgentState::Idle));
        assert_eq!(AgentState::from_str_value("unknown"), None);
    }

    // -- TaskStatus tests --

    #[test]
    fn test_task_status_serialize_all_variants() {
        let variants = vec![
            (TaskStatus::Draft, "draft"),
            (TaskStatus::SpecReview, "spec_review"),
            (TaskStatus::AwaitingApproval, "awaiting_approval"),
            (TaskStatus::Approved, "approved"),
            (TaskStatus::InProgress, "in_progress"),
            (TaskStatus::Verification, "verification"),
            (TaskStatus::CodeReview, "code_review"),
            (TaskStatus::Rework, "rework"),
            (TaskStatus::Completed, "completed"),
            (TaskStatus::Blocked, "blocked"),
            (TaskStatus::PendingHandover, "pending_handover"),
            (TaskStatus::Cancelled, "cancelled"),
        ];

        for (status, expected) in variants {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, format!("\"{expected}\""), "Failed for {status:?}");
        }
    }

    #[test]
    fn test_task_status_roundtrip() {
        let original = TaskStatus::InProgress;
        let json = serde_json::to_string(&original).unwrap();
        let restored: TaskStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn test_task_status_is_terminal() {
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
        assert!(!TaskStatus::InProgress.is_terminal());
        assert!(!TaskStatus::Draft.is_terminal());
        assert!(!TaskStatus::Rework.is_terminal());
    }

    #[test]
    fn test_task_status_from_str_value() {
        assert_eq!(
            TaskStatus::from_str_value("in_progress"),
            Some(TaskStatus::InProgress)
        );
        assert_eq!(
            TaskStatus::from_str_value("code_review"),
            Some(TaskStatus::CodeReview)
        );
        assert_eq!(TaskStatus::from_str_value("invalid"), None);
    }

    // -- ActivityType tests --

    #[test]
    fn test_activity_type_serialize() {
        let json = serde_json::to_string(&ActivityType::CheckIn).unwrap();
        assert_eq!(json, "\"check_in\"");

        let json = serde_json::to_string(&ActivityType::LockAcquired).unwrap();
        assert_eq!(json, "\"lock_acquired\"");
    }

    #[test]
    fn test_activity_type_roundtrip() {
        let original = ActivityType::ContextShared;
        let json = serde_json::to_string(&original).unwrap();
        let restored: ActivityType = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn test_activity_type_from_str_value() {
        assert_eq!(
            ActivityType::from_str_value("task_started"),
            Some(ActivityType::TaskStarted)
        );
        assert_eq!(ActivityType::from_str_value("nope"), None);
    }
}
