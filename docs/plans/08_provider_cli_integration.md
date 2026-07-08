# Detailed Implementation Plan: 08 - Provider CLI Integration (Subscription-first)

**Status:** Ready for Implementation (supports WS-E, WS-F, WS-G — finalized 2026-07-08)
**Target:** `crates/co-force-core/src/orchestration/providers.rs`, `[providers]` configuration block, templates in Plan 05/06
**Research Source:** Deep research 2026-07-08 (official OpenAI/Google documentation) + comparison against **Tutti's** provider architecture (`ref/tutti` — Apache-2.0, multi-agent shared workspace project running on existing user subscriptions)

## 1. Context & Principles

Previous documentation only mentioned Claude Code CLI (and "antigravity-cli" by extrapolation). In reality, users have **multiple parallel agent subscriptions** — Claude (Anthropic), Codex (OpenAI/ChatGPT), Antigravity (Google) — and the core value of Co-Force (cross-reviews, multi-model critiques) is only fully realized when **genuinely diverse providers** collaborate in the same workspace.

| # | Principle | Consequence |
| :- | :--- | :--- |
| P1 | **Subscription-first** | Agent CLIs run on subscriptions the user has already paid for (OAuth/device-flow login), NOT burning API keys for coding work. API keys are strictly optional per-provider fallbacks. |
| P2 | **Declared Registry, Not Hardcoded** (reaffirming F-05) | All provider specs (binary name, flags, MCP configs, auth markers) live in configuration + default registry tables — adding new providers requires no modifications to Rust core. Model learned from Tutti's `ProviderSpec` (`registry.go`). |
| P3 | **Verify at Setup, Do Not Trust Docs** | CLI flags evolve rapidly (the Gemini CLI was shutdown by Google in June 2026, replaced by `agy`). The installer/enrollment script smoke-tests each CLI: binary must be present, login must be valid, and the MCP header must reach the server. |
| P4 | **Diversity is a Feature, Not an Option** | Having ≥ 2 providers in the system makes `reviewer_must_differ = "provider"` genuinely feasible (resolving the gate deadlock F-22); critique fan-outs between Claude ↔ GPT ↔ Gemini behave exactly as designed in Plan 07 §6. |

## 2. Provider Registry — Configuration Schema (implements F-05)

```toml
# server.toml — one block per provider; default table is built-in, this block overrides/extends
[providers.claude-code]
binary_names        = ["claude"]
headless_command    = ["claude", "-p", "{prompt}"]          # L2/L3 spawn template
auto_approve_flags  = ["--permission-mode", "acceptEdits"]   # L2 (developer machine — does not bypass all approvals)
sandbox_bypass_flags = ["--dangerously-skip-permissions"]    # L3 ONLY (worktree sandbox on the server)
resume_flags        = ["--resume", "{session_id}"]
mcp_config_kind     = "claude-json"        # method to write MCP config for this client (Plan 05 §3)
auth_marker_paths   = ["~/.claude.json", "~/.claude/auth.json"]
auth_status_command = ["claude", "auth", "status"]
login_hint          = "claude login (subscription) · claude setup-token (headless, long-term token)"
rules_files         = ["AGENTS.md", "CLAUDE.md"]
placements          = ["L1", "L2", "L3"]
```

Required fields: `binary_names`, `headless_command`, `mcp_config_kind`, `auth_marker_paths`, `placements`. The `ProcessManager` (Plan 03) only reads the registry — no more `match provider` blocks in the Rust code.

## 3. Specifications for the 3 Main Providers (verified 2026-07-08)

