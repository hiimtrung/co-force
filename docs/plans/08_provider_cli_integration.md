# Kế Hoạch Triển Khai Chi Tiết: 08 - Provider CLI Integration (Subscription-first)

**Status:** Ready for Implementation (bổ trợ WS-E, WS-F, WS-G — chốt 2026-07-08)
**Target:** `crates/co-force-core/src/orchestration/providers.rs`, config `[providers]`, templates trong Plan 05/06
**Nguồn nghiên cứu:** deep research 2026-07-08 (docs chính thức OpenAI/Google) + đối chiếu kiến trúc provider của **Tutti** (`ref/tutti` — Apache-2.0, dự án shared-workspace đa agent chạy trên subscription sẵn có)

## 1. Context & Nguyên tắc

Tài liệu trước đây chỉ nhắc Claude Code CLI (và "antigravity-cli" phỏng đoán). Thực tế user có **nhiều subscription agent** song song — Claude (Anthropic), Codex (OpenAI/ChatGPT), Antigravity (Google) — và giá trị cốt lõi của Co-Force (review chéo, critique đa mô hình) chỉ phát huy tối đa khi **các provider khác nhau thật sự** tham gia cùng workspace.

| # | Nguyên tắc | Hệ quả |
| :- | :--- | :--- |
| P1 | **Subscription-first** | Agent CLIs chạy bằng subscription user đã trả (login OAuth/device-flow), KHÔNG đốt API key cho công việc code. API key chỉ là fallback tùy chọn per-provider. |
| P2 | **Registry khai báo, không hardcode** (tái khẳng định F-05) | Mọi spec provider (binary, flags, MCP config, auth markers) nằm trong config + bảng registry mặc định — thêm provider mới không sửa core Rust. Mô hình học từ Tutti `ProviderSpec` (registry.go). |
| P3 | **Verify lúc setup, không tin docs** | CLI flags trôi nhanh (Gemini CLI đã bị Google shutdown 6/2026, thay bằng `agy`). Installer/enrollment smoke-test từng CLI: binary có, login còn hạn, MCP header đi tới nơi. |
| P4 | **Diversity là tính năng, không phải tùy chọn** | ≥ 2 provider trong hệ → `reviewer_must_differ = "provider"` khả thi thực (gỡ deadlock F-22); critique fan-out Claude ↔ GPT ↔ Gemini đúng như thiết kế Plan 07 §6. |

## 2. Provider Registry — schema config (hiện thực hóa F-05)

```toml
# server.toml — mỗi provider một block; bảng mặc định built-in, block này để override/thêm
[providers.claude-code]
binary_names        = ["claude"]
headless_command    = ["claude", "-p", "{prompt}"]          # L2/L3 spawn template
auto_approve_flags  = ["--permission-mode", "acceptEdits"]   # L2 (máy dev — không bypass hết)
sandbox_bypass_flags = ["--dangerously-skip-permissions"]    # CHỈ L3 (worktree sandbox trên server)
resume_flags        = ["--resume", "{session_id}"]
mcp_config_kind     = "claude-json"        # cách ghi MCP config cho client này (Plan 05 §3)
auth_marker_paths   = ["~/.claude.json", "~/.claude/auth.json"]
auth_status_command = ["claude", "auth", "status"]
login_hint          = "claude login (subscription) · claude setup-token (headless, token dài hạn)"
rules_files         = ["AGENTS.md", "CLAUDE.md"]
placements          = ["L1", "L2", "L3"]
```

Trường bắt buộc: `binary_names`, `headless_command`, `mcp_config_kind`, `auth_marker_paths`, `placements`. ProcessManager (Plan 03) chỉ đọc registry — không còn `match provider` trong Rust.

## 3. Bảng spec 3 providers chính (verified 2026-07-08)

