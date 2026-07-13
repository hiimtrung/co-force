//! Memory use cases: store, recall, classify, consolidate, skill management.
//!
//! Implements the Agentic RAG layer (Plan 04).
//!
//! Key behaviors:
//! - `store_memory`: saves even if embedding fails → index_status: "pending"
//! - `recall`: returns SERVICE_UNAVAILABLE if LLM is down (no keyword fallback)
//! - `consolidate`: nightly dedup via cosine similarity > 0.92

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::llm::{
    ollama::LlmProvider,
    vector_search::{bytes_to_embedding, cosine_similarity, embedding_to_bytes, VectorSearch},
};
use crate::types::{MemoryEntryId, WorkspaceId};

// ---------------------------------------------------------------------------
// StoreMemoryUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StoreMemoryRequest {
    pub workspace_id: WorkspaceId,
    pub agent_id: String,
    pub content: String,
    pub entry_type: Option<String>, // None → auto-classify via LLM
    pub tags: Vec<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreMemoryResponse {
    pub entry_id: String,
    pub entry_type: String,
    pub index_status: String, // "indexed" | "pending" | "partial"
    pub message: Option<String>,
}

pub struct StoreMemoryUseCase {
    conn: tokio_rusqlite::Connection,
    llm: Arc<dyn LlmProvider>,
}

impl StoreMemoryUseCase {
    pub fn new(conn: tokio_rusqlite::Connection, llm: Arc<dyn LlmProvider>) -> Self {
        Self { conn, llm }
    }

