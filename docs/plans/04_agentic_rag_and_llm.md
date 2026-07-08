# Detailed Implementation Plan: 04 - Agentic RAG and LLM Integration

**Status:** Ready for Implementation
**Target:** `crates/co-force-core/src/llm/`

> **⚠️ Update 2026-07-08 (see `docs/review_findings.md` F-02):** **Do not use the `embedvec` crate** (extremely low adoption, ~1.3k downloads). Final decision:
> - Embeddings are stored as a **BLOB in the `memory_entries` table** of SQLite; search is done using **brute-force cosine similarity** in Rust (< 10ms with several thousand entries × 1024d).
> - Abstracted behind the `VectorSearch` trait — when a workspace exceeds ~50k entries, upgrade to `sqlite-vec` (same DB file) or `hnsw_rs` without changing the use case.
> - Consequence: No separate index files anymore → UC-30 (Vector DB corruption recovery) is out of scope.
> - Step 5 in the "Steps to Implement" below is updated based on this decision.
>
> **⚠️ Update v2.3 (F-19):**
> 1. Use Ollama's **`/api/embed`** endpoint (receives batch `input`, returns `embeddings: [[f32]]`) — `/api/embeddings` is deprecated.
> 2. Behavior when LLM is down is set per N2 (no silent degradation): `store_memory` **still saves** (no data loss) but the response specifies `index_status: "pending"` + queues for re-embedding; `recall` cannot embed the query → returns `SERVICE_UNAVAILABLE` (no fallback results). §3.1 below is updated accordingly.
> 3. The phrase "completely relies on Local LLMs" is only true for **embedding + classifier**; the **reasoner** is allowed to go to the cloud (N-03) — when the user selects the cloud, spec/diff summaries will leave the local machine (explicitly noted in the installer + docs, defaulting to local `qwen3:14b`).

## 1. Context & Objectives
The RAG (Retrieval-Augmented Generation) and data classification features of Co-Force rely on Local LLMs (Ollama) to keep the project code secure. This module designs the **Agentic Chunking** algorithm, resolving the shortcomings of traditional fixed-size chunking (which breaks code logic).

*Reference Documents:*
- `architecture.md` §1 (Ollama required, 3 model roles), Plan 06 §5 (`[llm]` config)
- `URD.md` (Section 15.A: Agentic RAG Chunking Strategy)

---

## 2. LlmProvider Interface (Ports Layer)
**Location:** `crates/co-force-core/src/engine/ports.rs`

Create a common trait to easily switch to OpenAI if requested by the user.

```rust
use async_trait::async_trait;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Number of dimensions depends on model config (mxbai-embed-large = 1024);
    /// changing model/dimensions → re-embed everything (Plan 06 §7), not hardcoded.
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
    
    /// Returns classification type (Memory/Knowledge/Skill) and confidence score (0.0 -> 1.0)
    async fn classify(&self, content: &str) -> anyhow::Result<(String, f32)>;
}
```

## 3. Ollama Integration
**Location:** `crates/co-force-core/src/llm/ollama.rs`

Use the HTTP client `reqwest` to call the local Ollama API (typically at port 11434).

### 3.1 Embed Function & Fallback Logic
```rust
use reqwest::Client;
use serde::Deserialize;

pub struct OllamaProvider {
    client: Client,
    base_url: String, // default: http://localhost:11434
    embedding_model: String, // mxbai-embed-large
    classifier_model: String, // gemma4:e2b
}

impl OllamaProvider {
    pub async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        #[derive(Deserialize)]
        struct EmbedResponse { embeddings: Vec<Vec<f32>> }

        // timeout 30s
        let req = self.client.post(format!("{}/api/embed", self.base_url)) // /api/embeddings is deprecated
            .json(&serde_json::json!({
                "model": self.embedding_model,
                "input": text            // /api/embed receives batch; response: { embeddings: [[f32]] }
            }))
            .timeout(std::time::Duration::from_secs(30));

        let res = req.send().await?;
        if !res.status().is_success() {
            return Err(anyhow::anyhow!("Ollama error: {}", res.status()));
        }
        
        let mut parsed: EmbedResponse = res.json().await?;
        parsed.embeddings.pop().ok_or_else(|| anyhow::anyhow!("empty embeddings"))
    }
}
```
**Resilience Note (NO silent fallback — F-19):** If `embed()` returns `Err`, `StoreMemoryUseCase` still stores the Memory (embedding NULL) but the **response must explicitly state** `index_status: "pending"` — the agent/user knows the entry cannot be searched yet. A background queue scans entries missing embeddings and re-embeds them when the LLM recovers; in the meantime, `recall` returns a warning `PARTIAL_INDEX {pending_count}`. Specifically, if `recall` cannot embed the **query** → return `SERVICE_UNAVAILABLE`; never substitute with keyword search.

