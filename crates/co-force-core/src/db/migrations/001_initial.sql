-- Co-Force per-workspace database schema (v1)
-- Applied automatically by `Database::run_migrations_async`.
--
-- Two-tier DB design (F-17):
--   - server.db: api_tokens, workspaces registry, audit_log (Plan 06 §4.1)
--   - This file: per-workspace DB (data/{workspaceId}/co-force.db)

PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;

-- ===== AGENTS =====
CREATE TABLE IF NOT EXISTS agents (
    agent_id     TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    name         TEXT NOT NULL,
    role         TEXT DEFAULT 'developer',
    provider     TEXT,
    machine_id   TEXT NOT NULL,
    state        TEXT DEFAULT 'idle',
    current_task_id TEXT,
    last_seen    TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- ===== TASKS (state machine: Plan 07 §3) =====
CREATE TABLE IF NOT EXISTS tasks (
    task_id      TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    title        TEXT NOT NULL,
    objective    TEXT,
    status       TEXT DEFAULT 'draft',   -- matches TaskStatus enum values
    revision     INTEGER DEFAULT 1,      -- F-21: increments based on server-observed events
    rework_cycle INTEGER DEFAULT 0,      -- Plan 07 §3: escalates if it exceeds limit
    assigned_agent_id       TEXT,
    delegated_from_agent_id TEXT,
    parent_task_id          TEXT,
    use_cases    TEXT,   -- JSON
    prerequisites TEXT,  -- JSON
    verification_plan TEXT, -- JSON
    required_skills   TEXT, -- JSON
    locked_files      TEXT, -- JSON
    impact_analysis   TEXT, -- JSON
    priority     INTEGER DEFAULT 0,
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP,
    FOREIGN KEY (assigned_agent_id) REFERENCES agents(agent_id),
    FOREIGN KEY (parent_task_id)    REFERENCES tasks(task_id)
);

-- ===== FILE LOCKS =====
CREATE TABLE IF NOT EXISTS file_locks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id TEXT NOT NULL,
    file_path    TEXT NOT NULL,
    agent_id     TEXT NOT NULL,
    machine_id   TEXT NOT NULL,
    task_id      TEXT,
    reason       TEXT,
    locked_at    TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    expires_at   TIMESTAMP,
    UNIQUE(workspace_id, file_path),
    FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
);

-- ===== MEMORY ENTRIES (F-02: embedding as BLOB in SQLite) =====
CREATE TABLE IF NOT EXISTS memory_entries (
    entry_id     TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    entry_type   TEXT NOT NULL,  -- 'memory' | 'knowledge' | 'skill'
    content      TEXT NOT NULL,
    source       TEXT,
    agent_id     TEXT,
    confidence   REAL DEFAULT 1.0,
    tags         TEXT,           -- JSON array
    embedding    BLOB,           -- vector stored within the DB; NULL = awaiting re-embedding
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    accessed_at  TIMESTAMP,
    access_count INTEGER DEFAULT 0
);

-- ===== SKILLS =====
CREATE TABLE IF NOT EXISTS skills (
    skill_id     TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    name         TEXT NOT NULL,
    description  TEXT,
    category     TEXT,
    steps        TEXT,           -- JSON array
    source_memories TEXT,        -- JSON array of entry_ids
    usage_count  INTEGER DEFAULT 0,
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- ===== EMBEDDING CACHE =====
CREATE TABLE IF NOT EXISTS embedding_cache (
    content_hash TEXT PRIMARY KEY, -- SHA-256 of content
    embedding    BLOB NOT NULL,
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- ===== AGENT ACTIVITIES (append-only log, from URD Group H) =====
CREATE TABLE IF NOT EXISTS agent_activities (
    activity_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    content TEXT, -- JSON
    related_task_id TEXT,
    related_files TEXT, -- JSON
    version INTEGER DEFAULT 1,
    occurred_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
);

-- ===== SHARED CONTEXTS =====
CREATE TABLE IF NOT EXISTS shared_contexts (
    context_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    source_agent_id TEXT NOT NULL,
    target_agent_id TEXT,
    context_type TEXT NOT NULL,
    content TEXT NOT NULL,
    resolved BOOLEAN DEFAULT FALSE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    resolved_at TIMESTAMP,
    FOREIGN KEY (source_agent_id) REFERENCES agents(agent_id)
);

-- ===== HOT-PATH INDEXES (Master Plan WS-A) =====
CREATE INDEX IF NOT EXISTS idx_activities_ws_time ON agent_activities(workspace_id, occurred_at);
CREATE INDEX IF NOT EXISTS idx_memory_ws_type ON memory_entries(workspace_id, entry_type);
CREATE INDEX IF NOT EXISTS idx_tasks_ws_status ON tasks(workspace_id, status);
CREATE INDEX IF NOT EXISTS idx_file_locks_ws ON file_locks(workspace_id);
CREATE INDEX IF NOT EXISTS idx_agents_ws ON agents(workspace_id);