| | **Claude Code** | **Codex CLI** | **Antigravity CLI (`agy`)** |
| :--- | :--- | :--- | :--- |
| Hãng / subscription | Anthropic — Claude Pro/Max | OpenAI — ChatGPT Plus/Pro | Google — tài khoản Google |
| Binary | `claude` | `codex` | `agy` (kế nhiệm Gemini CLI — Gemini CLI shutdown 2026-06-18) |
| Headless (L2/L3) | `claude -p "<prompt>"` | `codex exec --json "<prompt>"` | `agy -p "<prompt>"` (+ `--print-timeout` cho job dài) |
| Auto-approve | `--permission-mode acceptEdits` (L2) / `--dangerously-skip-permissions` (L3) | `--full-auto`; xem ⚠️ C1 | `--dangerously-skip-permissions` |
| Resume/handover | `--resume <sessionId>` | `codex exec resume` | `-c` / `--conversation <ID>` |
| MCP config (co-force server) | `claude mcp add -s local -t http co-force <url> --header "Authorization: Bearer …"` → `~/.claude.json` (machine-scope, F-18 ✓) | `~/.codex/config.toml`: `[mcp_servers.co-force] url = "…" bearer_token_env_var = "CO_FORCE_TOKEN"` — **hỗ trợ streamable HTTP + Bearer native** | `.agents/mcp_config.json` (per-workspace) hoặc `~/.gemini/config/mcp_config.json` (global), field `serverUrl`; header auth **cần verify lúc enroll** (⚠️ C3) |
| Auth markers (health probe) | `~/.claude.json`, `~/.claude/auth.json` | `~/.codex/auth.json` | System keyring (macOS Keychain / libsecret) — probe bằng auth status, không phải file |
| Login headless (server L3) | `claude setup-token` | `codex login` (browser — SSH port-forward) hoặc `OPENAI_API_KEY` fallback | Google OAuth SSH flow: CLI in URL + one-time code, hoàn tất trên browser máy khác; `ANTIGRAVITY_API_KEY` fallback |
| Rules file agent tự đọc | `AGENTS.md`, `CLAUDE.md` | `AGENTS.md` (native) | `AGENTS.md` (root, prepend mọi prompt) + skills `.agents/skills/` |
| Placements | L1 · L2 · L3 | L1 · L2 · L3 | L1 · L2 · L3 |

Provider phụ (registry có sẵn spec, tắt mặc định): **Cursor CLI** (`cursor-agent`, MCP `~/.cursor/mcp.json`, login subscription Cursor). Thêm provider mới = thêm block config (P2).

### ⚠️ Caveats đã xác minh (phải xử lý trong code, không được lờ)

- **C1 — Codex `exec` + MCP approvals:** issue openai/codex#24135 — trong `codex exec`, MCP tool call bị auto-cancel vì stdin đóng, không có config key tắt approval prompt; bypass duy nhất là `--dangerously-bypass-approvals-and-sandbox`. **Chốt:** L3 dùng flag này (chấp nhận được: worker chạy trong git worktree sandbox cô lập trên server, cgroup limit, không có secrets ngoài worker token); **L2 trên máy dev KHÔNG dùng** — spawn directive Codex cho L2 phải kiểm tra version/behavior lúc runtime, không được → server trả `SPAWN_DENIED {reason: "provider_headless_limitation"}` gợi ý dùng provider khác hoặc L3.
- **C2 — Codex `bearer_token_env_var` đọc biến môi trường** (đúng bài học F-18): enrollment script phải đảm bảo `CO_FORCE_TOKEN` tồn tại trong env của codex — ghi vào managed block trong shell profile (đánh dấu, idempotent) và verify bằng 1 lần `tools/list` thật; user từ chối sửa profile → fallback ghi token thẳng vào `~/.codex/config.toml`? **Không** — config.toml không nhận literal token; fallback là stdio shim (`mcp_servers.co-force.command = npx mcp-remote <url> --header …`).
- **C3 — `agy` MCP header:** tài liệu công khai chưa khẳng định custom Authorization header cho `serverUrl`; enrollment verify thật — không gửi được header → dùng stdio shim như C2 fallback. Đánh dấu TODO re-verify mỗi release agy (CLI mới, đổi nhanh).
- **C4 — Auth status không đồng nhất:** `claude auth status` / `codex login status` / agy qua keyring — mỗi provider 1 parser (mô hình Tutti `service_helpers.go`), trả về `logged_in | expired | absent`.

