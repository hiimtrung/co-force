//! Core domain structs for the Co-Force system.
//!
//! These structs map to SQLite tables defined in `db/migrations/001_initial.sql`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::enums::{ActivityType, AgentState, TaskStatus};
use super::ids::{ActivityId, AgentId, ContextId, MemoryEntryId, SkillId, TaskId, WorkspaceId};

// ---------------------------------------------------------------------------
// Agent
// ---------------------------------------------------------------------------

/// An AI agent registered in a workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub agent_id: AgentId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub role: String,
    pub provider: Option<String>,
    pub machine_id: String,
    pub state: AgentState,
    pub current_task_id: Option<TaskId>,
    pub last_seen: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

/// A unit of work tracked through the quality-gate state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub objective: Option<String>,
    pub status: TaskStatus,
    /// Increments based on server-observed events (F-21).
    pub revision: i64,
    /// Escalates if it exceeds the rework cycle limit (Plan 07 §3).
    pub rework_cycle: i64,
    pub assigned_agent_id: Option<AgentId>,
    pub delegated_from_agent_id: Option<AgentId>,
    pub parent_task_id: Option<TaskId>,
    /// JSON array of use case descriptions.
    pub use_cases: Option<serde_json::Value>,
    /// JSON array of prerequisites.
    pub prerequisites: Option<serde_json::Value>,
    /// JSON verification plan.
    pub verification_plan: Option<serde_json::Value>,
    /// JSON array of required skills.
    pub required_skills: Option<serde_json::Value>,
    /// JSON array of locked file paths.
    pub locked_files: Option<serde_json::Value>,
    /// JSON impact analysis object.
    pub impact_analysis: Option<serde_json::Value>,
    pub priority: i64,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// FileLock
// ---------------------------------------------------------------------------

/// An exclusive lock on a file path within a workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLock {
    pub id: Option<i64>,
    pub workspace_id: WorkspaceId,
    pub file_path: String,
    pub agent_id: AgentId,
    pub machine_id: String,
    pub task_id: Option<TaskId>,
    pub reason: Option<String>,
    pub locked_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// AgentActivity
// ---------------------------------------------------------------------------

/// An append-only log entry recording agent actions in the workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentActivity {
    pub activity_id: ActivityId,
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub activity_type: ActivityType,
    /// JSON content — typically `{summary, details}`.
    pub content: Option<serde_json::Value>,
    pub related_task_id: Option<TaskId>,
    /// JSON array of related file paths.
    pub related_files: Option<Vec<String>>,
    pub version: i64,
    pub occurred_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// SharedContext
// ---------------------------------------------------------------------------

/// A context block shared between agents (lazy resolution).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedContext {
    pub context_id: ContextId,
    pub workspace_id: WorkspaceId,
    pub source_agent_id: AgentId,
    pub target_agent_id: Option<AgentId>,
    pub context_type: String,
    pub content: serde_json::Value,
    pub resolved: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// MemoryEntry
// ---------------------------------------------------------------------------

/// A memory/knowledge/skill entry stored in the workspace knowledge base.
/// Embedding vector stored as a BLOB in SQLite (decision F-02).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub entry_id: MemoryEntryId,
    pub workspace_id: WorkspaceId,
    /// One of: `"memory"`, `"knowledge"`, `"skill"`.
    pub entry_type: String,
    pub content: String,
    pub source: Option<String>,
    pub agent_id: Option<AgentId>,
    pub confidence: f64,
    /// JSON array of tags.
    pub tags: Option<serde_json::Value>,
    /// Vector embedding stored as bytes (F-02: BLOB in SQLite).
    #[serde(skip)]
    pub embedding: Option<Vec<u8>>,
    pub created_at: Option<DateTime<Utc>>,
    pub accessed_at: Option<DateTime<Utc>>,
    pub access_count: i64,
}

// ---------------------------------------------------------------------------
// Skill
// ---------------------------------------------------------------------------

/// A reified skill derived from accumulated memory entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub skill_id: SkillId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    /// JSON array of skill steps.
    pub steps: Option<serde_json::Value>,
    /// JSON array of `entry_id`s linking to source memories.
    pub source_memories: Option<serde_json::Value>,
    pub usage_count: i64,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// EmbeddingCache
// ---------------------------------------------------------------------------

/// Cached embedding keyed by SHA-256 content hash, avoiding redundant
/// calls to the embedding model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingCacheEntry {
    pub content_hash: String,
    #[serde(skip)]
    pub embedding: Vec<u8>,
    pub created_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Handover & ProviderStatus
// ---------------------------------------------------------------------------

/// Context handover packet between agents when limits or rate limits are reached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handover {
    pub handover_id: String,
    pub task_id: TaskId,
    pub from_agent_id: AgentId,
    pub to_agent_id: Option<AgentId>,
    pub reason: String,
    pub package: serde_json::Value,
    pub provider_cooldown_until: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub accepted_at: Option<DateTime<Utc>>,
}

