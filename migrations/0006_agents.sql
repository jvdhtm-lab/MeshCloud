-- Phase 6: agent registry, capabilities, and messages.

CREATE TABLE IF NOT EXISTS agents (
    agent_id       TEXT PRIMARY KEY,
    node_id        TEXT NOT NULL REFERENCES nodes (node_id) ON DELETE CASCADE,
    participant_id TEXT NOT NULL REFERENCES participants (participant_id) ON DELETE CASCADE,
    public_key     TEXT NOT NULL,
    manifest       JSONB NOT NULL,
    manifest_hash  TEXT NOT NULL,
    backend        TEXT,
    status         TEXT NOT NULL DEFAULT 'offline'
                   CHECK (status IN ('online', 'offline', 'draining')),
    last_heartbeat TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_agents_node ON agents (node_id);
CREATE INDEX IF NOT EXISTS idx_agents_status ON agents (status);

CREATE TABLE IF NOT EXISTS agent_capabilities (
    agent_id             TEXT PRIMARY KEY REFERENCES agents (agent_id) ON DELETE CASCADE,
    skills               JSONB NOT NULL,
    tools                JSONB NOT NULL,
    models               JSONB NOT NULL,
    max_concurrent_tasks INTEGER NOT NULL,
    max_depth_supported  INTEGER NOT NULL,
    accepts_delegation   BOOLEAN NOT NULL,
    min_budget_credits   BIGINT NOT NULL,
    max_task_seconds     BIGINT NOT NULL,
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS agent_messages (
    id          BIGSERIAL PRIMARY KEY,
    from_agent  TEXT NOT NULL,
    to_agent    TEXT,
    kind        TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    nonce       BIGINT NOT NULL,
    signature   TEXT,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_agent_messages_from ON agent_messages (from_agent, received_at);
