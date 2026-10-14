-- Phase 0 schema: participants and worker nodes.

CREATE TABLE IF NOT EXISTS participants (
    participant_id TEXT PRIMARY KEY,
    public_key     TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'active'
                   CHECK (status IN ('active', 'suspended', 'closed')),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS nodes (
    node_id        TEXT PRIMARY KEY,
    participant_id TEXT NOT NULL REFERENCES participants (participant_id) ON DELETE CASCADE,
    public_key     TEXT NOT NULL,
    capabilities   JSONB NOT NULL,
    status         TEXT NOT NULL DEFAULT 'offline'
                   CHECK (status IN ('online', 'offline', 'draining', 'decommissioned')),
    last_heartbeat TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_nodes_participant_id ON nodes (participant_id);
CREATE INDEX IF NOT EXISTS idx_nodes_status ON nodes (status);