## 4. Tích hợp theo 3 lane (architecture.md §5)

- **L1 (interactive):** Plan 05 enrollment ghi MCP config cho **mọi CLI phát hiện được trên máy** theo `mcp_config_kind` (bảng §3) — một máy dev có cả `claude` + `codex` + `agy` thì cả 3 vào cùng workspace, mỗi CLI một agent identity (cùng agent token của máy).
- **L2 (spawn-by-directive):** `spawn_directive` dựng từ `headless_command` + `auto_approve_flags` + env (scoped token TTL ngắn). Requester chạy bằng shell tool — server chọn provider theo yêu cầu task + availability máy đó (report lúc check-in: client gửi danh sách CLI local).
- **L3 (worker pool):** installer (Plan 06 §3.3) cài + login từng CLI được chọn dưới user `coforce` (login flow headless theo bảng §3); ProcessManager spawn với `sandbox_bypass_flags` trong worktree; health probe C4 chạy 30 phút/lần — subscription hết hạn → component `provider.<name>` = down → `SERVICE_UNAVAILABLE` khi cần spawn + alert kèm lệnh re-login (fail-loud N2, **không** âm thầm chuyển sang API key).

## 5. Tận dụng tối đa subscription cho Quality Engine

1. **Review chéo & critique thật sự đa mô hình:** policy mặc định khi hệ có ≥ 2 providers → nâng `reviewer_must_differ = "provider"`; critique fan-out ưu tiên phủ Claude + GPT + Gemini trước khi lặp cùng provider (Plan 07 §6 — "bất đồng là tín hiệu").
2. **Reasoner qua CLI worker (tùy chọn, tiết kiệm API cost):** `reasoner_provider = "cli-worker"` — recheck/critique-tổng-hợp route thành job L3 chạy trên subscription thay vì gọi API reasoner. Trade-off: latency cao hơn, không streaming; phù hợp nightly distillation/consolidation. Embedding/classifier **vẫn bắt buộc Ollama** (không CLI nào làm embedding).
3. **Cost visibility:** mỗi spawn ghi provider + duration vào `agent_activities`; dashboard hiển thị usage per provider để user cân đối hạn mức subscription (rate limit của Claude Max / ChatGPT Pro là tài nguyên thật).
4. **Rate-limit awareness & cross-provider failover (Plan 03 §5):** bảng `provider_status` (server.db) ghi `rate_limited_until` per máy/provider — nguồn: agent tự khai khi `handover(reason="rate_limit")` hoặc stderr parser khi L2/L3 worker exit (mở rộng C4). Trong cooldown: plan_team/auto-staffing/delegation **không giao việc** cho provider đó; task đang dở được handover sang provider khác (kịch bản chuẩn: Claude limit → agy tiếp quản); hết cooldown → provider tự trở lại pool. Dashboard hiển thị cooldown còn lại.

## 6. Trình tự Triển khai (Step-by-Step)

1. `providers.rs`: struct `ProviderSpec` + registry mặc định 4 providers (bảng §3) + merge override từ `server.toml [providers]`; unit test template rendering (`{prompt}`, `{cwd}`, `{session_id}` escaping).
2. Auth probes (C4): trait `AuthStatusParser`, 1 impl/provider, mock test với output thật (fixture từ CLI hiện hành).
3. Plan 05 script: detect đủ `claude`/`codex`/`agy`/`cursor-agent`, ghi MCP config theo `mcp_config_kind` (golden-file test per kind), verify header per client (C2/C3), fallback stdio shim.
4. Plan 06 installer §3.3: bước login từng CLI (in URL/code cho flow SSH), smoke test spawn headless 1 prompt "ping" per provider.
5. ProcessManager (Plan 03) đọc registry — xóa mọi `match provider`; spawn_directive builder cho L2 + sandbox spawn cho L3 (kèm C1 gate).
6. Quality Engine hooks (Plan 07): provider-diversity picker cho review/critique; option `cli-worker` reasoner.
7. E2E: 1 task đi đủ vòng Dev (Claude Code) → reviewer (Codex L3) → critique (agy L3) trên server test.
