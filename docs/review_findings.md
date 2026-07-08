# Co-Force — Báo cáo Review Tài liệu Tổng thể & Đánh giá Khả thi

**Ngày review:** 2026-07-08
**Phạm vi:** `URD.md`, `implementation_instructions.md`, `docs/plans/01–04`, `progress.md`, `AGENTS.md`, trạng thái code trong `crates/`
**Phương pháp:** Đối chiếu chéo giữa các tài liệu + xác minh thực tế trên crates.io và Ollama registry (không đánh giá theo trí nhớ).

---

## 1. Kết luận Tổng quan

| Hạng mục | Đánh giá |
| :--- | :--- |
| Chất lượng URD | 🟢 Rất chi tiết, use case đầy đủ, có phân tích rủi ro nghiêm túc |
| Tính nhất quán giữa các docs | 🟡 Có 5 mâu thuẫn lớn cần sửa (xem §2) |
| Tính khả thi tổng thể | 🟢 **~85%** (tăng so với 82% trong URD — nhờ `rmcp` đã đạt 2.x stable) |
| Trạng thái code | 🔴 Skeleton chưa compile (lib.rs khai báo module chưa tồn tại, `co-force-mcp` thiếu `main.rs`) |
| Trải nghiệm setup theo thiết kế hiện tại | 🟡 Quá nặng cho lần đầu (bắt buộc Ollama + 2 models + wizard) → cần "zero-config first run" (xem Plan 05) |

**Khuyến nghị chính:** Cắt scope MVP về đúng "Sweet Spot" mà chính URD §11.3 đã xác định (1 user, 2–3 agents, 1 máy) — chạy được **không cần Ollama**, setup dưới 2 phút. RAG và A2A spawning là Phase 2–3. Chi tiết trong `docs/plans/00_roadmap.md`.

---

## 2. Phát hiện Lớn (phải sửa trước khi code)

### F-01 🔴 `rmcp` version & API drift — cả 3 nguồn đều sai lệch nhau

| Nguồn | Ghi nhận |
| :--- | :--- |
| `URD.md` §3.3, `implementation_instructions.md` | `rmcp = "0.16"`, features `["server", "transport-sse"]`, macro `#[rmcp::server]` |
| Root `Cargo.toml` | `rmcp = "0.1"`, features `["server", "transport-io"]` |
| **Thực tế crates.io (verified 2026-07-08)** | **`rmcp = "2.1.0"`** (stable, cập nhật 2026-07-02, ~15M downloads) |

Hệ quả quan trọng:
1. **Feature `transport-sse` không còn tồn tại** trong rmcp 2.x. MCP spec đã deprecate SSE transport; thay bằng **Streamable HTTP** — feature đúng là `transport-streamable-http-server` (+ `transport-streamable-http-server-session` cho session binding). Stdio dùng `transport-io`.
2. Macro `#[rmcp::server]` không phải API thật. API rmcp 2.x: `#[tool_router]` trên `impl`, `#[tool(...)]` trên từng method, implement trait `ServerHandler`, params qua struct derive `schemars::JsonSchema`.
3. **Risk R-01 (breaking changes pre-1.0) trong URD §13 đã lỗi thời theo hướng tốt**: rmcp đã qua 1.0 → hạ risk từ 🔴 xuống 🟢.

**Hành động:** Đã cập nhật root `Cargo.toml` lên `rmcp = "2"` với features đúng. Sample code trong plan 02 và implementation_instructions cần viết theo API `tool_router` (đã bổ sung ghi chú cập nhật vào 2 file này).

### F-02 🔴 Vector DB `embedvec` — tồn tại nhưng adoption quá thấp

Verified: `embedvec 0.8.0` có trên crates.io nhưng chỉ **~1.3k downloads** (so với `sqlite-vec` ~1.9M, `hnsw_rs` ~620k). Đặt hạ tầng memory dài hạn lên một crate gần như không ai dùng là rủi ro không cần thiết.

