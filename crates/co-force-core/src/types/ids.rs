//! Strong-typed identifiers using the Newtype pattern.
//!
//! Prevents ID mismatches at compile time — e.g., passing an `AgentId`
//! where a `TaskId` is expected will not compile.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Macro to generate a strongly-typed identifier wrapping a `String`.
///
/// Each generated type implements: `Debug`, `Clone`, `Serialize`, `Deserialize`,
/// `PartialEq`, `Eq`, `Hash`, `Display`, `From<String>`, `AsRef<str>`.
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
        pub struct $name(pub String);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl $name {
            /// Creates a new identifier with a random UUID v4.
            pub fn new() -> Self {
                Self(uuid::Uuid::new_v4().to_string())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

define_id!(
    /// Unique identifier for an agent within a workspace.
    AgentId
);

define_id!(
    /// Unique identifier for a workspace (usually derived from the project path).
    WorkspaceId
);

define_id!(
    /// Unique identifier for a task.
    TaskId
);

define_id!(
    /// Unique identifier for an agent activity log entry.
    ActivityId
);

define_id!(
    /// Unique identifier for a shared context block.
    ContextId
);

define_id!(
    /// Unique identifier for a memory entry.
    MemoryEntryId
);

define_id!(
    /// Unique identifier for a skill entry.
    SkillId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_id_new_is_unique() {
        let id1 = AgentId::new();
        let id2 = AgentId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_id_from_string() {
        let id = AgentId::from("agent-001".to_string());
        assert_eq!(id.0, "agent-001");
        assert_eq!(id.as_ref(), "agent-001");
    }

    #[test]
    fn test_id_from_str() {
        let id = TaskId::from("task-42");
        assert_eq!(id.to_string(), "task-42");
    }

    #[test]
    fn test_id_display() {
        let id = WorkspaceId::from("ws-main");
        assert_eq!(format!("{id}"), "ws-main");
    }

    #[test]
    fn test_id_serialization_roundtrip() {
        let id = AgentId::from("test-ser");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"test-ser\"");
        let deserialized: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn test_id_equality_and_hash() {
        use std::collections::HashSet;
        let id1 = TaskId::from("t-1");
        let id2 = TaskId::from("t-1");
        let id3 = TaskId::from("t-2");

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);

        let mut set = HashSet::new();
        set.insert(id1.clone());
        assert!(set.contains(&id2));
        assert!(!set.contains(&id3));
    }

    #[test]
    fn test_all_id_types_compile() {
        // Ensures the macro generates all types correctly
        let _ = AgentId::new();
        let _ = WorkspaceId::new();
        let _ = TaskId::new();
        let _ = ActivityId::new();
        let _ = ContextId::new();
        let _ = MemoryEntryId::new();
        let _ = SkillId::new();
    }
}
