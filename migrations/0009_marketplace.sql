-- Phase 9: offer revocation and resource reservations.

ALTER TABLE resource_offers ADD COLUMN IF NOT EXISTS revoked_at TIMESTAMPTZ;

CREATE TABLE IF NOT EXISTS reservations (
    reservation_id TEXT PRIMARY KEY,
    participant_id TEXT NOT NULL REFERENCES participants (participant_id) ON DELETE CASCADE,
    workload_type  TEXT NOT NULL,
    credits        BIGINT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'active'
                   CHECK (status IN ('active', 'consumed', 'expired', 'cancelled')),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at     TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_reservations_participant
    ON reservations (participant_id, status);
CREATE INDEX IF NOT EXISTS idx_reservations_expiry
    ON reservations (status, expires_at);
