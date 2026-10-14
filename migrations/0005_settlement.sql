-- Phase 5: reward batches and settlement.

ALTER TABLE participants ADD COLUMN IF NOT EXISTS wallet_address TEXT;

-- Receipts gain an intermediate state so a claimed-but-not-confirmed batch can
-- be safely requeued.
ALTER TABLE contribution_receipts DROP CONSTRAINT IF EXISTS contribution_receipts_status_check;
ALTER TABLE contribution_receipts
    ADD CONSTRAINT contribution_receipts_status_check
    CHECK (status IN ('issued', 'batched', 'settled', 'void'));

CREATE TABLE IF NOT EXISTS reward_batches (
    batch_id      TEXT PRIMARY KEY,
    merkle_root   TEXT NOT NULL,
    total_reward  BIGINT NOT NULL,
    receipt_count INTEGER NOT NULL,
    chain_id      BIGINT NOT NULL DEFAULT 0,
    status        TEXT NOT NULL DEFAULT 'submitted'
                  CHECK (status IN ('submitted', 'settled', 'failed')),
    tx_hash       TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    settled_at    TIMESTAMPTZ
);

-- A receipt may belong to at most one batch: the primary key on receipt_id
-- makes double-settlement impossible.
CREATE TABLE IF NOT EXISTS batch_receipts (
    batch_id   TEXT NOT NULL REFERENCES reward_batches (batch_id) ON DELETE CASCADE,
    receipt_id TEXT NOT NULL REFERENCES contribution_receipts (receipt_id) ON DELETE CASCADE,
    PRIMARY KEY (receipt_id)
);

CREATE INDEX IF NOT EXISTS idx_batch_receipts_batch ON batch_receipts (batch_id);
CREATE INDEX IF NOT EXISTS idx_reward_batches_status ON reward_batches (status, created_at);

CREATE TABLE IF NOT EXISTS settlements (
    settlement_id TEXT PRIMARY KEY,
    batch_id      TEXT NOT NULL REFERENCES reward_batches (batch_id) ON DELETE CASCADE,
    chain_id      BIGINT NOT NULL DEFAULT 0,
    tx_hash       TEXT,
    status        TEXT NOT NULL DEFAULT 'settled'
                  CHECK (status IN ('submitted', 'settled', 'failed')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_settlements_batch ON settlements (batch_id);
