# Co-Force: Subagent Progress Tracker

> **QUAN TRỌNG:** Đây là file đồng bộ trạng thái (Keep Track) trung tâm. 
> Bất kỳ Agent hay Subagent (PM, DEV, TEST, QA) nào khi bắt đầu công việc ĐỀU PHẢI đọc file này để tránh Race Condition. Khi bắt đầu một task, phải ghi rõ `[Đang xử lý bởi <Tên Subagent>]`. Khi xong phải đánh dấu `[x]` và cập nhật kết quả.

## Tiêu chuẩn Trạng thái:
- `[ ]` Chưa bắt đầu
- `[Đang xử lý bởi PM]` Đang lên kế hoạch/phân tích
- `[Đang xử lý bởi DEV]` Đang code hoặc viết Test (TDD)
- `[Đang xử lý bởi TEST]` Đang chạy test/kiểm thử
- `[Đang xử lý bởi QA]` Đang review/linting
- `[x]` Hoàn thành

---

## Sprint Hiện tại: Triển khai Kế hoạch Kiến trúc

> **Cập nhật 2026-07-08 (v2):** Định hướng chốt bởi product owner: **một release 1.0 end-to-end product-ready, KHÔNG có MVP**; server độc lập + **cloudflared tunnel** + auth token; **Ollama bắt buộc** (không degraded mode); client setup one-liner < 60s; trung tâm sản phẩm là **Quality Engine** (review chéo, verification evidence, critique — Plan 07). Thứ tự triển khai theo workstreams WS-A…WS-I trong `docs/plans/00_roadmap.md` (Master Plan). Lưu ý cho DEV: **rmcp 2.x** (API `tool_router`/`ServerHandler`, streamable-http thay SSE), **không dùng embedvec** (vector BLOB trong SQLite).

### 0. Foundation — làm workspace compile (Phase 0, ưu tiên trước tất cả)
- `[ ]` Sửa `co-force-core/src/lib.rs` (module `db`, `workspace`, `engine`, `ollama` đang khai báo nhưng chưa có file)
- `[ ]` Tạo `co-force-mcp/src/main.rs` tối thiểu (hiện thiếu → `cargo check` fail)
- `[ ]` CI: cargo test + clippy -D warnings + fmt --check

### 1. Database and Domain Layer (Plan 01)
- `[ ]` Setup Cargo.toml dependencies (serde, tokio, rusqlite, mockall)
- `[ ]` Định nghĩa Strong Types (AgentId, TaskId, WorkspaceId...) trong `types/mod.rs`
- `[ ]` Định nghĩa Enums (AgentState, TaskStatus, ActivityType...)
- `[ ]` Định nghĩa Core Structs (Agent, Task, AgentActivity, SharedContext)
- `[ ]` Triển khai SQLite Migrations (001_initial.sql)
- `[ ]` Định nghĩa Repository Traits (AgentRepository, LockRepository...) trong `engine/ports.rs`

### 2. MCP Server and Use Cases (Plan 02)
- `[ ]` Xây dựng `CheckInUseCase` (TDD, Unit Test trước)
- `[ ]` Xây dựng `LockFilesUseCase`
- `[ ]` Setup `co-force-mcp/src/main.rs` với struct `CoForceMcp`
- `[ ]` Đăng ký các tool handlers (rmcp 2.x: `#[tool_router]` + `#[tool]` + `ServerHandler`)
- `[ ]` Cấu hình Transport Layer (stdio / streamable-http) dựa trên CLI args

### 3. Active A2A Orchestration (Plan 03)
- `[ ]` Khởi tạo In-Memory Event Bus (`tokio::sync::broadcast`)
- `[ ]` Viết module Dynamic AGENTS.md Generator (`doc_generator.rs`)
- `[ ]` Viết module Process Manager (`process_mgr.rs`) để spawn lệnh OS
- `[ ]` Triển khai tool `co_force_spawn_agent`
- `[ ]` Triển khai tool `co_force_handover` — **cross-provider theo Plan 03 §5**: bảng `handovers` + package validator (reasoner) + lock escrow chuyển atomic + `HANDOVER_INCOMPLETE`
- `[ ]` Provider cooldown (`provider_status` server.db) + stderr rate-limit parser per provider; staffing/delegation tránh provider đang limit
- `[ ]` Reclaim mở rộng: re-dispatch tự động sang provider khác + package server tổng hợp từ activity journal
- `[ ]` Integration test kịch bản chuẩn: claude rate_limit → handover → agy tiếp quản (chủ động + bị động kill -9), không rơi gate

