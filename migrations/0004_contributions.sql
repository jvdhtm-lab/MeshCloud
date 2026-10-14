-- Phase 4: verification records, contribution receipts, and credit balances.

CREATE TABLE IF NOT EXISTS verification_records (
    id            BIGSERIAL PRIMARY KEY,
    job_id        TEXT NOT NULL,
    attempt_id    TEXT NOT NULL,
    method        TEXT NOT NULL,
    outcome       TEXT NOT NULL
                  CHECK (outcome IN ('accepted', 'rejected', 'error')),
    evidence_hash TEXT,
    detail        TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_verification_records_attempt
    ON verification_records (attempt_id);
CREATE INDEX IF NOT EXISTS idx_verification_records_job
    ON verification_records (job_id);

CREATE TABLE IF NOT EXISTS contribution_receipts (
    receipt_id          TEXT PRIMARY KEY,
    subject_type        TEXT NOT NULL DEFAULT 'job_attempt',
    job_id              TEXT NOT NULL REFERENCES jobs (job_id) ON DELETE CASCADE,
    attempt_id          TEXT NOT NULL,
    participant_id      TEXT NOT NULL,
    node_id             TEXT NOT NULL,
    workload_type       TEXT NOT NULL,
    accepted_units      BIGINT NOT NULL,
    verification_method TEXT NOT NULL,
    evidence_hash       TEXT NOT NULL,
    policy_version      INTEGER NOT NULL,
    reward_amount       BIGINT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'issued'
                        CHECK (status IN ('issued', 'settled', 'void')),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- One receipt per attempt: replays cannot create duplicate credit.
    CONSTRAINT contribution_receipts_attempt_unique UNIQUE (attempt_id)
);

CREATE INDEX IF NOT EXISTS idx_contribution_receipts_participant
    ON contribution_receipts (participant_id);
CREATE INDEX IF NOT EXISTS idx_contribution_receipts_job
    ON contribution_receipts (job_id);

CREATE TABLE IF NOT EXISTS credit_balances (
    participant_id TEXT PRIMARY KEY REFERENCES participants (participant_id) ON DELETE CASCADE,
    earned         BIGINT NOT NULL DEFAULT 0,
    spent          BIGINT NOT NULL DEFAULT 0,
    reserved       BIGINT NOT NULL DEFAULT 0,
    available      BIGINT NOT NULL DEFAULT 0,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
