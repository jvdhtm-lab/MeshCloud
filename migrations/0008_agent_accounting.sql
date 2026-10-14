-- Phase 8: agent-attributed receipts, delegator/delegatee roles, agent rewards.

ALTER TABLE contribution_receipts ADD COLUMN IF NOT EXISTS agent_id TEXT;
ALTER TABLE contribution_receipts ADD COLUMN IF NOT EXISTS role TEXT NOT NULL DEFAULT 'worker';
ALTER TABLE contribution_receipts ADD COLUMN IF NOT EXISTS root_job_id TEXT;

-- One receipt per (attempt, role): a subtask can credit both its delegatee and
-- its delegator, but never twice for the same role.
ALTER TABLE contribution_receipts
    DROP CONSTRAINT IF EXISTS contribution_receipts_attempt_unique;
ALTER TABLE contribution_receipts
    ADD CONSTRAINT contribution_receipts_attempt_role_unique UNIQUE (attempt_id, role);

CREATE INDEX IF NOT EXISTS idx_contribution_receipts_agent ON contribution_receipts (agent_id);
