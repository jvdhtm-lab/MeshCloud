-- Phase 12: abuse reporting and contribution disputes.

CREATE TABLE IF NOT EXISTS abuse_reports (
    report_id            TEXT PRIMARY KEY,
    reporter_participant TEXT,
    subject_type         TEXT NOT NULL CHECK (subject_type IN ('node', 'agent')),
    subject_id           TEXT NOT NULL,
    category             TEXT NOT NULL,
    detail               TEXT,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_abuse_subject ON abuse_reports (subject_type, subject_id);

CREATE TABLE IF NOT EXISTS contribution_disputes (
    dispute_id TEXT PRIMARY KEY,
    receipt_id TEXT NOT NULL REFERENCES contribution_receipts (receipt_id) ON DELETE CASCADE,
    reason     TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'open'
               CHECK (status IN ('open', 'upheld', 'rejected')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_disputes_receipt ON contribution_disputes (receipt_id);