**Khuyến nghị (đơn giản hóa cả setup lẫn code):**
- **MVP:** Lưu embedding dạng BLOB ngay trong SQLite (`memory_entries`), tìm kiếm bằng **brute-force cosine** trong Rust. Với quy mô thực tế của workspace memory (vài nghìn entries × 1024d), brute-force < 10ms — hoàn toàn đủ, **không cần thêm bất kỳ storage engine nào**, không có file index để corrupt (loại bỏ luôn UC-30).
- **Khi > ~50k entries:** nâng cấp lên `sqlite-vec` (cùng file SQLite, không đổi kiến trúc) hoặc `hnsw_rs`.
- Trait `VectorSearch` che phía sau để việc nâng cấp không chạm use case.

### F-03 🟡 Mâu thuẫn vị trí lưu Vector Store

- URD §5.2: `<project>/.co-force/memory/embedvec-index/`
- URD §6.2: `~/.co-force/vector_store/{workspaceId}/`

**Chốt (theo mô hình central server):** Mọi state của server (SQLite, vectors) nằm ở **máy chạy server**: `~/.co-force/data/{workspaceId}/co-force.db` (vector nằm trong cùng file DB theo F-02). `<project>/.co-force/` chỉ chứa file phía client: `agent.json`, `session_status.json`, `AGENTS.md` sinh tự động — tất cả git-ignored.

### F-04 🟡 OS-level `chmod -w` toàn workspace (Lớp 4 / UC-04 reclaim) — rủi ro UX cao

Vấn đề với thiết kế "đặt toàn bộ file Read-only, chỉ mở khi lock":
1. Phá build tools, git (`.git/index`), formatter, IDE watchers — những tiến trình hợp pháp không đi qua MCP.
2. Agent chạy **cùng OS user** → tự `chmod +w` được; hàng rào chỉ mang tính nhắc nhở, không phải bảo mật thật.
3. Cross-platform phức tạp (Windows ACL ≠ chmod), nguy cơ để lại workspace read-only vĩnh viễn nếu server crash giữa chừng.

**Chốt:** Hạ xuống **strict mode opt-in** (`config: enforcement = "advisory" | "strict"`, mặc định `advisory`), chỉ áp cho file nguồn (không đụng `.git/`, `target/`, `node_modules/`). MVP dựa vào Lớp 1–3 (rule injection + tool description + interlocking errors) — đây cũng là kết luận thực dụng vì Lớp 3 (CHECK_IN_REQUIRED) đã đủ tạo vòng self-correction cho LLM.

### F-05 🟡 UC-37/38 (Spawn / Handover) — khả thi có điều kiện, thuộc Phase 3

- `claude -p "<prompt>"` là thật; nhưng `antigravity-cli --task --auto-approve` cần xác minh từng provider — CLI flags thay đổi thường xuyên.
- Thiếu trong thiết kế: **max spawn depth** (chống sub-agent đẻ sub-agent đệ quy), cost/token budget cho agent nền, reaping zombie process, và cách user quan sát/kill agent nền.
- **Chốt:** Provider registry dạng config (template lệnh khai báo trong `config.toml`, không hardcode trong Rust), `max_spawn_depth = 1` mặc định, mọi spawn ghi `agent_activities` + hiển thị trên dashboard. Đẩy sang Phase 3.

---

## 3. Phát hiện Nhỏ (sửa khi chạm vào từng file)