### 3.2 Classify Function (Few-shot prompting)
To get the gemma4 model (2B params) to classify data correctly, use few-shot prompting.

```rust
let prompt = format!(
    "You are a routing classification AI.\n\
    Classify the following text into exactly one of three categories: MEMORY, KNOWLEDGE, or SKILL.\n\
    - MEMORY: A specific fact about a file, bug, or temporary state (e.g., 'auth.ts has a bug on line 42').\n\
    - KNOWLEDGE: A general architectural principle or rule (e.g., 'always use PostgreSQL, not MySQL').\n\
    - SKILL: A step-by-step reusable procedure or script (e.g., '1. Install X, 2. Run Y').\n\
    \n\
    Text to classify: \"{content}\"\n\
    Respond ONLY with the category name (MEMORY, KNOWLEDGE, or SKILL)."
);
// Send to /api/generate
```

---

## 4. Agentic Chunking Algorithm
**Location:** `crates/co-force-core/src/llm/chunker.rs`

An intelligent text-splitting algorithm preserving logic integrity.

### Pseudo-code Implementation
```rust
pub struct Chunk {
    pub id: String,                 // parent needs id for child reference (F-19.4)
    pub content: String,
    pub is_parent: bool,
    pub parent_id: Option<String>,
}

pub fn agentic_chunking(text: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    
    // Step 1: Structural Splitting
    // Use regex to split by blank lines (\n\n) or markdown headers (##)
    let initial_splits = text.split("\n\n").collect::<Vec<_>>();
    
    let mut current_parent = String::new();
    let mut child_chunks = Vec::new();

    // Step 2: Merge into Child chunk (128-256 tokens)
    for split in initial_splits {
        if token_count(&current_parent) + token_count(split) <= 1024 {
            // Keep as Parent context
            current_parent.push_str(split);
            current_parent.push_str("\n\n");
            
            // Simultaneously extract ~200 token segments as Children
            child_chunks.push(split.to_string());
        } else {
            // Reached Parent limit -> Create Parent-Child structure
            let parent_id = uuid::Uuid::new_v4().to_string();
            chunks.push(Chunk { id: parent_id.clone(), content: current_parent.clone(), is_parent: true, parent_id: None });
            
            for child in &child_chunks {
                chunks.push(Chunk { id: uuid::Uuid::new_v4().to_string(), content: child.clone(), is_parent: false, parent_id: Some(parent_id.clone()) });
            }
            
            // Reset state
            current_parent = split.to_string();
            child_chunks.clear();
            child_chunks.push(split.to_string());
        }
    }
    
    // Cleanup last loop...
    
    chunks
}
```

### Retrieval Logic (When calling `co_force_recall`)
When an agent queries, the system searches vector similarity on **Child Chunks**. If Child Chunk X is found, the system retrieves its `parent_id` and returns the content of the corresponding **Parent Chunk** to the Agent. This ensures fast search times with small segments while providing a context window large enough for the Agent to understand the code/prose.

---

## 5. Steps to Implement (Step-by-Step)
1. In `core`, create the `llm/ollama.rs` file and configure `reqwest`.
2. Define the standard JSON structures (Serde) for the Ollama API Request/Response (`/api/embeddings`, `/api/generate`).
3. Configure the Fallback & Retry algorithm in `StoreMemoryUseCase`.
4. Implement `agentic_chunking` and write Unit Tests to verify token grouping logic (the `token_count` estimator can use `tiktoken-rs` or a rough character/word count).
5. Implement `BruteForceCosine` under the `VectorSearch` trait (F-02 decision — DO NOT use any vector DB libraries; load embedding BLOBs from `memory_entries`). Ensure the Query -> Embedding -> Cosine Similarity -> Fetch SQLite pipeline works smoothly.
