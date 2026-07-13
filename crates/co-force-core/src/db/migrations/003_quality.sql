-- Migration 003: Quality Engine & Bidirectional Messaging tables (Plan 07)
-- Applied by Database::run_migrations after version check.

PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;

-- ===== AGENT MESSAGES (Bidirectional A2A Messaging — Plan 07 §4) =====
CREATE TABLE IF NOT EXISTS agent_messages (
    message_id      TEXT PRIMARY KEY,
    workspace_id    TEXT NOT NULL,
    from_agent_id   TEXT NOT NULL,
    to_agent_id     TEXT,                    -- NULL = broadcast by role_filter
    role_filter     TEXT,                    -- e.g. 'reviewer' → send to all agents with that role
    kind            TEXT NOT NULL,           -- info|question|review_request|critique_request|review_response|critique_response|answer
    payload         TEXT NOT NULL,           -- JSON schema specific to kind
    correlation_id  TEXT,                    -- links request ↔ response
    requires_response BOOLEAN DEFAULT FALSE,
    created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    delivered_at    TIMESTAMP,
    responded_at    TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_msg_inbox ON agent_messages(workspace_id, to_agent_id, delivered_at);
CREATE INDEX IF NOT EXISTS idx_msg_role ON agent_messages(workspace_id, role_filter, delivered_at);

-- ===== REVIEWS (Plan 07 §5) =====
CREATE TABLE IF NOT EXISTS reviews (
    review_id           TEXT PRIMARY KEY,
    task_id             TEXT NOT NULL,
    workspace_id        TEXT NOT NULL,
    task_revision       INTEGER NOT NULL,
    reviewer_agent_id   TEXT NOT NULL,
    verdict             TEXT NOT NULL,       -- approved | changes_requested
    findings            TEXT,               -- JSON [{file, line, severity, issue, suggestion}]
    assist_checklist    TEXT,               -- JSON — LLM-generated checklist for reviewer
    created_at          TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_reviews_task ON reviews(task_id, task_revision);

-- ===== CRITIQUES (Plan 07 §6) =====
CREATE TABLE IF NOT EXISTS critiques (
    critique_id     TEXT PRIMARY KEY,
    workspace_id    TEXT NOT NULL,
    subject         TEXT NOT NULL,
    content         TEXT NOT NULL,
    critic_agent_id TEXT NOT NULL,
    position        TEXT NOT NULL,          -- agree | disagree
    arguments       TEXT,                   -- JSON array
    risks           TEXT,                   -- JSON array
    alternatives    TEXT,                   -- JSON array
    consolidation   TEXT,                   -- Reasoner LLM output
    correlation_id  TEXT,
    created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- ===== VERIFICATION RECORDS (Plan 07 §5.1) =====
CREATE TABLE IF NOT EXISTS verification_records (
    verification_id TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    task_revision   INTEGER NOT NULL,
    commit_sha      TEXT,                   -- required when git remote is configured
    steps           TEXT NOT NULL,          -- JSON [{kind, command, exit_code, summary, output_digest}]
    submitted_by    TEXT NOT NULL,
    created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_verification_task ON verification_records(task_id, task_revision);

-- ===== QUALITY POLICIES (Plan 07 §8) =====
CREATE TABLE IF NOT EXISTS quality_policies (
    policy_id               TEXT PRIMARY KEY,
    workspace_id            TEXT NOT NULL UNIQUE,  -- one policy per workspace
    reviews_required        INTEGER DEFAULT 1,
    reviewer_must_differ    TEXT DEFAULT 'agent',  -- agent | provider
    require_recheck         BOOLEAN DEFAULT TRUE,
    require_verification_evidence BOOLEAN DEFAULT TRUE,
    required_evidence_kinds TEXT DEFAULT '["test"]',  -- JSON array
    critique_fanout         INTEGER DEFAULT 2,
    max_rework_cycles       INTEGER DEFAULT 3,
    definition_of_done      TEXT DEFAULT '[]',     -- JSON array of strings
    created_at              TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at              TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- ===== QUALITY SCORES (Plan 07 §9) =====
CREATE TABLE IF NOT EXISTS quality_scores (
    score_id            TEXT PRIMARY KEY,
    task_id             TEXT NOT NULL,
    workspace_id        TEXT NOT NULL,
    rework_cycles       INTEGER DEFAULT 0,
    findings_count      INTEGER DEFAULT 0,
    findings_by_severity TEXT,              -- JSON {critical:N, major:N, minor:N}
    review_coverage     BOOLEAN DEFAULT FALSE,
    evidence_integrity  BOOLEAN DEFAULT FALSE,
    duration_secs       INTEGER,
    completed_at        TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_quality_scores_ws ON quality_scores(workspace_id, completed_at);