| # | Phát hiện | Sửa |
| :- | :--- | :--- |
| F-06 | Bảng UC summary (URD §7.1) thiếu UC-37, UC-38 (có trong Group I) | Bổ sung 2 dòng vào bảng, Priority P2 |
| F-07 | Version drift: impl doc ghi `rusqlite 0.31`/`tokio-rusqlite 0.5`, Cargo.toml dùng `0.32`/`0.6` | Chốt theo Cargo.toml (nguồn sự thật duy nhất về version là Cargo.toml) |
| F-08 | Đường dẫn config Claude sai: `~/.claude/claude_desktop_config.json` | Claude Code dùng `.mcp.json` (project scope) hoặc lệnh `claude mcp add`; `claude_desktop_config.json` là của Claude **Desktop** (macOS: `~/Library/Application Support/Claude/`) |
| F-09 | "Implicit Session Binding" (UC-01) mô tả theo SSE query params | Viết lại theo transport thật: stdio = 1 process/1 session (trivial); Streamable HTTP = bind qua `Mcp-Session-Id` header (rmcp feature `transport-streamable-http-server-session`) |
| F-10 | Typo `.windsufrules` (URD §9.2) | `.windsurfrules` |
| F-11 | `lib.rs` khai báo `db`, `workspace`, `engine`, `ollama` nhưng file chưa tồn tại; `co-force-mcp` khai báo `[[bin]]` nhưng thiếu `src/main.rs` → `cargo check` fail | Task đầu tiên của Phase 0: làm workspace compile + CI xanh |
| F-12 | Model names: `gemma4:e2b`, `mxbai-embed-large` — **verified tồn tại trên ollama.com** ✅ | Không cần sửa |
| F-13 | Dashboard đề xuất 2 port riêng (3847 HTTP + 3848 WS) | Gộp 1 port: axum serve static + WebSocket upgrade trên cùng listener — bớt 1 firewall rule, bớt 1 config key |
| F-14 | Chưa có plan cho Setup Wizard, `co-force init`, distribution — trong khi đây là yếu tố quyết định adoption | Đã bổ sung `docs/plans/05_setup_ux_and_onboarding.md` |
| F-15 | Chưa có roadmap/phasing tổng — 4 plans hiện tại không có thứ tự ưu tiên và định nghĩa MVP | Đã bổ sung `docs/plans/00_roadmap.md` |

---

## 4. Đánh giá Khả thi (cập nhật với dữ kiện đã verify)

| Component | URD cũ | Đánh giá lại | Lý do |
| :--- | :--- | :--- | :--- |
| MCP Server (rmcp) | 🟢 90% | 🟢 **97%** | rmcp đã 2.x stable, docs/examples phong phú; hết rủi ro pre-1.0 |
| SQLite (rusqlite) | 🟢 95% | 🟢 95% | Không đổi |
| Ollama Integration | 🟢 95% | 🟢 95% | Model names verified; giữ fallback queue |
| Vector Search | 🟢 85% (embedvec) | 🟢 **95%** (brute-force → sqlite-vec) | Loại bỏ dependency rủi ro, xóa luôn UC-30 |
| Agentic Chunking | 🟡 70% | 🟡 70% | Không đổi — Phase 2, có thể ship bản structural-only trước |
| LLM Classification | 🟡 75% | 🟡 78% | Few-shot + confidence threshold + manual override đã thiết kế tốt |
| Skill Auto-Detection | 🟡 65% | 🟡 65% | Phase 3+, không nằm trên critical path |
| Cross-Machine Sync | 🟢 85% | 🟢 85% | Streamable HTTP thay SSE — cùng độ khó |
| 4-Layer Guardrails | — | 🟢 88% (Lớp 1–3) / 🔴 50% (Lớp 4) | Lớp 4 hạ xuống opt-in (F-04) |
| A2A Spawn/Handover | — | 🟡 70% | Khả thi với provider registry + depth limit (F-05) |
| Tauri App | 🟢 90% | 🟢 90% | Nhưng đẩy Phase 4 — MVP dùng embedded web dashboard |

**Tổng thể: 🟢 ~85%** với điều kiện tuân theo phasing ở `00_roadmap.md`. Rủi ro lớn nhất còn lại không phải kỹ thuật mà là **scope**: 26 MCP tools + RAG + spawning + dashboard + Tauri là quá nhiều cho một MVP; nếu làm tuần tự theo roadmap thì từng phase đều khả thi độc lập và có giá trị sử dụng ngay.

