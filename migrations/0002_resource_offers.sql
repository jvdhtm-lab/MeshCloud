-- Phase 1: latest signed resource offer per node.

CREATE TABLE IF NOT EXISTS resource_offers (
    node_id    TEXT PRIMARY KEY REFERENCES nodes (node_id) ON DELETE CASCADE,
    offer      JSONB NOT NULL,
    public_key TEXT NOT NULL,
    signature  TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_resource_offers_expires_at ON resource_offers (expires_at);
