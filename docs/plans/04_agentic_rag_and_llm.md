# Kế Hoạch Triển Khai Chi Tiết: 04 - Agentic RAG and LLM Integration

**Status:** Ready for Implementation
**Target:** `crates/co-force-core/src/llm/`

> **⚠️ Cập nhật 2026-07-08 (xem `docs/review_findings.md` F-02):** **Không dùng crate `embedvec`** (adoption quá thấp ~1.3k downloads). Quyết định chốt:
> - Embedding lưu dạng **BLOB trong bảng `memory_entries`** của SQLite; search bằng **brute-force cosine** trong Rust (< 10ms với vài nghìn entries × 1024d).
> - Che sau trait `VectorSearch` — khi workspace vượt ~50k entries, nâng cấp lên `sqlite-vec` (cùng file DB) hoặc `hnsw_rs` mà không đổi use case.
> - Hệ quả: không còn index file riêng → UC-30 (Vector DB corruption recovery) bị loại khỏi scope.
> - Bước 5 trong "Trình tự Triển khai" bên dưới đọc theo quyết định này.
>
> **⚠️ Cập nhật v2.3 (F-19):**
> 1. Dùng endpoint **`/api/embed`** của Ollama (nhận batch `input`, trả `embeddings: [[f32]]`) — `/api/embeddings` đã deprecated.
> 2. Hành vi khi LLM down chốt theo N2 (no silent degradation): `store_memory` **vẫn lưu** (không mất dữ liệu) nhưng response ghi rõ `index_status: "pending"` + queue re-embed; `recall` không embed được query → `SERVICE_UNAVAILABLE` (không có kết quả thay thế). §3.1 bên dưới đọc theo hướng này.
> 3. Câu "dựa hoàn toàn Local LLMs" chỉ còn đúng cho **embedding + classifier**; **reasoner** được phép đi cloud (N-03) — khi user chọn cloud, nội dung spec/diff summary sẽ rời máy (nói rõ trong installer + docs, mặc định vẫn local `qwen3:14b`).

## 1. Context & Mục Tiêu
Tính năng RAG (Retrieval-Augmented Generation) và phân loại dữ liệu (Classification) của Co-Force dựa hoàn toàn vào Local LLMs (Ollama) nhằm bảo mật code dự án. Module này thiết kế thuật toán **Agentic Chunking**, giải quyết nhược điểm của fixed-size chunking truyền thống (làm rách logic của code).

*Tài liệu tham chiếu:*
- `architecture.md` §1 (Ollama bắt buộc, 3 vai trò model), Plan 06 §5 (config `[llm]`)
- `URD.md` (Section 15.A: Agentic RAG Chunking Strategy)

---

## 2. LlmProvider Interface (Ports Layer)
**Vị trí:** `crates/co-force-core/src/engine/ports.rs`

Tạo trait chung để có thể switch sang OpenAI nếu user cầu.

```rust
use async_trait::async_trait;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Số dimension phụ thuộc model config (mxbai-embed-large = 1024);
    /// đổi model/dimension → re-embed toàn bộ (Plan 06 §7), không hardcode.
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
    
    /// Trả về Type phân loại (Memory/Knowledge/Skill) và độ tự tin (0.0 -> 1.0)
    async fn classify(&self, content: &str) -> anyhow::Result<(String, f32)>;
}
```

## 3. Ollama Integration
**Vị trí:** `crates/co-force-core/src/llm/ollama.rs`

Sử dụng thư viện HTTP client `reqwest` để gọi Ollama API local (thường ở cổng 11434).