---

## 5. Tóm tắt Quyết định Kiến trúc (chốt sau review)

1. **rmcp 2.x**, transports: `stdio` (mặc định, solo) + `streamable-http` (team/LAN). Không dùng SSE.
2. **Một file SQLite duy nhất cho mỗi workspace** chứa cả metadata lẫn vectors (brute-force cosine, trait `VectorSearch` để nâng cấp sau). Không dùng `embedvec`.
3. **Server state tại `~/.co-force/data/{workspaceId}/`** trên máy chạy server; `<project>/.co-force/` chỉ là client-side cache (git-ignored).
4. **Guardrails mặc định = Lớp 1–3**; chmod enforcement là strict-mode opt-in.
5. **Ollama là optional** — server chạy đầy đủ tính năng coordination khi không có Ollama; RAG degraded (store không vector, recall bằng keyword LIKE) + retry queue.
6. **Một binary `co-force` duy nhất** với subcommands (`serve`, `init`, `doctor`, `config`, `dashboard`) — chi tiết Plan 05.
7. **Dashboard MVP = web embedded trong binary** (axum, 1 port); Tauri là Phase 4.
8. **A2A spawn qua provider registry trong config.toml**, `max_spawn_depth = 1`, Phase 3.

Sơ đồ kiến trúc tổng hợp sau các quyết định trên: xem `docs/architecture.md`.

---

## 6. Điều chỉnh Định hướng (2026-07-08, v2 — quyết định của product owner)

Sau khi báo cáo review được trình bày, product owner chốt lại định hướng khiến **một số quyết định ở §5 bị thay thế**:

| Quyết định §5 cũ | Trạng thái | Thay bằng |
| :--- | :--- | :--- |
| #5 — Ollama optional, RAG degraded (keyword) khi thiếu | ❌ **Hủy** | Ollama **bắt buộc** trên server. Không có degraded mode cắt tính năng — lỗi thì `SERVICE_UNAVAILABLE` rõ ràng + auto-heal + alert (Master Plan §5) |
| Roadmap MVP phased (bản đầu của 00_roadmap) | ❌ **Hủy** | **Một release 1.0 end-to-end product-ready**, không MVP trung gian (Master Plan v2) |
| Solo zero-config là mặc định | 🔄 **Điều chỉnh** | Triển khai chuẩn = **server độc lập + cloudflared tunnel + domain public**; server init được phép nặng, **client setup < 60s không cần binary** (Plan 05 v2, Plan 06). LAN/local vẫn hỗ trợ với đầy đủ tính năng |
| #7 — Dashboard MVP tối giản | 🔄 **Mở rộng** | Dashboard đầy đủ trong 1.0 (review queue, quality metrics, admin token/enrollment) |

Quyết định **mới** phát sinh từ định hướng:
- **N-01:** Mục tiêu tối thượng của sản phẩm là **chất lượng đầu ra của agents, không phải tốc độ** → bổ sung **Quality Engine** làm workstream critical-path: role separation, review chéo bắt buộc, verification evidence, critique fan-out, server-side LLM recheck (Plan 07).
- **N-02:** Auth bằng Bearer token per-máy (issue/revoke độc lập), enrollment token TTL 24h đổi lấy agent token dài hạn (Plan 06 §4).
- **N-03:** Thêm vai trò model thứ 3 — **reasoner** (mặc định `qwen3:14b` local, hoặc cloud API) cho các dịch vụ chất lượng phía server.
- **N-04:** `wait_events` long-poll ≤ 55s để tương thích Cloudflare proxy timeout.

Các quyết định §5 còn lại (#1 rmcp 2.x + streamable HTTP, #2 vector BLOB trong SQLite, #3 storage layout server-side, #4 guardrails Lớp 1–3 + strict mode opt-in, #6 một binary, #8 provider registry cho spawn) **giữ nguyên hiệu lực**.
