-- Phase 2: jobs and job attempts (leases, retries, results).

CREATE TABLE IF NOT EXISTS jobs (
    job_id             TEXT PRIMARY KEY,
    kind               TEXT NOT NULL DEFAULT 'job',
    root_job_id        TEXT NOT NULL,
    parent_job_id      TEXT REFERENCES jobs (job_id) ON DELETE CASCADE,
    depth              INTEGER NOT NULL DEFAULT 0,
    requester_id       TEXT NOT NULL,
    workload_type      TEXT NOT NULL,
    input              JSONB NOT NULL,
    status             TEXT NOT NULL DEFAULT 'pending'
                       CHECK (status IN ('pending', 'scheduled', 'running', 'succeeded',
                                         'retry_wait', 'failed', 'cancelled', 'expired')),
    required_cpu_cores INTEGER NOT NULL DEFAULT 0,
    required_ram_gb    INTEGER NOT NULL DEFAULT 0,
    max_attempts       INTEGER NOT NULL DEFAULT 3,
    attempts           INTEGER NOT NULL DEFAULT 0,
    worker_id          TEXT REFERENCES nodes (node_id) ON DELETE SET NULL,
    result_hash        TEXT,
    deadline           TIMESTAMPTZ NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_jobs_status_created ON jobs (status, created_at);
CREATE INDEX IF NOT EXISTS idx_jobs_root_job_id ON jobs (root_job_id);
CREATE INDEX IF NOT EXISTS idx_jobs_worker_id ON jobs (worker_id);

CREATE TABLE IF NOT EXISTS job_attempts (
    attempt_id       TEXT PRIMARY KEY,
    job_id           TEXT NOT NULL REFERENCES jobs (job_id) ON DELETE CASCADE,
    worker_id        TEXT NOT NULL,
    attempt_number   INTEGER NOT NULL,
    status           TEXT NOT NULL DEFAULT 'running'
                     CHECK (status IN ('running', 'succeeded', 'failed', 'expired')),
    result_hash      TEXT,
    log              TEXT,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    started_at       TIMESTAMPTZ,
    finished_at      TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_job_attempts_job ON job_attempts (job_id, attempt_number);
CREATE INDEX IF NOT EXISTS idx_job_attempts_lease ON job_attempts (status, lease_expires_at);

-- A job's attempts share the root job; the root references itself.
ALTER TABLE jobs
    ADD CONSTRAINT jobs_root_job_id_fkey
    FOREIGN KEY (root_job_id) REFERENCES jobs (job_id) ON DELETE CASCADE;