### 3.1 Hàm Embed & Fallback Logic
```rust
use reqwest::Client;
use serde::Deserialize;

pub struct OllamaProvider {
    client: Client,
    base_url: String, // mặc định: http://localhost:11434
    embedding_model: String, // mxbai-embed-large
    classifier_model: String, // gemma4:e2b
}

impl OllamaProvider {
    pub async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        #[derive(Deserialize)]
        struct EmbedResponse { embeddings: Vec<Vec<f32>> }

        // timeout 30s
        let req = self.client.post(format!("{}/api/embed", self.base_url)) // /api/embeddings đã deprecated
            .json(&serde_json::json!({
                "model": self.embedding_model,
                "input": text            // /api/embed nhận batch; response: { embeddings: [[f32]] }
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
**Lưu ý Resilience (KHÔNG phải silent fallback — F-19):** Nếu `embed()` trả về `Err`, `StoreMemoryUseCase` vẫn lưu Memory (embedding NULL) nhưng **response phải nói rõ** `index_status: "pending"` — agent/user biết entry chưa search được. Background queue quét entries thiếu embedding và re-embed khi LLM hồi phục; trong lúc đó `recall` trả kèm warning `PARTIAL_INDEX {pending_count}`. Riêng `recall` mà không embed được **query** → trả `SERVICE_UNAVAILABLE`, tuyệt đối không thay bằng keyword search.

### 3.2 Hàm Classify (Few-shot prompting)
Để model gemma4 (2B params) nhận diện chính xác loại dữ liệu, phải dùng few-shot prompting.

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
// Gửi lên /api/generate
```

---

## 4. Agentic Chunking Algorithm
**Vị trí:** `crates/co-force-core/src/llm/chunker.rs`

Thuật toán cắt văn bản thông minh bảo toàn tính toàn vẹn của logic.

### Pseudo-code Implementation
```rust
pub struct Chunk {
    pub id: String,                 // parent cần id để children tham chiếu (F-19.4)
    pub content: String,
    pub is_parent: bool,
    pub parent_id: Option<String>,
}

pub fn agentic_chunking(text: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    
    // Bước 1: Structural Splitting
    // Sử dụng Regex để cắt theo blank lines (\n\n) hoặc markdown headers (##)
    let initial_splits = text.split("\n\n").collect::<Vec<_>>();
    
    let mut current_parent = String::new();
    let mut child_chunks = Vec::new();

    // Bước 2: Ghép nối (Merging) thành Child chunk (128-256 tokens)
    for split in initial_splits {
        if token_count(&current_parent) + token_count(split) <= 1024 {
            // Giữ lại làm Parent context
            current_parent.push_str(split);
            current_parent.push_str("\n\n");
            
            // Đồng thời tách các đoạn ~200 tokens thành Child
            child_chunks.push(split.to_string());
        } else {
            // Đạt giới hạn Parent -> Tạo cấu trúc Parent-Child
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
    
    // Cleanup vòng lặp cuối...
    
    chunks
}
```

### Retrieval Logic (Khi gọi `co_force_recall`)
Khi agent query, hệ thống search vector similarity trên các **Child Chunks**. Nếu tìm thấy Child Chunk X, hệ thống lấy `parent_id` của nó và trả về toàn bộ nội dung của **Parent Chunk** tương ứng cho Agent. Cách này đảm bảo tốc độ search của đoạn nhỏ, nhưng cung cấp Context đủ lớn để Agent hiểu đoạn code/văn bản.

---

## 5. Trình tự Triển khai (Step-by-Step)
1. Trong `core`, tạo file `llm/ollama.rs` và cài đặt `reqwest`.
2. Định nghĩa cấu trúc JSON chuẩn (Serde) cho Request/Response của API Ollama (`/api/embeddings`, `/api/generate`).
3. Cài đặt thuật toán Fallback & Retry trong `StoreMemoryUseCase`.
4. Viết hàm `agentic_chunking` và các Unit Tests kiểm tra tính đúng đắn của việc gom nhóm token (`token_count` estimator có thể dùng `tiktoken-rs` hoặc đếm từ thô).
5. Implement `BruteForceCosine` sau trait `VectorSearch` (quyết định F-02 — KHÔNG dùng thư viện vector DB nào; embedding BLOB đọc từ `memory_entries`). Đảm bảo luồng Query -> Embedding -> Cosine Similarity -> Fetch SQLite hoạt động trơn tru.