| | **Claude Code** | **Codex CLI** | **Antigravity CLI (`agy`)** |
| :--- | :--- | :--- | :--- |
| Vendor / Subscription | Anthropic — Claude Pro/Max | OpenAI — ChatGPT Plus/Pro | Google — Google Account |
| Binary | `claude` | `codex` | `agy` (Gemini CLI successor — Gemini CLI shutdown 2026-06-18) |
| Headless Command (L2/L3) | `claude -p "<prompt>"` | `codex exec --json "<prompt>"` | `agy -p "<prompt>"` (+ `--print-timeout` for long jobs) |
| Auto-approve Flags | `--permission-mode acceptEdits` (L2) / `--dangerously-skip-permissions` (L3) | `--full-auto`; see ⚠️ C1 | `--dangerously-skip-permissions` |
| Resume/handover | `--resume <sessionId>` | `codex exec resume` | `-c` / `--conversation <ID>` |
| MCP Config (co-force server) | `claude mcp add -s local -t http co-force <url> --header "Authorization: Bearer …"` → `~/.claude.json` (machine-scope, F-18 ✓) | `~/.codex/config.toml`: `[mcp_servers.co-force] url = "…" bearer_token_env_var = "CO_FORCE_TOKEN"` — **supports native HTTP + Bearer** | `.agents/mcp_config.json` (per-workspace) or `~/.gemini/config/mcp_config.json` (global), field `serverUrl`; header auth **must be verified during enrollment** (⚠️ C3) |
| Auth Markers (health probe) | `~/.claude.json`, `~/.claude/auth.json` | `~/.codex/auth.json` | System keyring (macOS Keychain / libsecret) — probed via auth status, not a file |
| Headless Login (server L3) | `claude setup-token` | `codex login` (browser — SSH port-forward) or `OPENAI_API_KEY` fallback | Google OAuth SSH flow: CLI prints URL + one-time code, completed in browser on another machine; `ANTIGRAVITY_API_KEY` fallback |
| Rules File Read natively | `AGENTS.md`, `CLAUDE.md` | `AGENTS.md` (native) | `AGENTS.md` (root, prepended to every prompt) + skills under `.agents/skills/` |
| Placements | L1 · L2 · L3 | L1 · L2 · L3 | L1 · L2 · L3 |

Optional Provider (spec exists in registry, disabled by default): **Cursor CLI** (`cursor-agent`, MCP `~/.cursor/mcp.json`, login via Cursor subscription). Adding a new provider is as simple as adding a new config block (P2).

### ⚠️ Verified Caveats (Must be handled in code, do not ignore)

- **C1 — Codex `exec` + MCP approvals:** issue openai/codex#24135 — in `codex exec`, MCP tool calls are automatically cancelled because stdin is closed, and there is no config key to disable the approval prompt; the only bypass is `--dangerously-bypass-approvals-and-sandbox`. **Verdict:** L3 uses this flag (acceptable since the worker runs in an isolated git worktree sandbox on the server, with cgroup limits and no secrets other than the worker token); **L2 on developer machines MUST NOT use this** — the Codex spawn directive for L2 must verify the version/behavior at runtime; if not possible, the server returns `SPAWN_DENIED {reason: "provider_headless_limitation"}` suggesting another provider or L3.
- **C2 — Codex `bearer_token_env_var` reads environment variables** (aligning with F-18): the enrollment script must ensure `CO_FORCE_TOKEN` exists in Codex's env — writing to a managed block in the shell profile (marked, idempotent) and verifying with one real `tools/list` call; if the user refuses to modify their profile → fallback to writing the token directly to `~/.codex/config.toml`? **No** — config.toml does not accept a literal token; fallback is the stdio shim (`mcp_servers.co-force.command = npx mcp-remote <url> --header …`).
- **C3 — `agy` MCP header:** public documentation has not confirmed custom Authorization headers for `serverUrl`; verify this during enrollment — if headers cannot be sent → fall back to the stdio shim as in C2. Mark a TODO to re-verify this on each `agy` release (new CLI, changes rapidly).
- **C4 — Inconsistent Auth Status Command:** `claude auth status` / `codex login status` / agy via keyring — one parser per provider (Tutti `service_helpers.go` model), returning `logged_in | expired | absent`.

## 4. Integration via the 3 Lanes (architecture.md §5)

