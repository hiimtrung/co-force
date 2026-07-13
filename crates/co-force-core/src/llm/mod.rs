//! LLM Provider trait and implementations for Co-Force.
//!
//! Supports 3 model roles: embedding, classifier, reasoner.
//! Default implementation uses Ollama (local, private, mandatory per N2).
//!
//! Key decisions:
//! - Embeddings stored as BLOB in SQLite — no vector DB dependency
//! - /api/embed endpoint (not deprecated /api/embeddings)
//! - No silent fallback: if LLM is down, return explicit errors

pub mod chunker;
pub mod memory;
pub mod ollama;
pub mod skills;
pub mod vector_search;

pub use chunker::{agentic_chunking, Chunk};
pub use memory::{
    ConsolidateMemoryUseCase, ConsolidateResponse, RecallRequest, RecallResponse, RecallResult,
    RecallUseCase, StoreMemoryRequest, StoreMemoryResponse, StoreMemoryUseCase,
};
pub use ollama::{LlmProvider, OllamaProvider};
pub use skills::{
    CreateSkillRequest, CreateSkillResponse, CreateSkillUseCase, GetSkillRequest, GetSkillResponse,
    GetSkillUseCase, ListSkillsRequest, ListSkillsResponse, ListSkillsUseCase, SkillSummary,
};
pub use vector_search::{BruteForceCosine, SearchResult, VectorSearch};
