//! Domain types for the Co-Force system.
//!
//! This module contains all core data definitions:
//! - **ids**: Strong-typed identifiers (Newtype pattern) preventing ID mismatches.
//! - **enums**: Agent states, task statuses, activity types.
//! - **structs**: Agent, Task, FileLock, AgentActivity, SharedContext,
//!   MemoryEntry, Skill, EmbeddingCacheEntry.

pub mod enums;
pub mod ids;
pub mod structs;

// Re-export everything for ergonomic access via `use co_force_core::types::*`.
pub use enums::*;
pub use ids::*;
pub use structs::*;