### 4. Agentic RAG and LLM (Plan 04)
- `[ ]` Định nghĩa `LlmProvider` interface
- `[ ]` Triển khai `OllamaProvider` (reqwest `/api/embeddings`, `/api/generate`)
- `[ ]` Viết thuật toán `agentic_chunking` (Structural Splitting; Semantic Boundary là stretch goal)
- `[ ]` Vector Search brute-force cosine (BLOB trong SQLite, trait `VectorSearch`) + Fallback logic vào Memory Use Case

### 5. Client Setup & Onboarding (Plan 05 v2 — WS-G)
- `[ ]` Endpoint `/api/enroll`: enrollment token (TTL 24h) → agent token dài hạn per máy
- `[ ]` Endpoint `/setup`: serve script sh/ps1 templated theo `public_url`
- `[ ]` Script client: detect IDE, ghi config **machine-scope** (`claude mcp add -s local` / `~/.cursor/mcp.json` — token per-máy KHÔNG vào file project, F-18; fallback CI mới ghi `.mcp.json` + gitignore-trước-token), rule injection quality protocol, verify tools/list
- `[ ]` E2E: container sạch → one-liner → check-in thành công < 60s

### 6. Server Deployment & Ops (Plan 06 — WS-F)
- `[ ]` `server.db` cấp server (`api_tokens`, `workspaces` registry, `audit_log` — F-17) + AuthLayer (Bearer, rate limit, audit) — làm trước, WS-B phụ thuộc
- `[ ]` Axum router hợp nhất: /mcp + /api + /dashboard + /setup + /healthz (1 port, bind 127.0.0.1)
- `[ ]` Installer `co-force-server install` (checkpoint/resume): Ollama + pull models + verify, cloudflared tunnel + DNS, systemd units + hardening
- `[ ]` Health model per-component, fail-loud `SERVICE_UNAVAILABLE`, alert webhook
- `[ ]` Backup timer + restore + upgrade path; admin CLI (token/status/backup/restore/upgrade)

### 7. Quality Engine & A2A hai chiều (Plan 07 — WS-C, critical path)
- `[ ]` Migrations: agent_messages, reviews, critiques, verification_records, quality_policies, quality_scores
- `[ ]` Task state machine mới (pure function + unit test đầy đủ): draft → spec_review → awaiting_approval → approved → in_progress → verification → code_review → completed (+ rework/blocked/handover)
- `[ ]` Messaging: send/respond + inbox piggyback trên mọi tool response + `wait_events` long-poll 55s
- `[ ]` Review workflow: request/assign (separation of duties)/submit/rework + auto-staffing
- `[ ]` Verification evidence validator + task revision tracking (chống "đã test rồi" giả)
- `[ ]` LLM services (reasoner): spec recheck, review assist, distillation, consolidation (prompt templates có version)
- `[ ]` Critique fan-out + tổng hợp bất đồng; quality scores + metrics API
- `[ ]` Integration test kịch bản "3 agents như một team" (Master Plan §6.1)

### 8. Provider CLI Integration (Plan 08 — bổ trợ WS-E/F/G)
- `[ ]` `providers.rs`: `ProviderSpec` registry mặc định (claude/codex/agy/cursor-agent) + merge override từ `server.toml [providers]` + test template rendering
- `[ ]` Auth-status parsers per provider (C4) + health component `provider.<cli>`
- `[ ]` Enrollment script: detect + ghi MCP config per CLI kind (codex toml / agy json / claude cmd), verify header (C2/C3), stdio shim fallback
- `[ ]` Installer: login subscription headless per CLI + smoke test spawn
- `[ ]` L2 spawn directive builder + C1 gate (Codex exec MCP approvals); L3 sandbox bypass flags
- `[ ]` Quality: provider-diversity picker (review/critique); option `reasoner_provider = "cli-worker"`