- **L1 (interactive):** Plan 05 enrollment writes MCP configuration for **every CLI detected on the machine** according to its `mcp_config_kind` (table §3) — if a developer machine has both `claude`, `codex`, and `agy`, all three join the same workspace, each CLI using its own agent identity (sharing the same machine-scope agent token).
- **L2 (spawn-by-directive):** The `spawn_directive` is constructed from `headless_command` + `auto_approve_flags` + env (scoped, short-lived token). The requester runs this via a shell tool — the server selects the provider based on the task requirements + availability on that machine (reported during check-in: the client sends its list of locally available CLIs).
- **L3 (worker pool):** The installer (Plan 06 §3.3) installs + logs into each selected CLI under the `coforce` user (headless login flow per table §3); the `ProcessManager` spawns the CLI with `sandbox_bypass_flags` in a worktree; a health probe (C4) runs every 30 minutes — if the subscription expires → the component `provider.<name>` goes down → returns `SERVICE_UNAVAILABLE` when a spawn is requested + alerts with a re-login command (fail-loud N2, **no** silent fallback to API keys).

## 5. Maximizing Subscription Utility for the Quality Engine

1. **Cross-Review & Genuinely Multi-Model Critique:** The default policy when the system has ≥ 2 providers → enforces `reviewer_must_differ = "provider"`; critique fan-outs prioritize covering Claude + GPT + Gemini before repeating the same provider (Plan 07 §6 — "dissent is a signal").
2. **Reasoner via CLI Worker (Optional, saves API costs):** `reasoner_provider = "cli-worker"` — rechecks/consolidated critiques are routed as L3 jobs running on the subscription instead of calling a cloud reasoner API. Trade-off: higher latency, no streaming; ideal for nightly distillation/consolidation. Embedding/classification **still strictly requires Ollama** (no CLI provider does embedding).
3. **Cost Visibility:** Every spawn logs the provider + duration into `agent_activities`; the dashboard displays usage per provider so users can manage subscription limits (rate limits for Claude Max / ChatGPT Pro are real resources).
4. **Rate-limit Awareness & Cross-Provider Failover (Plan 03 §5):** The `provider_status` table (`server.db`) records `rate_limited_until` per machine/provider — sourced from: the agent declaring it via `handover(reason="rate_limit")` or the stderr parser when L2/L3 workers exit (expanding on C4). During cooldown: plan_team/auto-staffing/delegation **do not assign tasks** to that provider; in-progress tasks are handed over to another provider (standard scenario: Claude limit reached → agy takes over); once the cooldown expires → the provider automatically returns to the pool. The dashboard displays the remaining cooldown.

## 6. Steps to Implement (Step-by-Step)

1. `providers.rs`: implement the `ProviderSpec` struct + default registry for the 4 providers (table §3) + merge overrides from `server.toml [providers]`; unit test template rendering (`{prompt}`, `{cwd}`, `{session_id}` escaping).
2. Auth probes (C4): implement the `AuthStatusParser` trait, 1 implementation per provider, mock-testing against real output (fixtures from current CLIs).
3. Plan 05 script: detect all of `claude`/`codex`/`agy`/`cursor-agent`, write MCP configuration based on its `mcp_config_kind` (golden-file test per kind), verify custom header support (C2/C3), falling back to the stdio shim if unsupported.
4. Plan 06 installer §3.3: implement the login step for each CLI (printing URLs/codes for SSH flows), smoke-testing headless spawning with a simple "ping" prompt per provider.
5. ProcessManager (Plan 03): reads the registry — removing all `match provider` blocks; implements the `spawn_directive` builder for L2 + sandbox spawning for L3 (with C1 gate).
6. Quality Engine hooks (Plan 07): provider-diversity picker for reviews/critiques; `cli-worker` reasoner option.
7. E2E: test one task traversing the entire cycle: Dev (Claude Code) → reviewer (Codex L3) → critique (agy L3) on the test server.
