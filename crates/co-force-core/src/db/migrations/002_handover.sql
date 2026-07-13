-- 002_handover.sql
-- Add handovers and provider_status tables for Active A2A Orchestration

CREATE TABLE IF NOT EXISTS handovers (
    handover_id             TEXT PRIMARY KEY,
    task_id                 TEXT NOT NULL,
    from_agent_id           TEXT NOT NULL,
    to_agent_id             TEXT,              -- NULL until accepted
    reason                  TEXT NOT NULL,     -- rate_limit | context_exhaustion | session_end | manual
    package                 TEXT NOT NULL,     -- JSON validated package
    provider_cooldown_until TIMESTAMP,
    created_at              TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    accepted_at             TIMESTAMP,
    FOREIGN KEY (task_id) REFERENCES tasks(task_id)
);

CREATE TABLE IF NOT EXISTS provider_status (
    machine_id              TEXT NOT NULL,
    provider                TEXT NOT NULL,
    rate_limited_until      TIMESTAMP,
    last_error              TEXT,
    PRIMARY KEY (machine_id, provider)
);