/// Dynamic provider status per machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub machine_id: String,
    pub provider: String,
    pub rate_limited_until: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent() -> Agent {
        Agent {
            agent_id: AgentId::from("agent-1"),
            workspace_id: WorkspaceId::from("ws-main"),
            name: "Claude".to_string(),
            role: "developer".to_string(),
            provider: Some("claude".to_string()),
            machine_id: "machine-abc".to_string(),
            state: AgentState::Idle,
            current_task_id: None,
            last_seen: None,
            created_at: None,
        }
    }

    fn sample_task() -> Task {
        Task {
            task_id: TaskId::from("task-1"),
            workspace_id: WorkspaceId::from("ws-main"),
            title: "Implement auth".to_string(),
            objective: Some("Add bearer token auth".to_string()),
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
        }
    }

    #[test]
    fn test_agent_serialization_roundtrip() {
        let agent = sample_agent();
        let json = serde_json::to_string(&agent).unwrap();
        let restored: Agent = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.agent_id, agent.agent_id);
        assert_eq!(restored.name, "Claude");
        assert_eq!(restored.state, AgentState::Idle);
    }

    #[test]
    fn test_task_serialization_roundtrip() {
        let task = sample_task();
        let json = serde_json::to_string(&task).unwrap();
        let restored: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.task_id, task.task_id);
        assert_eq!(restored.status, TaskStatus::Draft);
        assert_eq!(restored.revision, 1);
    }

    #[test]
    fn test_agent_state_in_json() {
        let agent = sample_agent();
        let json = serde_json::to_string(&agent).unwrap();
        // State should be serialized as snake_case
        assert!(json.contains("\"idle\""));
    }

    #[test]
    fn test_task_status_in_json() {
        let mut task = sample_task();
        task.status = TaskStatus::InProgress;
        let json = serde_json::to_string(&task).unwrap();
        assert!(json.contains("\"in_progress\""));
    }

    #[test]
    fn test_activity_serialization() {
        let activity = AgentActivity {
            activity_id: ActivityId::from("act-1"),
            workspace_id: WorkspaceId::from("ws-1"),
            agent_id: AgentId::from("agent-1"),
            activity_type: ActivityType::CheckIn,
            content: Some(serde_json::json!({"summary": "Checked in"})),
            related_task_id: None,
            related_files: Some(vec!["src/main.rs".to_string()]),
            version: 1,
            occurred_at: Utc::now(),
        };
        let json = serde_json::to_string(&activity).unwrap();
        let restored: AgentActivity = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.activity_type, ActivityType::CheckIn);
        assert_eq!(restored.related_files.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_shared_context_serialization() {
        let ctx = SharedContext {
            context_id: ContextId::from("ctx-1"),
            workspace_id: WorkspaceId::from("ws-1"),
            source_agent_id: AgentId::from("agent-a"),
            target_agent_id: Some(AgentId::from("agent-b")),
            context_type: "handover".to_string(),
            content: serde_json::json!({"notes": "Take over auth task"}),
            resolved: false,
            created_at: None,
            resolved_at: None,
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let restored: SharedContext = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.context_type, "handover");
        assert!(!restored.resolved);
    }

    #[test]
    fn test_file_lock_serialization() {
        let lock = FileLock {
            id: Some(1),
            workspace_id: WorkspaceId::from("ws-1"),
            file_path: "src/lib.rs".to_string(),
            agent_id: AgentId::from("agent-1"),
            machine_id: "machine-1".to_string(),
            task_id: Some(TaskId::from("task-1")),
            reason: Some("Editing module structure".to_string()),
            locked_at: None,
            expires_at: None,
        };
        let json = serde_json::to_string(&lock).unwrap();
        let restored: FileLock = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.file_path, "src/lib.rs");
    }

    #[test]
    fn test_memory_entry_embedding_skipped_in_json() {
        let entry = MemoryEntry {
            entry_id: MemoryEntryId::from("mem-1"),
            workspace_id: WorkspaceId::from("ws-1"),
            entry_type: "memory".to_string(),
            content: "Rust uses zero-cost abstractions".to_string(),
            source: None,
            agent_id: None,
            confidence: 0.95,
            tags: Some(serde_json::json!(["rust", "concepts"])),
            embedding: Some(vec![1, 2, 3, 4]),
            created_at: None,
            accessed_at: None,
            access_count: 0,
        };
        let json = serde_json::to_string(&entry).unwrap();
        // Embedding field should be skipped in serialization
        assert!(!json.contains("embedding"));
        assert!(json.contains("\"memory\""));
    }

    #[test]
    fn test_skill_serialization() {
        let skill = Skill {
            skill_id: SkillId::from("skill-1"),
            workspace_id: WorkspaceId::from("ws-1"),
            name: "Docker Deployment".to_string(),
            description: Some("How to deploy with Docker".to_string()),
            category: Some("devops".to_string()),
            steps: Some(serde_json::json!([
                "Build image",
                "Push to registry",
                "Deploy"
            ])),
            source_memories: Some(serde_json::json!(["mem-1", "mem-2"])),
            usage_count: 5,
            created_at: None,
            updated_at: None,
        };
        let json = serde_json::to_string(&skill).unwrap();
        let restored: Skill = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, "Docker Deployment");
        assert_eq!(restored.usage_count, 5);
    }
}
