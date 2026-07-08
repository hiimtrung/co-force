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

---

## 7. Review vòng 3 (2026-07-08, v2.3) — điểm bất khả thi/không hợp lý còn sót sau v2.2

### F-16 🔴 Docker Compose (Plan 06 §2.1) hỏng về network — `bind 127.0.0.1` trong container

Nguyên tắc "chỉ bind 127.0.0.1" đúng cho bare-metal nhưng **bất khả thi trong Docker**: mỗi container có network namespace riêng — `co-force` bind loopback thì container `cloudflared` (và `ollama` ngược lại) **không bao giờ với tới được** qua bridge network. Compose file như viết sẽ không chạy.

**Chốt:** Trong container, services bind `0.0.0.0` (override qua env `CO_FORCE_BIND`); tính cô lập do **compose network không publish port ra host** đảm nhiệm — tương đương về an toàn với loopback ở bare-metal. Cloudflared token-mode: public hostname trên CF dashboard trỏ `http://co-force:3846` (tên service). Đã sửa Plan 06 §2.1.

### F-17 🔴 `api_tokens` trong DB per-workspace là bất khả thi

Architecture §7 đặt `api_tokens` trong `data/{workspaceId}/co-force.db`, nhưng: (1) admin/enrollment token có `workspace_scope = "*"` — không thuộc workspace nào; (2) AuthLayer chạy **trước khi** request được gắn workspace (enrollment còn xảy ra trước khi workspace tồn tại) — không biết mở file DB nào để tra token.

**Chốt:** Thêm **DB cấp server** `/var/lib/co-force/server.db`: `api_tokens`, `workspaces` (registry id ↔ tên ↔ đường dẫn data), `audit_log`, install/ops state. DB per-workspace giữ phần còn lại. Đã sửa architecture §7, Plan 06 §4.1, Master Plan WS-A.

### F-18 🔴 Cơ chế giao agent token cho client (Plan 05 §3) không hoạt động như mô tả

Hai lỗi ghép vào nhau:
1. `.mcp.json` env expansion (`${CO_FORCE_TOKEN}`) đọc **biến môi trường của process client** — không có cơ chế nào tự đọc file `.co-force/token`. Script ghi token vào file rồi tham chiếu `${VAR}` = client nhận header rỗng.
2. `.mcp.json`/`.cursor/mcp.json` là file **project-scope, mặc định được commit để chia sẻ team** — trong khi agent token là **per-máy**. Nhét token per-máy vào file chia sẻ là mâu thuẫn thiết kế (buộc gitignore file vốn sinh ra để commit).

**Chốt (đã sửa Plan 05 §3):** token đi vào **config user/machine-scope, nằm ngoài repo**: Claude Code dùng `claude mcp add -s local -t http ... --header` (ghi vào `~/.claude.json`, per-project-per-máy, không commit); Cursor/Windsurf dùng config global `~/.cursor/mcp.json` / `~/.codeium/windsurf/mcp_config.json`. Chỉ fallback ghi token thẳng vào file project (CI/generic) mới cần gitignore + `git check-ignore` gate như cũ. Kèm theo: script **verify header thật sự được gửi** (gọi `tools/list`); client nào không hỗ trợ custom header → in hướng dẫn thay vì ghi config chết.

### F-19 🟡 Plan 04 mâu thuẫn chính sách N2 + API Ollama lỗi thời

1. §3.1 "lưu memory với `vector_id = null`, cron 5 phút re-embed" — viết theo tinh thần fallback cũ, **âm thầm** (vi phạm N2). Chốt hành vi per-tool: `store_memory` khi embed fail → **vẫn lưu** (không mất dữ liệu) nhưng response ghi rõ `index_status: "pending"`; `recall` khi không embed được query → `SERVICE_UNAVAILABLE` (không có kết quả thay thế); queue re-embed xả khi LLM hồi phục. Nhất quán với `PARTIAL_INDEX` (architecture §6.3) và Master Plan §5.
2. Endpoint `/api/embeddings` đã **deprecated** phía Ollama — dùng `/api/embed` (nhận batch `input`, trả `embeddings: [[f32]]`).
3. §1 "dựa hoàn toàn vào Local LLMs nhằm bảo mật" đã lỗi thời: embedding/classifier luôn local, **reasoner được phép đi cloud** (N-03) — tài liệu phải nói rõ trade-off dữ liệu gửi đi khi user chọn cloud.
4. Pseudo-code chunking: struct `Chunk` không có `id` nhưng children tham chiếu `parent_id`; `parent_id` sinh ra **sau** khi push parent với `parent_id: None` → mọi liên kết parent-child đứt. Đã sửa mẫu.

