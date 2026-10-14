-- Phase 7: delegation DAG columns and the waiting_on_children state.

ALTER TABLE jobs ADD COLUMN IF NOT EXISTS max_depth INTEGER NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS max_fanout INTEGER NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS budget_credits BIGINT NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS reserved_credits BIGINT NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS assignee_agent_id TEXT;

ALTER TABLE jobs DROP CONSTRAINT IF EXISTS jobs_status_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_status_check CHECK (status IN (
    'pending', 'scheduled', 'running', 'waiting_on_children',
    'succeeded', 'retry_wait', 'failed', 'cancelled', 'expired'
));

CREATE INDEX IF NOT EXISTS idx_jobs_assignee_status ON jobs (assignee_agent_id, status);
