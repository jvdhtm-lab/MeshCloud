-- Phase 11: distributed storage metadata.

-- Storage receipts are not tied to a job.
ALTER TABLE contribution_receipts ALTER COLUMN job_id DROP NOT NULL;

CREATE TABLE IF NOT EXISTS storage_objects (
    object_id         TEXT PRIMARY KEY,
    owner_participant TEXT NOT NULL,
    size_bytes        BIGINT NOT NULL,
    encrypted_bytes   BIGINT NOT NULL,
    data_shards       INTEGER NOT NULL,
    parity_shards     INTEGER NOT NULL,
    nonce             TEXT NOT NULL,
    status            TEXT NOT NULL DEFAULT 'healthy'
                      CHECK (status IN ('healthy', 'degraded', 'lost')),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS storage_shards (
    shard_id     TEXT PRIMARY KEY,
    object_id    TEXT NOT NULL REFERENCES storage_objects (object_id) ON DELETE CASCADE,
    shard_index  INTEGER NOT NULL,
    node_id      TEXT NOT NULL,
    checksum     TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'present'
                 CHECK (status IN ('present', 'missing')),
    last_verified TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (object_id, shard_index)
);

CREATE INDEX IF NOT EXISTS idx_storage_shards_object ON storage_shards (object_id);
CREATE INDEX IF NOT EXISTS idx_storage_shards_node ON storage_shards (node_id);

CREATE TABLE IF NOT EXISTS storage_challenges (
    challenge_id TEXT PRIMARY KEY,
    shard_id     TEXT NOT NULL REFERENCES storage_shards (shard_id) ON DELETE CASCADE,
    nonce        BIGINT NOT NULL,
    outcome      TEXT NOT NULL CHECK (outcome IN ('verified', 'failed')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_storage_challenges_shard ON storage_challenges (shard_id);
