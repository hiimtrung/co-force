# Implementation Plan: 04 - Agentic RAG and LLM Integration

**Status:** Plan
**Target:** `crates/co-force-core/src/llm/`

## 1. Overview
Triển khai hệ thống tự động trích xuất, nhúng (embedding), và truy xuất thông tin (RAG), kết hợp với LLM để phân loại dữ liệu đầu vào. Trọng tâm là kiến trúc Local-first sử dụng Ollama, không gửi code/dữ liệu ra cloud trừ khi có yêu cầu.

## 2. Giải pháp Kỹ thuật

### 2.1 Provider Adapter (LLM Interface)
*Tham chiếu:* `implementation_instructions.md` (Section 7)

Tạo một `LlmProvider` trait để abstract hóa API của LLM, giúp dễ mock (cho TDD) hoặc switch provider (OpenAI vs Ollama).
- **Vị trí:** `crates/co-force-core/src/llm/ollama.rs`
- **Sử dụng:** `reqwest` HTTP Client.
- **Tương tác:** Gọi API `/api/embeddings` cho vector và `/api/generate` cho classification.
- **Model mặc định:** `mxbai-embed-large` cho Embedding, `gemma4:e2b` cho Classifier.

### 2.2 Agentic Chunking Strategy
*Tham chiếu:* `URD.md` (Section 15.A Agentic RAG Chunking Strategy)

Không cắt chuỗi ngẫu nhiên (Fixed-size chunking) vì dễ làm hỏng logic của code/markdown. Thay vào đó, áp dụng Agentic Chunking:
- **Bước 1 (Structural Splitting):** Dùng Regex hoặc Parser cơ bản để chia văn bản theo function/class (với Code) hoặc heading/paragraph (với Markdown/Prose).
- **Bước 2 (Semantic Boundary):** Tính cosine similarity giữa các cụm liền kề; nếu có sự suy giảm lớn (>0.3) thì tách thành node độc lập.
- **Bước 3 (Parent-Child Hierarchy):** Lưu 2 bản: Chunk nhỏ (128-256 tokens) đưa vào Vector DB để tìm kiếm nhanh, nhưng khi hit sẽ trả về Chunk cha (512-1024 tokens) để giữ đủ ngữ cảnh.

### 2.3 Vector Database & Tích hợp Bộ nhớ
Tích hợp một Vector Engine nhẹ (có thể là in-memory cosine similarity hoặc thư viện như `embedvec`) kết hợp với SQLite:
- Mỗi khi `StoreMemoryUseCase` chạy, hash MD5/SHA256 của chuỗi để kiểm tra `embedding_cache` (bảng sqlite). Nếu có lấy ra dùng luôn, tiết kiệm tài nguyên.
- Dữ liệu thô (Raw Memory) lưu vào bảng SQLite `memory_entries` kèm vector_id.
- Truy vấn (`co_force_recall`) sẽ embed chuỗi câu hỏi -> quét vector similarity -> fetch raw content từ SQLite trả về cho Agent.

### 2.4 Fallback Strategy
*Tham chiếu:* `URD.md` (UC-28: Ollama Unavailable Fallback)

Xử lý sự cố nếu máy của user không chạy Ollama:
- LLM Provider báo lỗi -> Use Case bắt lỗi -> Lưu Memory Entry thành plain text (không nhúng).
- Bật cờ `unclassified` và `vector_id = null`.
- Background task sẽ retry nhúng sau 5 phút nếu Ollama online trở lại, đảm bảo không mất dữ liệu.