    pub async fn execute(&self, req: StoreMemoryRequest) -> Result<StoreMemoryResponse> {
        let entry_id = MemoryEntryId::new().to_string();
        let now = Utc::now().to_rfc3339();

        // Classify if type not provided
        let (entry_type, _confidence) = if let Some(t) = req.entry_type {
            (t, 1.0f32)
        } else {
            match self.llm.classify(&req.content).await {
                Ok((cat, conf)) => (cat.to_lowercase(), conf),
                Err(_) => ("memory".to_string(), 0.5),
            }
        };

        // Try to get embedding — non-fatal if it fails (F-19)
        let (embedding_bytes, index_status, message) = match self.llm.embed(&req.content).await {
            Ok(emb) => (Some(embedding_to_bytes(&emb)), "indexed".to_string(), None),
            Err(e) => (
                None,
                "pending".to_string(),
                Some(format!(
                    "Embedding failed — entry saved without vector index. \
                         Will be re-embedded when LLM recovers. Reason: {e}"
                )),
            ),
        };

        let tags_json = serde_json::to_string(&req.tags).unwrap_or_else(|_| "[]".to_string());
        let eid = entry_id.clone();
        let etype = entry_type.clone();
        let content = req.content.clone();
        let source = req.source.clone();
        let agent_id = req.agent_id.clone();
        let ws_id = req.workspace_id.to_string();

        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO memory_entries \
                     (entry_id, workspace_id, entry_type, content, source, agent_id, \
                      confidence, tags, embedding, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        eid,
                        ws_id,
                        etype,
                        content,
                        source,
                        agent_id,
                        1.0f64,
                        tags_json,
                        embedding_bytes,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to store memory: {e}"))?;

        Ok(StoreMemoryResponse {
            entry_id,
            entry_type,
            index_status,
            message,
        })
    }
}

// ---------------------------------------------------------------------------
// RecallUseCase
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RecallRequest {
    pub workspace_id: WorkspaceId,
    pub query: String,
    pub top_k: usize,
    pub min_score: f32,
    pub type_filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResponse {
    pub results: Vec<RecallResult>,
    pub index_status: Option<String>, // Set if PARTIAL_INDEX
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResult {
    pub entry_id: String,
    pub content: String,
    pub entry_type: String,
    pub similarity: f32,
    pub tags: Vec<String>,
}

pub struct RecallUseCase {
    vector_search: Arc<dyn VectorSearch>,
    llm: Arc<dyn LlmProvider>,
    conn: tokio_rusqlite::Connection,
}

impl RecallUseCase {
    pub fn new(
        vector_search: Arc<dyn VectorSearch>,
        llm: Arc<dyn LlmProvider>,
        conn: tokio_rusqlite::Connection,
    ) -> Self {
        Self {
            vector_search,
            llm,
            conn,
        }
    }

    pub async fn execute(&self, req: RecallRequest) -> Result<RecallResponse> {
        // Check for pending embeddings first
        let ws_id = req.workspace_id.to_string();
        let pending_count: i64 = self
            .conn
            .call(move |conn| {
                let count: i64 = conn
                    .query_row(
                        "SELECT count(*) FROM memory_entries \
                         WHERE workspace_id = ?1 AND embedding IS NULL",
                        [ws_id],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                Ok(count)
            })
            .await
            .unwrap_or(0);

        // Embed the query — returns SERVICE_UNAVAILABLE if LLM is down (N2)
        let query_embedding = self.llm.embed(&req.query).await.map_err(|e| {
            anyhow::anyhow!(
                "SERVICE_UNAVAILABLE: Cannot perform recall — LLM is required to embed \
                 the query. Retry after LLM recovers. Error: {e}"
            )
        })?;

        let mut results = self
            .vector_search
            .search(
                &query_embedding,
                &req.workspace_id.to_string(),
                req.top_k,
                req.min_score,
            )
            .await?;

        // Apply type filter
        if let Some(type_filter) = &req.type_filter {
            results.retain(|r| r.entry_type == *type_filter);
        }

        let index_status = if pending_count > 0 {
            Some(format!(
                "PARTIAL_INDEX: {} entries are awaiting embedding. \
                 Results may be incomplete.",
                pending_count
            ))
        } else {
            None
        };

        Ok(RecallResponse {
            results: results
                .into_iter()
                .map(|r| RecallResult {
                    entry_id: r.entry_id,
                    content: r.content,
                    entry_type: r.entry_type,
                    similarity: r.similarity,
                    tags: Vec::new(), // Could fetch from DB if needed
                })
                .collect(),
            index_status,
        })
    }
}

// ---------------------------------------------------------------------------
// ConsolidateMemoryUseCase (nightly dedup)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidateResponse {
    pub deduped_count: usize,
    pub entries_processed: usize,
}

pub struct ConsolidateMemoryUseCase {
    conn: tokio_rusqlite::Connection,
}

impl ConsolidateMemoryUseCase {
    pub fn new(conn: tokio_rusqlite::Connection) -> Self {
        Self { conn }
    }

    /// Deduplicates entries with cosine similarity > 0.92.
    /// Keeps the most recently accessed entry.
    pub async fn execute(&self, workspace_id: &WorkspaceId) -> Result<ConsolidateResponse> {
        let ws_id = workspace_id.to_string();
        let similarity_threshold = 0.92f32;

        // Load all entries with embeddings
        let entries: Vec<(String, Vec<u8>)> = self
            .conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT entry_id, embedding FROM memory_entries \
                     WHERE workspace_id = ?1 AND embedding IS NOT NULL \
                     ORDER BY accessed_at DESC NULLS LAST",
                )?;
                let rows = stmt.query_map([ws_id], |row| {
                    let entry_id: String = row.get(0)?;
                    let emb: Vec<u8> = row.get(1)?;
                    Ok((entry_id, emb))
                })?;
                Ok(rows.collect::<Result<Vec<_>, rusqlite::Error>>()?)
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load entries for consolidation: {e}"))?;

        let entries_processed = entries.len();
        let mut to_delete: Vec<String> = Vec::new();
        let mut kept: Vec<(String, Vec<f32>)> = Vec::new();

        for (entry_id, emb_bytes) in entries {
            if to_delete.contains(&entry_id) {
                continue; // Already marked for deletion
            }

            let embedding = bytes_to_embedding(&emb_bytes);
            let mut is_duplicate = false;

            for (kept_id, kept_emb) in &kept {
                let sim = cosine_similarity(&embedding, kept_emb);
                if sim > similarity_threshold {
                    // This entry is a near-duplicate of a kept entry
                    to_delete.push(entry_id.clone());
                    is_duplicate = true;
                    tracing::debug!(
                        "Deduplicating entry {} (similar to {}, sim={:.3})",
                        entry_id,
                        kept_id,
                        sim
                    );
                    break;
                }
            }

            if !is_duplicate {
                kept.push((entry_id, embedding));
            }
        }

        // Delete near-duplicates
        let deduped_count = to_delete.len();
        if !to_delete.is_empty() {
            let ws_id2 = workspace_id.to_string();
            let to_delete_clone = to_delete.clone();
            self.conn
                .call(move |conn| {
                    let placeholders: String = to_delete_clone
                        .iter()
                        .enumerate()
                        .map(|(i, _)| format!("?{}", i + 2))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let sql = format!(
                        "DELETE FROM memory_entries WHERE workspace_id = ?1 AND entry_id IN ({placeholders})"
                    );
                    let mut params: Vec<Box<dyn rusqlite::ToSql>> =
                        vec![Box::new(ws_id2)];
                    for id in &to_delete_clone {
                        params.push(Box::new(id.clone()));
                    }
                    conn.execute(
                        &sql,
                        rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                    )?;
                    Ok(())
                })
                .await
                .map_err(|e| anyhow::anyhow!("Failed to delete duplicates: {e}"))?;
        }

        Ok(ConsolidateResponse {
            deduped_count,
            entries_processed,
        })
    }
}

// ---------------------------------------------------------------------------
// Unit Tests (TDD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ollama::MockLlmProvider, vector_search::MockVectorSearch};

    fn make_mock_llm_with_embed() -> MockLlmProvider {
        let mut mock = MockLlmProvider::new();
        mock.expect_embed().returning(|_| Ok(vec![0.1, 0.2, 0.3]));
        mock.expect_classify()
            .returning(|_| Ok(("knowledge".to_string(), 0.9)));
        mock
    }

    #[tokio::test]
    async fn test_store_memory_succeeds_even_when_embed_fails() {
        // Open in-memory DB and migrate
        let db = crate::db::Database::open_in_memory().await.unwrap();

        let mut mock_llm = MockLlmProvider::new();
        mock_llm
            .expect_embed()
            .returning(|_| Err(anyhow::anyhow!("Ollama is down")));
        mock_llm
            .expect_classify()
            .returning(|_| Ok(("memory".to_string(), 0.8)));

        let uc = StoreMemoryUseCase::new(db.conn().clone(), Arc::new(mock_llm));

        let req = StoreMemoryRequest {
            workspace_id: WorkspaceId::from("ws-1"),
            agent_id: "agent-1".to_string(),
            content: "auth.ts has a null pointer bug on line 42".to_string(),
            entry_type: None,
            tags: vec!["bug".to_string()],
            source: None,
        };

        let res = uc.execute(req).await.unwrap();
        assert_eq!(res.index_status, "pending");
        assert!(res.message.is_some());
        assert!(!res.entry_id.is_empty());
    }

    #[tokio::test]
    async fn test_store_memory_indexes_when_llm_available() {
        let db = crate::db::Database::open_in_memory().await.unwrap();
        let uc = StoreMemoryUseCase::new(db.conn().clone(), Arc::new(make_mock_llm_with_embed()));

        let req = StoreMemoryRequest {
            workspace_id: WorkspaceId::from("ws-1"),
            agent_id: "agent-1".to_string(),
            content: "Always use PostgreSQL for relational data".to_string(),
            entry_type: Some("knowledge".to_string()),
            tags: vec!["db".to_string()],
            source: None,
        };

        let res = uc.execute(req).await.unwrap();
        assert_eq!(res.index_status, "indexed");
        assert!(res.message.is_none());
    }

    #[tokio::test]
    async fn test_recall_returns_service_unavailable_when_llm_down() {
        let db = crate::db::Database::open_in_memory().await.unwrap();

        let mut mock_llm = MockLlmProvider::new();
        mock_llm
            .expect_embed()
            .returning(|_| Err(anyhow::anyhow!("Ollama is down")));

        let mut mock_vs = MockVectorSearch::new();
        // Search should NOT be called when LLM is down
        mock_vs.expect_search().times(0);

        let uc = RecallUseCase::new(Arc::new(mock_vs), Arc::new(mock_llm), db.conn().clone());

        let req = RecallRequest {
            workspace_id: WorkspaceId::from("ws-1"),
            query: "postgres bug".to_string(),
            top_k: 5,
            min_score: 0.7,
            type_filter: None,
        };

        let result = uc.execute(req).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("SERVICE_UNAVAILABLE"),
            "Expected SERVICE_UNAVAILABLE, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_recall_returns_results_when_available() {
        let db = crate::db::Database::open_in_memory().await.unwrap();

        let mut mock_llm = MockLlmProvider::new();
        mock_llm
            .expect_embed()
            .returning(|_| Ok(vec![0.1, 0.2, 0.3]));

        let mut mock_vs = MockVectorSearch::new();
        mock_vs.expect_search().returning(|_, _, _, _| {
            Ok(vec![crate::llm::vector_search::SearchResult {
                entry_id: "e1".to_string(),
                similarity: 0.95,
                content: "PostgreSQL best practices".to_string(),
                entry_type: "knowledge".to_string(),
                parent_id: None,
            }])
        });

        let uc = RecallUseCase::new(Arc::new(mock_vs), Arc::new(mock_llm), db.conn().clone());

        let req = RecallRequest {
            workspace_id: WorkspaceId::from("ws-1"),
            query: "database".to_string(),
            top_k: 5,
            min_score: 0.7,
            type_filter: None,
        };

        let res = uc.execute(req).await.unwrap();
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].entry_id, "e1");
        assert!(res.results[0].similarity > 0.9);
    }
}