### F-20 🟡 Task state machine (Plan 07 §3) có trạng thái cụt & thiếu đường thoát

- `blocked` và `pending_handover` **không có transition ra** — task vào là kẹt vĩnh viễn.
- `awaiting_approval` không có đường user **từ chối** (chỉ có approve).
- Không có `cancelled` — user không thể hủy task sai/hết cần mà không phá state machine.

**Chốt:** thêm `blocked → in_progress` (dependency resolved), `pending_handover → in_progress` (agent mới nhận qua L2/L3), `pending_handover → approved` (timeout không ai nhận → về backlog), `awaiting_approval → draft` (user reject kèm lý do), và `cancelled` reachable từ mọi trạng thái chưa completed (chỉ user/admin). Đã sửa Plan 07 §3.

### F-21 🟡 Revision tracking "phát hiện code đổi sau submit" là bất khả thi như mô tả

Plan 07 bước 4 viết "revision tăng khi lock/unlock ghi nhận **file đổi** sau submit" — server **không nhìn thấy filesystem client**, không thể biết file đổi. Định nghĩa lại theo sự kiện server quan sát được: revision tăng khi (a) task quay lại `in_progress` (rework), (b) agent lock thêm/lock lại files của task sau khi đã có evidence, (c) submit_verification mới, (d) `commit_sha` mới. Khi workspace có git remote + L3 bật: server **verify `commit_sha` tồn tại trong mirror** ngay lúc `submit_verification` (fetch trước) — không thấy commit → `EVIDENCE_STALE {reason: "commit_not_found"}` yêu cầu push. Không có remote: chấp nhận giới hạn — hàng rào thật là reviewer chạy test độc lập. Đã sửa Plan 07 §5.1, architecture §6.3.

### F-22 🟡 Deadlock policy `reviewer_must_differ = "provider"` khi hệ chỉ có 1 provider

Workspace chỉ có agents Claude Code + worker pool `providers = ["claude-code"]` → không bao giờ thỏa policy → **mọi task kẹt ở code_review vĩnh viễn** (auto-staffing cũng chịu). Đứng gate + alert là đúng tinh thần N2, nhưng để user phát hiện lúc runtime là không hợp lý. **Chốt:** validate lúc **set policy / enroll / install**: nếu `reviewer_must_differ="provider"` mà < 2 providers khả dụng → từ chối set kèm hướng dẫn (thêm provider hoặc hạ về `"agent"`), dashboard cảnh báo khi số provider tụt xuống 1. Đã sửa Plan 07 §8.

### F-23 🟢 Tài liệu còn tham chiếu API/mô hình đã hủy (sửa ngay để DEV không copy nhầm)

| Nơi | Vấn đề | Sửa |
| :--- | :--- | :--- |
| `progress.md` §2 | Còn "`#[rmcp::server]`", "Transport (stdio / sse)" | → `tool_router`/`ServerHandler`, streamable-http |
| Plan 02 §3.2 sample | Sample code còn `#[rmcp::server]` | → `#[tool_router]` + ghi chú API thật |
| Plan 03 §4.1 + bước 4 | Hardcode `match provider` (antigravity/claude) trong Rust — trái quyết định F-05 (provider registry trong config) | Ghi chú sample minh họa; command template lấy từ `config.toml [providers]` |
| Plan 04 §2 | Comment "Trả về vector 1024 dimensions" hardcode | Dimension theo model config (đổi model → re-embed, Plan 06 §7) |
| Architecture §6.4 | Tool 1 ghi "duy nhất không cần session" nhưng tool 3 (`guide`) cũng không cần | Sửa wording |

### F-24 🟢 `wait_events` 55s vs timeout tool-call phía client MCP

55s nằm dưới timeout Cloudflare (100s) nhưng **một số MCP client có timeout tool-call riêng ngắn hơn** — client cắt sớm thì long-poll thành lỗi lặp. Chốt: default `timeoutSecs = 25`, max 55; enrollment script chạy 1 lần `wait_events` thật để đo và in khuyến nghị nếu client cắt sớm. Đã ghi vào Plan 07 §4.2.

---

## 8. Review vòng 4 (2026-07-08, v2.4) — Provider CLI coverage

### F-25 🟡 Toàn bộ docs chỉ nhắc Claude Code CLI — thiếu Codex CLI & Antigravity CLI (đã sửa)

