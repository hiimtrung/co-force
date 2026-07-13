//! Brute-force cosine similarity vector search (Plan 04 §5, F-02).
//!
//! Decision: embeddings are stored as BLOBs in SQLite `memory_entries`.
//! Search is done in-Rust via cosine similarity — no external vector DB.
//! Performance: < 10ms for ~5000 entries × 1024 dimensions.
//!
//! Upgrade path: wrap behind `VectorSearch` trait → swap to sqlite-vec when
//! workspace exceeds ~50k entries (same DB file, zero migration of data).

use anyhow::Result;
use async_trait::async_trait;

// ---------------------------------------------------------------------------
// VectorSearch trait (Ports layer)
// ---------------------------------------------------------------------------

/// Results from a vector similarity search.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entry_id: String,
    pub similarity: f32,
    pub content: String,
    pub entry_type: String,
    pub parent_id: Option<String>,
}

/// Trait for searching similar embeddings.
///
/// `BruteForceCosine` is the default implementation.
/// Future: `SqliteVecSearch` when workspaces exceed ~50k entries.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait VectorSearch: Send + Sync {
    /// Finds the `top_k` most similar entries for the given query embedding.
    ///
    /// Returns results sorted by similarity descending.
    /// Only returns entries with similarity >= `min_score`.
    async fn search(
        &self,
        query_embedding: &[f32],
        workspace_id: &str,
        top_k: usize,
        min_score: f32,
    ) -> Result<Vec<SearchResult>>;
}

// ---------------------------------------------------------------------------
// Cosine similarity helper
// ---------------------------------------------------------------------------

/// Computes the cosine similarity between two vectors.
///
/// Returns a value in [−1, 1]; values close to 1.0 indicate high similarity.
/// Returns 0.0 if either vector has zero magnitude.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Converts a Vec<f32> to raw bytes for BLOB storage.
pub fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Converts raw bytes from BLOB storage back to Vec<f32>.
pub fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

// ---------------------------------------------------------------------------
// BruteForceCosine implementation
// ---------------------------------------------------------------------------

/// In-memory brute-force cosine similarity search.
///
/// Loads all embeddings from `memory_entries` via the provided connection
/// and computes similarity in Rust. Adequate for < 50k entries.
pub struct BruteForceCosine {
    conn: tokio_rusqlite::Connection,
}

impl BruteForceCosine {
    pub fn new(conn: tokio_rusqlite::Connection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl VectorSearch for BruteForceCosine {
    async fn search(
        &self,
        query_embedding: &[f32],
        workspace_id: &str,
        top_k: usize,
        min_score: f32,
    ) -> Result<Vec<SearchResult>> {
        let ws_id = workspace_id.to_string();
        let query_embedding = query_embedding.to_vec();

        let results = self
            .conn
            .call(move |conn| {
                // Load all entries with embeddings for this workspace
                let mut stmt = conn.prepare(
                    "SELECT entry_id, content, entry_type, embedding \
                     FROM memory_entries \
                     WHERE workspace_id = ?1 AND embedding IS NOT NULL",
                )?;

                let rows = stmt.query_map([&ws_id], |row| {
                    let entry_id: String = row.get(0)?;
                    let content: String = row.get(1)?;
                    let entry_type: String = row.get(2)?;
                    let embedding_bytes: Vec<u8> = row.get(3)?;
                    Ok((entry_id, content, entry_type, embedding_bytes))
                })?;

                let mut scored: Vec<(f32, SearchResult)> = Vec::new();

                for row in rows {
                    let (entry_id, content, entry_type, emb_bytes) = row?;
                    let entry_embedding = bytes_to_embedding(&emb_bytes);
                    let score = cosine_similarity(&query_embedding, &entry_embedding);

                    if score >= min_score {
                        scored.push((
                            score,
                            SearchResult {
                                entry_id,
                                similarity: score,
                                content,
                                entry_type,
                                parent_id: None, // Set by caller if needed
                            },
                        ));
                    }
                }

                // Sort descending by similarity
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

                // Take top_k
                let results = scored.into_iter().take(top_k).map(|(_, r)| r).collect();

                Ok(results)
            })
            .await
            .map_err(|e| anyhow::anyhow!("Vector search failed: {e}"))?;

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// Unit Tests (TDD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!(
            (sim - 1.0).abs() < 1e-5,
            "Identical vectors should have similarity 1.0, got {sim}"
        );
    }

    #[test]
    fn test_cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - 0.0).abs() < 1e-5,
            "Orthogonal vectors should have similarity 0.0, got {sim}"
        );
    }

    #[test]
    fn test_cosine_similarity_opposite_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - (-1.0)).abs() < 1e-5,
            "Opposite vectors should have similarity -1.0, got {sim}"
        );
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0, "Zero vector should return 0.0");
    }

    #[test]
    fn test_cosine_similarity_mismatched_lengths() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0, "Mismatched lengths should return 0.0");
    }

    #[test]
    fn test_embedding_roundtrip() {
        let original = vec![0.1f32, 0.2, -0.3, 0.5, 1.0];
        let bytes = embedding_to_bytes(&original);
        let restored = bytes_to_embedding(&bytes);

        assert_eq!(original.len(), restored.len());
        for (a, b) in original.iter().zip(restored.iter()) {
            assert!((a - b).abs() < 1e-7, "Roundtrip error: {a} != {b}");
        }
    }

    #[test]
    fn test_embedding_bytes_length() {
        let embedding = vec![1.0f32; 1024]; // mxbai-embed-large dimension
        let bytes = embedding_to_bytes(&embedding);
        assert_eq!(bytes.len(), 1024 * 4, "Each f32 takes 4 bytes");
    }

    #[tokio::test]
    async fn test_mock_vector_search() {
        let mut mock = MockVectorSearch::new();
        mock.expect_search().returning(|_, _, _, _| {
            Ok(vec![SearchResult {
                entry_id: "e1".to_string(),
                similarity: 0.95,
                content: "Rust is memory safe".to_string(),
                entry_type: "knowledge".to_string(),
                parent_id: None,
            }])
        });

        let results = mock.search(&[0.1, 0.2, 0.3], "ws-1", 5, 0.7).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry_id, "e1");
        assert!(results[0].similarity > 0.9);
    }
}