### 9. Agent Operating Protocol (Plan 09 — bổ trợ WS-B/C/G)
- `[ ]` Rules template v1 (`workspace/protocol_templates/rules_v1.md`) + managed-block writer + golden-file test render
- `[ ]` Chuẩn hóa 38 tool descriptions theo Plan 09 §3 (đồng bộ với template, review chéo)
- `[ ]` `co_force_guide` renderer động (policy + team + backlog + 3 ví dụ đúng chuẩn + playbook theo role)
- `[ ]` Cờ `onboarding: true` trong check_in đầu tiên + `protocol_next_step` trỏ guide
- `[ ]` E2E "cold agent": enroll → prompt trung tính → agent tự check_in → recall → create_tasks → dừng chờ approve (lặp với claude/codex/agy)

### 10. Solo Orchestration & Team Bootstrap (Plan 10 — bổ trợ WS-C/E)
- `[ ]` `team_planner.rs`: heuristic phân cụm parallel lanes (lock sets rời nhau + prerequisites) + reasoner refinement (mock LLM test)
- `[ ]` Tool `co_force_plan_team` (#39) + `team_context.solo` trong check_in + solo nudge theo `solo_team_threshold_tasks`
- `[ ]` Mở rộng spawn L2: `taskIds[]`, bootstrap prompt hẹp + quy tắc git tường minh; option `use_local_worktrees` (worktree per task trên máy client)
- `[ ]` Stall detector daemon (in_progress không activity > `stall_timeout_secs` → inbox PM) + spawn record (PID, kill/respawn)
- `[ ]` Policy: solo/1-provider → `reviewer_must_differ="agent"` + gợi ý reviewer L3 provider khác
- `[ ]` E2E solo: 1 agent agy + 8 tasks 2 lanes → tự plan_team → spawn 2 dev + 1 reviewer → đủ gates, không race; kill 1 subagent → PM respawn

---

## Log Báo cáo (Subagent Reports)
*(Các subagent ghi chú lỗi, kết quả test, hoặc report cho Agent gốc tại đây)*
- **[Hệ thống]**: Khởi tạo file tracking. Sẵn sàng cho PM subagent phân bổ việc.
- **[Review 2026-07-08]**: Hoàn thành review docs tổng thể. Verified thực tế: rmcp = 2.1.0 (docs cũ ghi 0.16, Cargo.toml pin 0.1 — đã sửa lên "2"); embedvec tồn tại nhưng adoption quá thấp → loại; `gemma4:e2b` + `mxbai-embed-large` verified có trên Ollama. Tạo mới: `review_findings.md`, `architecture.md`, `plans/00_roadmap.md`, `plans/05_setup_ux_and_onboarding.md`. Khả thi tổng thể: ~85%. Cargo manifest resolve OK với rmcp 2.
- **[Architecture v2.1 2026-07-08]**: Bổ sung 2 section còn thiếu vào `architecture.md`: **§5 Mô hình thực thi A2A production** (3 lanes: L1 interactive client / L2 spawn-by-directive — server trả lệnh, agent tự chạy vì tunnel một chiều / L3 server worker pool — headless agent trên server đọc code qua git mirror + worktree sandbox; ma trận placement; sequence end-to-end Dev↔Worker review) và **§6 MCP Tool Interface** (vòng đời kết nối, response envelope với inbox piggyback + protocol_next_step, 9 error codes chuẩn, catalog đầy đủ 38 tools). Đồng bộ Plan 03 (banner 3-lane), Plan 06 (§3.3 worker pool provisioning + config `[workers]`), Plan 07 (evidence thêm `commit_sha`). DEV lưu ý: `submit_verification` bắt buộc `commit_sha` khi workspace có git; worker không bao giờ push main.
- **[Architecture v2.2 2026-07-08]**: Sửa 2 điểm vô lý theo feedback product owner: (1) **Server luôn headless** (bare-metal systemd hoặc **Docker Compose** — Plan 06 §2.1 mới, kèm compose file: co-force + ollama + cloudflared token-mode); Tauri là app **phía client** (crates đổi hướng thành `co-force-app`, chỉ gọi HTTPS qua tunnel). (2) **Xóa mọi luồng server-ghi-file-sang-client** (bất khả thi với tunnel một chiều): rules/config tĩnh do enrollment script ghi 1 lần; state động (locks/tasks/team/inbox) đi **in-band** qua response envelope (`workspace_pulse` + `inbox`); `session_status.json` bị bỏ trong production (chỉ còn ở LAN mode); doc-generator chỉ ghi vào worker worktrees (server FS) + serve qua `/api/workspaces/{id}/agents.md`. Lớp 4 guardrail đổi hình thái từ "file cục bộ" → "in-band state" (architecture.md §5.6). Đồng bộ: sơ đồ §1 (thêm node Enrollment, sửa luồng), §2 (client app), §3 (doc-gen), §7 (client layout chỉ còn agent.json + token), Plan 03 banner.
- **[Cross-Provider Handover v2.8 2026-07-08]**: Lấp gap F-29 — use case "Claude CLI chạm rate limit giữa feature → agy CLI tiếp quản". Viết **Plan 03 §5 mới**: context externalize qua 5 kênh không phụ thuộc provider (conversation KHÔNG port được giữa CLI — task record + handover package + activity stream + memory + git state); package schema validated bằng reasoner (thiếu remaining/next_steps → `HANDOVER_INCOMPLETE`, error code mới); **lock escrow** chuyển atomic (sửa lỗ hổng flow cũ nhả locks tự do); **provider cooldown** `provider_status` per máy/provider (tự khai qua handover reason=rate_limit hoặc stderr parser) — staffing tránh provider đang limit; ma trận target (online offer / L3 pushed / L2 cùng máy khi chưa push); flow bị động: chết đột ngột → re-dispatch + package server tổng hợp (rules mới: update_task journal = bảo hiểm handover, handover SỚM khi thấy warning đầu tiên). Đồng bộ: architecture §5.4/§6.3/tool 23, Plans 07/08/09, roadmap WS-A/WS-E DoD, tracker section 3.
- **[Solo Orchestration v2.7 2026-07-08]**: Lấp gap F-28 — kịch bản "1 agent (agy) + việc dài nhiều task → context phình → hallucinate". Tạo **`plans/10_solo_orchestration.md`**: solo detection 3 tầng (SOLO RULE trong rules + `team_context.solo` in-band + server nudge); tool #39 `co_force_plan_team` (heuristic lanes theo lock sets rời nhau + reasoner → estimate dev/reviewer/qa/ba kèm rationale, user xác nhận trước khi spawn); PM lifecycle (spawn L2 bootstrap hẹp, stall detector, kill/respawn, PM không code); chống race cùng máy 2 mức (git tường minh / `use_local_worktrees`); solo 1-provider → review chéo giữa identities + diversity bù bằng reasoner/L3. Catalog lên **39 tools**. Đồng bộ: architecture §6.4, Plans 03/06 (`[a2a]` 4 keys mới)/07/09 (SOLO RULE + playbook PM), roadmap §2.1–2.2/WS-E, tracker section 10.
- **[Agent Operating Protocol v2.6 2026-07-08]**: Lấp gap F-27 — chưa có đặc tả "agent làm gì sau khi setup xong". Tạo **`plans/09_agent_operating_protocol.md`**: chuỗi khám phá 4 điểm chạm của agent lạnh (rules file → tool descriptions → check_in response → envelope mọi response); **rules template v1 chốt** (managed block tiếng Anh — điểm khởi đầu check_in, vòng đời task theo gates, quy tắc hành vi đồng nhất, bảng "tool nào khi nào"; nguyên tắc: chỉ hứa những gì server thật sự enforce); chuẩn 38 tool descriptions (Lớp 2); spec `co_force_guide` động + `onboarding: true`; playbook 4 roles; ma trận quy-tắc ↔ lớp-enforce (hành vi đồng nhất không dựa thiện chí LLM); E2E "cold agent" làm nghiệm thu. URD §9.3 + §10 gắn note thay thế (template cũ dọa "OS Permission Denied" sai, flow cũ thiếu gates); Plan 05 §3.5/§4, architecture §6, roadmap WS-B/WS-G trỏ về Plan 09. Thêm section 9 vào tracker.
- **[Docs cleanup v2.5 2026-07-08]**: Xóa `docs/implementation_instructions.md` (F-26) — file chứa chỉ dẫn sai không cứu được: rmcp 0.16/SSE, "Ollama optional" (trái N2), 2 port dashboard (trái F-13), classify fallback keyword (silent degradation), schema cũ có `vector_id` (trái F-02), deployment SSE. Nội dung giá trị đã chuyển: schema SQL đầy đủ → Plan 01 §3 (cập nhật `embedding BLOB`, `tasks.revision`/`rework_cycle`, TaskStatus theo Plan 07, indexes); tham chiếu Plans 01/02/04 trỏ về architecture.md; URD banner cập nhật (thêm Plan 08 + 38 tools + ghi chú xóa file). Kiểm kê còn lại: URD (giữ — nguồn use case/Appendix B, có banner thứ tự ưu tiên), review_findings/progress (living docs), plans 00–08, architecture. `ref/tutti` được gitignore đúng. **DEV không còn đọc implementation_instructions.md — mọi hướng dẫn implement nằm trong plans 01–08.**
- **[Provider CLI research v2.4 2026-07-08]**: Docs trước đây chỉ nhắc Claude Code CLI (F-25). Deep research (docs chính thức OpenAI/Google, xác minh 2026-07-08) + đối chiếu `ref/tutti` (mô hình ProviderSpec registry, Codex qua app-server, ACP adapters). Tạo mới **`plans/08_provider_cli_integration.md`**: subscription-first (login OAuth, không đốt API key), registry khai báo trong config, spec verified — Codex CLI (`codex exec --json`, MCP HTTP + `bearer_token_env_var` native, auth `~/.codex/auth.json`), Antigravity CLI `agy` (kế nhiệm Gemini CLI đã shutdown 6/2026; `agy -p`, `--dangerously-skip-permissions`, MCP `.agents/mcp_config.json`, Google OAuth keyring), + 4 caveats C1–C4 (nổi bật: Codex exec auto-cancel MCP approvals → chỉ bypass trong L3 sandbox). Đồng bộ: architecture §1/§5 (client nodes + provider list), Plan 03 (registry ref), Plan 05 (bảng config thêm codex/agy + detect + machineInfo.clis), Plan 06 (§3.3 subscription login headless per CLI, `[workers].providers` 3 CLI, health `provider.<cli>`), Plan 07 (diversity picker 3 hãng, option reasoner `cli-worker`), roadmap. DEV lưu ý: ≥2 providers mở khóa `reviewer_must_differ="provider"` (gỡ F-22).
- **[Review v2.3 2026-07-08]**: Vòng review thứ 3 tìm điểm bất khả thi còn sót — 9 findings mới (F-16…F-24, chi tiết `review_findings.md` §7). 3 lỗi 🔴 đã sửa docs: (1) Docker Compose bind 127.0.0.1 → cloudflared không với tới được (Plan 06 §2.1: bind 0.0.0.0 trong container, cô lập bằng compose network); (2) `api_tokens` không thể nằm trong DB per-workspace vì auth chạy trước khi biết workspace → thêm `server.db` cấp server (architecture §7, Plan 06 §4.1, WS-A); (3) cơ chế token qua `.mcp.json` env-expansion không hoạt động (env var ≠ file) + token per-máy không được vào file project commit → chuyển sang config machine-scope (`claude mcp add -s local`, `~/.cursor/mcp.json` — Plan 05 §3). 🟡 đã sửa: Plan 04 dùng `/api/embed` (endpoint cũ deprecated) + chốt hành vi per-tool khi LLM down theo N2; state machine thêm đường ra cho `blocked`/`pending_handover`, reject cho `awaiting_approval`, thêm `cancelled` (Plan 07 §3); revision tracking định nghĩa lại theo sự kiện server quan sát được + verify `commit_sha` trong mirror (Plan 07 §5.1); validate policy `reviewer_must_differ="provider"` lúc set để tránh deadlock gate (Plan 07 §8). 🟢: dọn API cũ trong sample Plan 02/03, progress.md; `wait_events` default 25s vì timeout tool-call phía client.
- **[Direction pivot 2026-07-08 v2]**: Product owner chốt: no-MVP (1 release end-to-end), server độc lập + cloudflared, Ollama bắt buộc/không degraded mode, client one-liner, mục tiêu = chất lượng cực hạn (không phải tốc độ). Viết lại Master Plan (`00_roadmap.md` v2, 9 workstreams, ~10–12 tuần), tạo `plans/06_server_deployment_and_tunnel.md` + `plans/07_quality_engine_and_a2a.md`, viết lại `plans/05` (client < 60s), cập nhật `architecture.md` v2 + `review_findings.md` §6. Bổ sung 38 MCP tools (thêm nhóm Quality + Messaging), 6 bảng DB mới, vai trò model thứ 3 (reasoner).