Kiến trúc trước đây hardcode `claude`/`antigravity-cli` phỏng đoán trong ví dụ; không có spec tích hợp cho **Codex CLI** (OpenAI) và **Antigravity CLI `agy`** (Google — kế nhiệm Gemini CLI, Gemini CLI đã shutdown 2026-06-18). Điều này mâu thuẫn trực tiếp với Quality Engine: `reviewer_must_differ = "provider"` và critique đa mô hình (Plan 07) chỉ có nghĩa khi ≥ 2 provider thật sự tham gia — liên đới F-22.

**Đã deep-research (docs chính thức OpenAI/Google) + đối chiếu `ref/tutti`** (dự án shared-workspace đa agent chạy trên subscription — mô hình `ProviderSpec` registry với binary names, adapter command, auth markers, login spec per provider là mẫu tốt; Tutti điều khiển Codex qua `codex app-server` và các CLI khác qua ACP). Kết quả: **Plan 08 mới** (`docs/plans/08_provider_cli_integration.md`) — subscription-first, registry khai báo, spec verified cho claude/codex/agy/cursor-agent, 4 caveats phải xử lý trong code (C1: Codex `exec` auto-cancel MCP approvals — chỉ bypass trong L3 sandbox; C2: `bearer_token_env_var` cần env var thật; C3: agy MCP header chưa chắc — verify lúc enroll + stdio shim fallback; C4: auth-status parser per provider). Đồng bộ: architecture §1/§5, Plans 03/05/06/07, roadmap.

### F-26 🟢 Dọn tài liệu (2026-07-08): xóa `implementation_instructions.md`

File chứa quá nhiều chỉ dẫn sai không cứu được bằng banner: pin `rmcp 0.16` + `transport-sse` trong sample Cargo.toml; banner của chính nó còn ghi "Ollama là optional" (trái nguyên tắc N2 đã chốt); config mẫu 2 port dashboard (trái F-13); classify "fallback keyword heuristic nếu Ollama unreachable" (silent degradation — bị cấm); schema thiếu 7 bảng mới + còn cột `vector_id` (trái F-02); section Deployment SSE `http://<ip>:3846/sse` (transport đã bỏ); crate `co-force-tauri` (đã đổi hướng thành `co-force-app` client-side).

**Nội dung còn giá trị đã được chuyển đi trước khi xóa:** schema SQL đầy đủ 6 bảng gốc → **Plan 01 §3** (đã cập nhật: `embedding BLOB` thay `vector_id`, `tasks.status` theo Plan 07 + cột `revision`/`rework_cycle` cho F-21, indexes hot-path); TaskStatus enum trong Plan 01 §2.2 cập nhật theo state machine Plan 07. Tham chiếu trong Plans 01/02/04 đã trỏ về `architecture.md` + plans tương ứng; URD banner ghi chú việc xóa. Lệnh dev thường dùng (cargo test/clippy pedantic/fmt) đã nằm trong AGENTS.md §2.3–2.4 và DoD các workstream.

### F-27 🟡 Thiếu đặc tả "agent làm gì sau khi setup" — onboarding contract & hành vi đồng nhất (đã sửa)

Sau Plan 05 (client setup xong) chưa có tài liệu nào chốt: agent (Claude Code hay CLI bất kỳ) biết **điểm khởi đầu** ở đâu, **dùng tool nào cho việc gì**, và cơ chế nào khiến **mọi agent hành xử giống nhau**. Nội dung rải rác: Plan 05 §3.5 chỉ nói "rule injection" một dòng; template gốc URD §9.3 thì chứa chỉ dẫn sai (dọa "OS Permission Denied/chmod ban" — không còn đúng sau khi Lớp 4 đổi hình thái in-band; flow `update_task(completed)` nay là `GATE_VIOLATION`); URD §10 Phase 5–6 thiếu quality gates.

**Đã sửa — tạo `docs/plans/09_agent_operating_protocol.md`:** (1) chuỗi khám phá 4 điểm chạm của agent lạnh (rules file → tool descriptions → check_in response → envelope mọi response — agent không cần *nhớ* protocol, protocol tự tìm đến agent); (2) **rules template chốt v1** (managed block, tiếng Anh, chỉ hứa những gì server thật sự enforce, kèm bảng "tool nào khi nào"); (3) chuẩn viết 38 tool descriptions (Lớp 2); (4) spec `co_force_guide` động + cờ `onboarding: true`; (5) playbook theo role (developer/reviewer/critic/pm); (6) **ma trận hành-vi-đồng-nhất ↔ lớp enforce** — mỗi quy tắc có ít nhất 1 lớp cưỡng chế server-side, không dựa thiện chí LLM; (7) E2E "cold agent" làm tiêu chí nghiệm thu. URD §9.3/§10 gắn note thay thế; Plan 05, architecture §6, roadmap trỏ về Plan 09.

### F-28 🟡 Thiếu cơ chế cho kịch bản "1 agent duy nhất + công việc dài" (solo → context phình → hallucinate) — đã sửa

Kịch bản thực tế: workspace chỉ có 1 agent (vd Antigravity), việc dài/nhiều task nhỏ → agent ôm hết, context phình → hallucinate, chất lượng sụp. Docs có auto-staffing phía server (Plan 07 §2.2) nhưng thiếu: (1) cách agent **tự biết mình solo** và phải chia nhỏ; (2) đôn agent gốc làm **PM** + ước lượng cần bao nhiêu dev/test/ba/qa trước khi spawn; (3) xử lý race condition khi nhiều subagent chạy **cùng máy, cùng working tree** (locks logic không chặn `git add -A`, formatter toàn repo, build artifacts).

**Đã sửa — tạo `docs/plans/10_solo_orchestration.md`:** phát hiện solo 3 tầng (SOLO RULE trong rules template + `team_context.solo` in-band + server nudge theo `solo_team_threshold_tasks`); tool mới **`co_force_plan_team` (#39)** — heuristic phân cụm parallel lanes theo lock sets rời nhau + reasoner refinement → estimate team có căn cứ (dev/reviewer/qa/ba kèm rationale), trình user trước khi spawn; vòng đời PM quản lý subagents (spawn L2 nhận `taskIds[]` + bootstrap context hẹp chống hallucinate, stall detector báo PM qua inbox, kill/respawn, PM không code khi team chạy, gom câu hỏi trình user 1 lần); chống race cùng máy 2 mức (mặc định: quy tắc git tường minh trong bootstrap prompt / `use_local_worktrees = true`: worktree riêng per task, cô lập tuyệt đối); solo + 1 provider → `reviewer_must_differ="agent"` giữa các **identity** khác nhau + diversity mô hình bù bằng reasoner server-side & reviewer L3 provider khác. Đồng bộ: architecture §6.4 (catalog 39 tools), Plans 03/06/07/09, roadmap §2.1/WS-E (+DoD kịch bản solo).

### F-29 🟡 Handover xuyên provider khi chạm rate limit — flow cũ có 4 lỗ hổng (đã sửa)

Use case chuẩn: Claude CLI + agy CLI cùng làm 1 feature, Claude chạm rate limit subscription → phải chuyển context + task sang agy. Flow handover cũ (Plan 03 §4.2) không đáp ứng: (1) không phân biệt **rate limit** (reset sau N giờ — cần cooldown tracking, khác cạn context); (2) ngầm định context port được — thực tế conversation không mang sang provider khác được (`--resume` là per-provider); (3) bước "nhả toàn bộ locks" tạo cửa sổ cho agent thứ 3 chen vào giữa lúc chuyển giao; (4) không có re-dispatch khi agent chết đột ngột vì hard limit.

**Đã sửa — Plan 03 §5 mới (Cross-Provider Handover):** context externalize qua **5 kênh không phụ thuộc provider** (task record, handover package, activity stream, memory, git state); **handover package có schema** (done/remaining/decisions/gotchas/next_steps/code_state) được **reasoner validate** — thiếu/mơ hồ → lỗi mới `HANDOVER_INCOMPLETE` (bàn giao cẩu thả bị chặn như mọi gate); locks vào **escrow theo task**, chuyển atomic cho người kế nhiệm; **provider cooldown** (`provider_status` trong server.db) — staffing/delegation tránh provider đang limit, hết cooldown tự trở lại; ma trận target (agent khác online → offer inbox / đã push → L3 / local chưa push → L2 cùng máy); **flow bị động**: chết đột ngột → reclaim + tự offer sang provider khác với package server tổng hợp từ activity journal (rules mới: update_task progress notes = bảo hiểm handover). Đồng bộ: architecture §5.4/§6.3/§6.4 tool 23, Plans 07 (service validate)/08 (cooldown awareness)/09 (rules handover-sớm + journal), roadmap WS-A (bảng `handovers`, `provider_status`) + WS-E DoD kịch bản chuẩn.
