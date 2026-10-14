//! The MeshCloud scheduler: placement, atomic claims, leases, and retries.
//!
//! This crate owns the job lifecycle. It is the single scheduler — agent tasks
//! are jobs with children and are handled here too, in a later phase. Claiming
//! uses `SELECT ... FOR UPDATE SKIP LOCKED` so two workers can never claim the
//! same attempt.

#![forbid(unsafe_code)]

use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use sqlx::types::Json as SqlJson;
use sqlx::{PgPool, Row};

use mesh_protocol::ids::{AgentId, AttemptId, JobId, NodeId, ParticipantId};
use mesh_protocol::status::{IllegalTransition, JobState};
use mesh_protocol::{
    AttemptView, CancelJobResponse, ClaimedJobResponse, CompleteJobResponse, DelegateJobRequest,
    DelegateJobResponse, FailJobResponse, JobDetailResponse, JobTreeResponse, JobView,
    SubmitJobResponse, WaitJobResponse,
};

/// Errors raised by the scheduler.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    /// A referenced job does not exist.
    #[error("job not found: {0}")]
    NotFound(JobId),
    /// The attempt is not active: wrong job, wrong worker, or already finished.
    #[error("attempt is not active for this job and worker")]
    AttemptNotActive,
    /// The database reported an unknown job state.
    #[error("invalid job state in database: {0}")]
    InvalidState(String),
    /// A lifecycle transition was rejected by the state machine.
    #[error(transparent)]
    IllegalTransition(#[from] mesh_protocol::status::IllegalTransition),
    /// A delegation violated the DAG bounds.
    #[error(transparent)]
    Delegation(#[from] DelegationError),
    /// A database error occurred.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Errors raised when a delegation violates DAG bounds.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DelegationError {
    /// A job cannot be delegated to its own agent.
    #[error("a job cannot be delegated to itself")]
    SelfDelegation,
    /// The delegatee is already an ancestor in the tree.
    #[error("delegation would create a cycle to an ancestor agent")]
    AncestorCycle,
    /// The child would exceed the root's maximum depth.
    #[error("delegation exceeds max depth ({max_depth})")]
    DepthExceeded {
        /// The root's maximum depth.
        max_depth: i32,
    },
    /// The parent already has the maximum number of children.
    #[error("delegation exceeds max fanout ({max_fanout})")]
    FanoutExceeded {
        /// The parent's maximum fanout.
        max_fanout: i32,
    },
    /// The child's budget exceeds the parent's remaining budget.
    #[error("delegation exceeds the parent's remaining budget")]
    BudgetExceeded,
    /// The child's deadline is later than the parent's.
    #[error("child deadline is later than the parent deadline")]
    DeadlineWidened,
}

/// Inputs to the pure delegation-bounds check.
#[derive(Debug, Clone)]
pub struct DelegationBounds {
    /// Depth of the delegating (parent) job.
    pub parent_depth: i32,
    /// Maximum depth allowed by the root job.
    pub root_max_depth: i32,
    /// Number of children the parent already has.
    pub current_fanout: i32,
    /// Maximum children the parent may have.
    pub parent_max_fanout: i32,
    /// The parent's total budget.
    pub parent_budget: i64,
    /// Budget already reserved by the parent's children.
    pub parent_reserved: i64,
    /// Budget requested for the child.
    pub child_budget: i64,
    /// The parent's deadline.
    pub parent_deadline: DateTime<Utc>,
    /// The requested child deadline.
    pub child_deadline: DateTime<Utc>,
    /// The agent delegating (owner of the parent), if any.
    pub delegator_agent: Option<AgentId>,
    /// The agent receiving the child.
    pub delegatee_agent: AgentId,
    /// Agents that already own ancestors of the parent (including the parent).
    pub ancestor_agents: Vec<AgentId>,
}

/// Check every delegation invariant. Pure so it can be exhaustively tested.
pub fn check_bounds(bounds: &DelegationBounds) -> Result<(), DelegationError> {
    if bounds.delegator_agent.as_ref() == Some(&bounds.delegatee_agent) {
        return Err(DelegationError::SelfDelegation);
    }
    if bounds
        .ancestor_agents
        .iter()
        .any(|agent| agent == &bounds.delegatee_agent)
    {
        return Err(DelegationError::AncestorCycle);
    }
    if bounds.parent_depth + 1 > bounds.root_max_depth {
        return Err(DelegationError::DepthExceeded {
            max_depth: bounds.root_max_depth,
        });
    }
    if bounds.current_fanout + 1 > bounds.parent_max_fanout {
        return Err(DelegationError::FanoutExceeded {
            max_fanout: bounds.parent_max_fanout,
        });
    }
    let remaining = bounds.parent_budget - bounds.parent_reserved;
    if bounds.child_budget < 0 || bounds.child_budget > remaining {
        return Err(DelegationError::BudgetExceeded);
    }
    if bounds.child_deadline > bounds.parent_deadline {
        return Err(DelegationError::DeadlineWidened);
    }
    Ok(())
}

/// A job to be submitted.
#[derive(Debug, Clone)]
pub struct NewJob {
    /// Workload type the job requires.
    pub workload_type: String,
    /// Opaque job input.
    pub input: serde_json::Value,
    /// Job kind (`job` or `agent_task`).
    pub kind: String,
    /// Agent to assign the job to, if any.
    pub assignee_agent_id: Option<AgentId>,
    /// Minimum CPU cores required.
    pub required_cpu_cores: u32,
    /// Minimum RAM (GB) required.
    pub required_ram_gb: u32,
    /// Maximum attempts before the job fails.
    pub max_attempts: u32,
    /// Seconds from now until the job expires.
    pub deadline_seconds: u64,
    /// Maximum delegation depth for this root job.
    pub max_depth: u32,
    /// Maximum direct children this job may delegate.
    pub max_fanout: u32,
    /// Budget (credits) available to delegate to children.
    pub budget_credits: i64,
}

impl Default for NewJob {
    fn default() -> Self {
        Self {
            workload_type: String::new(),
            input: serde_json::Value::Null,
            kind: "job".to_string(),
            assignee_agent_id: None,
            required_cpu_cores: 0,
            required_ram_gb: 0,
            max_attempts: 3,
            deadline_seconds: 3600,
            max_depth: 0,
            max_fanout: 0,
            budget_credits: 0,
        }
    }
}

/// The claiming worker's capabilities, used for matching.
#[derive(Debug, Clone)]
pub struct WorkerContext {
    /// The worker's node id.
    pub worker_id: NodeId,
    /// CPU cores the worker offers.
    pub cpu_cores: u32,
    /// RAM (GB) the worker offers.
    pub ram_gb: u32,
    /// Workload types the worker can execute.
    pub supported_workloads: Vec<String>,
}

/// Which jobs a claim may match.
#[derive(Debug, Clone)]
pub struct ClaimFilter {
    /// Required job kind (`job` or `agent_task`).
    pub kind: String,
    /// Required assignee agent, if any.
    pub assignee_agent_id: Option<AgentId>,
}

impl ClaimFilter {
    /// A filter for ordinary worker jobs.
    pub fn worker_jobs() -> Self {
        Self {
            kind: "job".to_string(),
            assignee_agent_id: None,
        }
    }

    /// A filter for agent tasks assigned to a specific agent.
    pub fn agent_tasks(agent_id: AgentId) -> Self {
        Self {
            kind: "agent_task".to_string(),
            assignee_agent_id: Some(agent_id),
        }
    }
}

/// Everything needed to verify and account a job that a worker completed.
#[derive(Debug, Clone)]
pub struct CompletionContext {
    /// Workload type of the job.
    pub workload_type: String,
    /// Job input, replayed by the verifier.
    pub input: serde_json::Value,
    /// Current job state at read time.
    pub state: JobState,
    /// Job kind (`job` or `agent_task`).
    pub kind: String,
    /// Worker that ran the job, if assigned.
    pub worker_id: Option<NodeId>,
    /// Agent assigned to the job, if any.
    pub assignee_agent_id: Option<AgentId>,
    /// Parent job in the delegation DAG, if any.
    pub parent_job_id: Option<JobId>,
    /// Root of the delegation tree.
    pub root_job_id: Option<JobId>,
    /// Participant that owns the worker, if known.
    pub participant_id: Option<ParticipantId>,
}

/// Fetch the context required to verify and account a completion.
pub async fn completion_context(
    pool: &PgPool,
    job_id: &JobId,
) -> Result<Option<CompletionContext>, SchedulerError> {
    let row = sqlx::query(
        "SELECT j.workload_type, j.input, j.status, j.kind, j.worker_id, j.assignee_agent_id, \
             j.parent_job_id, j.root_job_id, n.participant_id \
         FROM jobs j LEFT JOIN nodes n ON n.node_id = j.worker_id \
         WHERE j.job_id = $1",
    )
    .bind(job_id.as_str())
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    Ok(Some(CompletionContext {
        workload_type: row.try_get("workload_type")?,
        input: row.try_get::<SqlJson<serde_json::Value>, _>("input")?.0,
        state: parse_state(&row.try_get::<String, _>("status")?)?,
        kind: row.try_get("kind")?,
        worker_id: row
            .try_get::<Option<String>, _>("worker_id")?
            .map(NodeId::new),
        assignee_agent_id: row
            .try_get::<Option<String>, _>("assignee_agent_id")?
            .map(AgentId::new),
        parent_job_id: row
            .try_get::<Option<String>, _>("parent_job_id")?
            .map(JobId::new),
        root_job_id: row
            .try_get::<Option<String>, _>("root_job_id")?
            .map(JobId::new),
        participant_id: row
            .try_get::<Option<String>, _>("participant_id")?
            .map(ParticipantId::new),
    }))
}

fn parse_state(raw: &str) -> Result<JobState, SchedulerError> {
    raw.parse::<JobState>()
        .map_err(|_| SchedulerError::InvalidState(raw.to_string()))
}

/// Submit a new root job. Returns its summary.
pub async fn submit_job(
    pool: &PgPool,
    requester_id: &str,
    job: NewJob,
) -> Result<SubmitJobResponse, SchedulerError> {
    let job_id = JobId::generate();
    let deadline = Utc::now() + Duration::seconds(job.deadline_seconds as i64);
    let max_attempts = job.max_attempts.max(1) as i32;

    sqlx::query(
        "INSERT INTO jobs (job_id, kind, root_job_id, parent_job_id, depth, requester_id, \
             workload_type, input, status, required_cpu_cores, required_ram_gb, \
             max_attempts, attempts, deadline, max_depth, max_fanout, budget_credits, \
             assignee_agent_id) \
         VALUES ($1, $2, $1, NULL, 0, $3, $4, $5, 'pending', $6, $7, $8, 0, $9, $10, $11, $12, $13)",
    )
    .bind(job_id.as_str())
    .bind(&job.kind)
    .bind(requester_id)
    .bind(&job.workload_type)
    .bind(SqlJson(&job.input))
    .bind(job.required_cpu_cores as i32)
    .bind(job.required_ram_gb as i32)
    .bind(max_attempts)
    .bind(deadline)
    .bind(job.max_depth as i32)
    .bind(job.max_fanout as i32)
    .bind(job.budget_credits)
    .bind(job.assignee_agent_id.as_ref().map(AgentId::as_str))
    .execute(pool)
    .await?;

    Ok(SubmitJobResponse {
        job_id,
        state: JobState::Pending,
        deadline,
    })
}

/// Atomically claim the next eligible job for a worker, if any.
pub async fn claim_job(
    pool: &PgPool,
    worker: &WorkerContext,
    lease_seconds: i64,
    filter: &ClaimFilter,
) -> Result<Option<ClaimedJobResponse>, SchedulerError> {
    let mut tx = pool.begin().await?;

    let row = sqlx::query(
        "WITH candidate AS ( \
             SELECT job_id FROM jobs \
             WHERE status = 'pending' \
               AND deadline > now() \
               AND attempts < max_attempts \
               AND required_cpu_cores <= $1 \
               AND required_ram_gb <= $2 \
               AND workload_type = ANY($3) \
               AND kind = $5 \
               AND ($6::text IS NULL OR assignee_agent_id = $6) \
               AND EXISTS ( \
                   SELECT 1 FROM resource_offers o \
                   WHERE o.node_id = $4 AND o.revoked_at IS NULL AND o.expires_at > now()) \
             ORDER BY created_at \
             FOR UPDATE SKIP LOCKED \
             LIMIT 1 \
         ) \
         UPDATE jobs \
         SET status = 'scheduled', worker_id = $4, attempts = attempts + 1, updated_at = now() \
         FROM candidate \
         WHERE jobs.job_id = candidate.job_id \
         RETURNING jobs.job_id, jobs.kind, jobs.workload_type, jobs.input, jobs.attempts, \
                   jobs.deadline, jobs.depth",
    )
    .bind(worker.cpu_cores as i32)
    .bind(worker.ram_gb as i32)
    .bind(&worker.supported_workloads)
    .bind(worker.worker_id.as_str())
    .bind(&filter.kind)
    .bind(filter.assignee_agent_id.as_ref().map(AgentId::as_str))
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = row else {
        tx.commit().await?;
        return Ok(None);
    };

    let job_id = JobId::new(row.try_get::<String, _>("job_id")?);
    let kind: String = row.try_get("kind")?;
    let workload_type: String = row.try_get("workload_type")?;
    let input: SqlJson<serde_json::Value> = row.try_get("input")?;
    let attempts: i32 = row.try_get("attempts")?;
    let deadline: DateTime<Utc> = row.try_get("deadline")?;
    let depth: i32 = row.try_get("depth")?;

    let attempt_id = AttemptId::generate();
    let lease_expires_at = Utc::now() + Duration::seconds(lease_seconds);

    sqlx::query(
        "INSERT INTO job_attempts \
             (attempt_id, job_id, worker_id, attempt_number, status, lease_expires_at, started_at) \
         VALUES ($1, $2, $3, $4, 'running', $5, now())",
    )
    .bind(attempt_id.as_str())
    .bind(job_id.as_str())
    .bind(worker.worker_id.as_str())
    .bind(attempts)
    .bind(lease_expires_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Some(ClaimedJobResponse {
        job_id,
        kind,
        attempt_id,
        workload_type,
        input: input.0,
        attempt_number: attempts as u32,
        lease_expires_at,
        deadline,
        depth: depth as u32,
    }))
}

/// Mark a claimed attempt as succeeded. Idempotent: completing an already
/// succeeded job is a no-op and never double-counts.
pub async fn complete_job(
    pool: &PgPool,
    job_id: &JobId,
    attempt_id: &AttemptId,
    result_hash: &str,
    log: Option<&str>,
) -> Result<CompleteJobResponse, SchedulerError> {
    let mut tx = pool.begin().await?;

    let current = lock_job_state(&mut tx, job_id).await?;
    if current == JobState::Succeeded || current.is_terminal() {
        tx.commit().await?;
        return Ok(CompleteJobResponse {
            job_id: job_id.clone(),
            state: current,
        });
    }

    let finished = reach_succeeded(current)?;

    let affected = sqlx::query(
        "UPDATE job_attempts \
         SET status = 'succeeded', result_hash = $2, log = COALESCE($3, log), finished_at = now() \
         WHERE attempt_id = $1 AND job_id = $4 AND status = 'running'",
    )
    .bind(attempt_id.as_str())
    .bind(result_hash)
    .bind(log)
    .bind(job_id.as_str())
    .execute(&mut *tx)
    .await?;

    if affected.rows_affected() == 0 {
        return Err(SchedulerError::AttemptNotActive);
    }

    sqlx::query(
        "UPDATE jobs SET status = 'succeeded', result_hash = $2, updated_at = now() WHERE job_id = $1",
    )
    .bind(job_id.as_str())
    .bind(result_hash)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // A completed child may complete its parent.
    let parent: Option<String> =
        sqlx::query_scalar("SELECT parent_job_id FROM jobs WHERE job_id = $1")
            .bind(job_id.as_str())
            .fetch_one(pool)
            .await?;
    if let Some(parent) = parent {
        maybe_complete_parent(pool, &JobId::new(parent)).await?;
    }

    Ok(CompleteJobResponse {
        job_id: job_id.clone(),
        state: finished,
    })
}

/// Mark a claimed attempt as failed. Retries if attempts remain.
pub async fn fail_job(
    pool: &PgPool,
    job_id: &JobId,
    attempt_id: &AttemptId,
    error: &str,
) -> Result<FailJobResponse, SchedulerError> {
    let mut tx = pool.begin().await?;

    let (current, attempts, max_attempts) = lock_job(&mut tx, job_id).await?;
    if current.is_terminal() {
        tx.commit().await?;
        return Ok(FailJobResponse {
            job_id: job_id.clone(),
            state: current,
            will_retry: false,
        });
    }

    let affected = sqlx::query(
        "UPDATE job_attempts \
         SET status = 'failed', log = COALESCE($2, log), finished_at = now() \
         WHERE attempt_id = $1 AND job_id = $3 AND status = 'running'",
    )
    .bind(attempt_id.as_str())
    .bind(error)
    .bind(job_id.as_str())
    .execute(&mut *tx)
    .await?;

    if affected.rows_affected() == 0 {
        return Err(SchedulerError::AttemptNotActive);
    }

    // scheduled -> retry_wait -> pending (retry) or -> failed (exhausted).
    let retry_wait = current.transition(JobState::RetryWait)?;
    let will_retry = attempts < max_attempts;
    let next = if will_retry {
        retry_wait.transition(JobState::Pending)?
    } else {
        retry_wait.transition(JobState::Failed)?
    };

    if will_retry {
        sqlx::query(
            "UPDATE jobs SET status = 'pending', worker_id = NULL, updated_at = now() WHERE job_id = $1",
        )
        .bind(job_id.as_str())
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            "UPDATE jobs SET status = 'failed', worker_id = NULL, updated_at = now() WHERE job_id = $1",
        )
        .bind(job_id.as_str())
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    if next == JobState::Failed {
        let parent: Option<String> =
            sqlx::query_scalar("SELECT parent_job_id FROM jobs WHERE job_id = $1")
                .bind(job_id.as_str())
                .fetch_one(pool)
                .await?;
        if let Some(parent) = parent {
            maybe_complete_parent(pool, &JobId::new(parent)).await?;
        }
    }

    Ok(FailJobResponse {
        job_id: job_id.clone(),
        state: next,
        will_retry,
    })
}

/// Delegate a child job from a parent, enforcing all DAG bounds.
pub async fn delegate_job(
    pool: &PgPool,
    parent_id: &JobId,
    spec: &DelegateJobRequest,
) -> Result<DelegateJobResponse, SchedulerError> {
    let mut tx = pool.begin().await?;

    let parent = sqlx::query(
        "SELECT root_job_id, depth, max_fanout, deadline, budget_credits, reserved_credits, \
             status, assignee_agent_id, requester_id \
         FROM jobs WHERE job_id = $1 FOR UPDATE",
    )
    .bind(parent_id.as_str())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| SchedulerError::NotFound(parent_id.clone()))?;

    let parent_state = parse_state(&parent.try_get::<String, _>("status")?)?;
    if parent_state.is_terminal() {
        return Err(SchedulerError::IllegalTransition(IllegalTransition {
            from: parent_state,
            to: JobState::Pending,
        }));
    }

    let root_job_id: String = parent.try_get("root_job_id")?;
    let parent_depth: i32 = parent.try_get("depth")?;
    let parent_max_fanout: i32 = parent.try_get("max_fanout")?;
    let parent_deadline: DateTime<Utc> = parent.try_get("deadline")?;
    let parent_budget: i64 = parent.try_get("budget_credits")?;
    let parent_reserved: i64 = parent.try_get("reserved_credits")?;
    let delegator_agent: Option<String> = parent.try_get("assignee_agent_id")?;
    let requester_id: String = parent.try_get("requester_id")?;

    let root_max_depth: i32 = sqlx::query_scalar("SELECT max_depth FROM jobs WHERE job_id = $1")
        .bind(&root_job_id)
        .fetch_one(&mut *tx)
        .await?;
    let current_fanout: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE parent_job_id = $1 AND status <> 'cancelled'",
    )
    .bind(parent_id.as_str())
    .fetch_one(&mut *tx)
    .await?;
    let ancestor_agents: Vec<String> = sqlx::query_scalar(
        "WITH RECURSIVE anc AS ( \
             SELECT job_id, parent_job_id, assignee_agent_id FROM jobs WHERE job_id = $1 \
             UNION ALL \
             SELECT j.job_id, j.parent_job_id, j.assignee_agent_id \
             FROM jobs j JOIN anc ON j.job_id = anc.parent_job_id \
         ) SELECT assignee_agent_id FROM anc WHERE assignee_agent_id IS NOT NULL",
    )
    .bind(parent_id.as_str())
    .fetch_all(&mut *tx)
    .await?;

    let child_deadline = Utc::now() + Duration::seconds(spec.deadline_seconds as i64);
    let bounds = DelegationBounds {
        parent_depth,
        root_max_depth,
        current_fanout: current_fanout as i32,
        parent_max_fanout,
        parent_budget,
        parent_reserved,
        child_budget: spec.budget_credits,
        parent_deadline,
        child_deadline,
        delegator_agent: delegator_agent.map(AgentId::new),
        delegatee_agent: spec.delegatee_agent_id.clone(),
        ancestor_agents: ancestor_agents.into_iter().map(AgentId::new).collect(),
    };
    check_bounds(&bounds)?;

    let child_id = JobId::generate();
    sqlx::query(
        "INSERT INTO jobs (job_id, kind, root_job_id, parent_job_id, depth, requester_id, \
             workload_type, input, status, required_cpu_cores, required_ram_gb, max_attempts, \
             attempts, deadline, max_depth, max_fanout, budget_credits, reserved_credits, \
             assignee_agent_id) \
         VALUES ($1, 'agent_task', $2, $3, $4, $5, $6, $7, 'pending', 0, 0, 2, 0, $8, 0, 0, 0, 0, $9)",
    )
    .bind(child_id.as_str())
    .bind(&root_job_id)
    .bind(parent_id.as_str())
    .bind(parent_depth + 1)
    .bind(&requester_id)
    .bind(&spec.workload_type)
    .bind(SqlJson(&spec.input))
    .bind(child_deadline)
    .bind(spec.delegatee_agent_id.as_str())
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE jobs SET reserved_credits = reserved_credits + $2, updated_at = now() \
         WHERE job_id = $1",
    )
    .bind(parent_id.as_str())
    .bind(spec.budget_credits)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(DelegateJobResponse {
        parent_job_id: parent_id.clone(),
        child_job_id: child_id,
        state: JobState::Pending,
    })
}

/// Move a running job into `waiting_on_children`.
pub async fn mark_waiting_on_children(
    pool: &PgPool,
    job_id: &JobId,
) -> Result<WaitJobResponse, SchedulerError> {
    let mut tx = pool.begin().await?;
    let current = lock_job_state(&mut tx, job_id).await?;
    let next = match current {
        JobState::WaitingOnChildren => current,
        JobState::Running => current.transition(JobState::WaitingOnChildren)?,
        JobState::Scheduled => current
            .transition(JobState::Running)?
            .transition(JobState::WaitingOnChildren)?,
        other => {
            return Err(SchedulerError::IllegalTransition(IllegalTransition {
                from: other,
                to: JobState::WaitingOnChildren,
            }))
        }
    };
    if next != current {
        sqlx::query(
            "UPDATE jobs SET status = 'waiting_on_children', updated_at = now() WHERE job_id = $1",
        )
        .bind(job_id.as_str())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(WaitJobResponse {
        job_id: job_id.clone(),
        state: next,
    })
}

/// Cancel a job and all of its live descendants.
pub async fn cancel_job(
    pool: &PgPool,
    job_id: &JobId,
) -> Result<CancelJobResponse, SchedulerError> {
    let affected = sqlx::query(
        "WITH RECURSIVE tree AS ( \
             SELECT job_id FROM jobs WHERE job_id = $1 \
             UNION ALL \
             SELECT j.job_id FROM jobs j JOIN tree t ON j.parent_job_id = t.job_id \
         ) \
         UPDATE jobs SET status = 'cancelled', updated_at = now() \
         WHERE job_id IN (SELECT job_id FROM tree) \
           AND status NOT IN ('succeeded', 'failed', 'cancelled', 'expired')",
    )
    .bind(job_id.as_str())
    .execute(pool)
    .await?
    .rows_affected();

    Ok(CancelJobResponse {
        job_id: job_id.clone(),
        cancelled: affected,
    })
}

/// Return the delegation tree rooted at the given job's root.
pub async fn job_tree(
    pool: &PgPool,
    job_id: &JobId,
) -> Result<Option<JobTreeResponse>, SchedulerError> {
    let root: Option<String> = sqlx::query_scalar("SELECT root_job_id FROM jobs WHERE job_id = $1")
        .bind(job_id.as_str())
        .fetch_optional(pool)
        .await?;
    let Some(root) = root else {
        return Ok(None);
    };

    let rows = sqlx::query(
        "SELECT job_id, kind, workload_type, status, worker_id, assignee_agent_id, \
             parent_job_id, depth, budget_credits, attempts, max_attempts, \
             deadline, created_at, result_hash \
         FROM jobs WHERE root_job_id = $1 ORDER BY depth, created_at",
    )
    .bind(&root)
    .fetch_all(pool)
    .await?;

    let jobs = rows
        .into_iter()
        .map(job_view_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(JobTreeResponse {
        root_job_id: JobId::new(root),
        jobs,
    }))
}

/// Complete or fail a parent once all of its children are terminal.
pub async fn maybe_complete_parent(pool: &PgPool, parent_id: &JobId) -> Result<(), SchedulerError> {
    let mut tx = pool.begin().await?;

    let state = match lock_job_state(&mut tx, parent_id).await {
        Ok(state) => state,
        Err(SchedulerError::NotFound(_)) => {
            tx.rollback().await?;
            return Ok(());
        }
        Err(err) => return Err(err),
    };
    if state != JobState::WaitingOnChildren {
        tx.commit().await?;
        return Ok(());
    }

    let counts = sqlx::query(
        "SELECT COUNT(*) AS total, \
             COUNT(*) FILTER (WHERE status = 'succeeded') AS ok, \
             COUNT(*) FILTER (WHERE status IN ('failed', 'expired', 'cancelled')) AS bad \
         FROM jobs WHERE parent_job_id = $1",
    )
    .bind(parent_id.as_str())
    .fetch_one(&mut *tx)
    .await?;
    let total: i64 = counts.try_get("total")?;
    let ok: i64 = counts.try_get("ok")?;
    let bad: i64 = counts.try_get("bad")?;

    if bad > 0 {
        let _ = state.transition(JobState::Failed)?;
        sqlx::query("UPDATE jobs SET status = 'failed', updated_at = now() WHERE job_id = $1")
            .bind(parent_id.as_str())
            .execute(&mut *tx)
            .await?;
    } else if total > 0 && ok == total {
        let hashes: Vec<Option<String>> = sqlx::query_scalar(
            "SELECT result_hash FROM jobs WHERE parent_job_id = $1 ORDER BY created_at",
        )
        .bind(parent_id.as_str())
        .fetch_all(&mut *tx)
        .await?;
        let aggregate = aggregate_hashes(hashes);
        let _ = state.transition(JobState::Succeeded)?;
        sqlx::query(
            "UPDATE jobs SET status = 'succeeded', result_hash = $2, updated_at = now() WHERE job_id = $1",
        )
        .bind(parent_id.as_str())
        .bind(&aggregate)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

fn aggregate_hashes(hashes: Vec<Option<String>>) -> String {
    let mut values: Vec<String> = hashes.into_iter().flatten().collect();
    values.sort();
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update(value.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// Reclaim jobs whose worker vanished by expiring stale leases and requeueing
/// them (or failing them when attempts are exhausted). Returns jobs affected.
pub async fn requeue_expired_leases(pool: &PgPool) -> Result<u64, SchedulerError> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "UPDATE job_attempts SET status = 'expired', finished_at = now() \
         WHERE status = 'running' AND lease_expires_at < now()",
    )
    .execute(&mut *tx)
    .await?;

    let requeued = sqlx::query(
        "UPDATE jobs SET \
             status = CASE WHEN attempts < max_attempts THEN 'pending' ELSE 'failed' END, \
             worker_id = NULL, updated_at = now() \
         WHERE status IN ('scheduled', 'running') \
           AND EXISTS ( \
               SELECT 1 FROM job_attempts a \
               WHERE a.job_id = jobs.job_id AND a.status = 'expired')",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();

    tx.commit().await?;
    Ok(requeued)
}

/// Expire jobs whose deadline has passed. Returns jobs affected.
pub async fn expire_overdue_jobs(pool: &PgPool) -> Result<u64, SchedulerError> {
    let affected = sqlx::query(
        "UPDATE jobs SET status = 'expired', updated_at = now() \
         WHERE status IN ('pending', 'scheduled', 'running', 'retry_wait') \
           AND deadline <= now()",
    )
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected)
}

/// List recent jobs, newest first.
pub async fn list_jobs(pool: &PgPool, limit: i64) -> Result<Vec<JobView>, SchedulerError> {
    let rows = sqlx::query(
        "SELECT job_id, kind, workload_type, status, worker_id, assignee_agent_id, \
             parent_job_id, depth, budget_credits, attempts, max_attempts, \
             deadline, created_at, result_hash \
         FROM jobs ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(job_view_from_row).collect()
}

/// Fetch one job with its attempts.
pub async fn job_detail(
    pool: &PgPool,
    job_id: &JobId,
) -> Result<Option<JobDetailResponse>, SchedulerError> {
    let row = sqlx::query(
        "SELECT job_id, kind, workload_type, status, worker_id, assignee_agent_id, \
             parent_job_id, depth, budget_credits, attempts, max_attempts, \
             deadline, created_at, result_hash \
         FROM jobs WHERE job_id = $1",
    )
    .bind(job_id.as_str())
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };
    let job = job_view_from_row(row)?;

    let attempt_rows = sqlx::query(
        "SELECT attempt_id, attempt_number, worker_id, status, result_hash, log, \
             started_at, finished_at \
         FROM job_attempts WHERE job_id = $1 ORDER BY attempt_number",
    )
    .bind(job_id.as_str())
    .fetch_all(pool)
    .await?;

    let mut attempts = Vec::with_capacity(attempt_rows.len());
    for row in attempt_rows {
        attempts.push(AttemptView {
            attempt_id: AttemptId::new(row.try_get::<String, _>("attempt_id")?),
            attempt_number: row.try_get::<i32, _>("attempt_number")? as u32,
            worker_id: NodeId::new(row.try_get::<String, _>("worker_id")?),
            state: parse_state(&row.try_get::<String, _>("status")?)?,
            result_hash: row.try_get("result_hash")?,
            log: row.try_get("log")?,
            started_at: row.try_get("started_at")?,
            finished_at: row.try_get("finished_at")?,
        });
    }

    Ok(Some(JobDetailResponse { job, attempts }))
}

fn job_view_from_row(row: sqlx::postgres::PgRow) -> Result<JobView, SchedulerError> {
    Ok(JobView {
        job_id: JobId::new(row.try_get::<String, _>("job_id")?),
        kind: row.try_get("kind")?,
        workload_type: row.try_get("workload_type")?,
        state: parse_state(&row.try_get::<String, _>("status")?)?,
        worker_id: row
            .try_get::<Option<String>, _>("worker_id")?
            .map(NodeId::new),
        assignee_agent_id: row
            .try_get::<Option<String>, _>("assignee_agent_id")?
            .map(AgentId::new),
        parent_job_id: row
            .try_get::<Option<String>, _>("parent_job_id")?
            .map(JobId::new),
        depth: row.try_get::<i32, _>("depth")? as u32,
        budget_credits: row.try_get("budget_credits")?,
        attempts: row.try_get::<i32, _>("attempts")? as u32,
        max_attempts: row.try_get::<i32, _>("max_attempts")? as u32,
        deadline: row.try_get("deadline")?,
        created_at: row.try_get("created_at")?,
        result_hash: row.try_get("result_hash")?,
    })
}

async fn lock_job_state(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job_id: &JobId,
) -> Result<JobState, SchedulerError> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM jobs WHERE job_id = $1 FOR UPDATE")
            .bind(job_id.as_str())
            .fetch_optional(&mut **tx)
            .await?;
    let status = status.ok_or_else(|| SchedulerError::NotFound(job_id.clone()))?;
    parse_state(&status)
}

async fn lock_job(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job_id: &JobId,
) -> Result<(JobState, i32, i32), SchedulerError> {
    let row =
        sqlx::query("SELECT status, attempts, max_attempts FROM jobs WHERE job_id = $1 FOR UPDATE")
            .bind(job_id.as_str())
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| SchedulerError::NotFound(job_id.clone()))?;
    let status = parse_state(&row.try_get::<String, _>("status")?)?;
    let attempts: i32 = row.try_get("attempts")?;
    let max_attempts: i32 = row.try_get("max_attempts")?;
    Ok((status, attempts, max_attempts))
}

/// Move a schedulable job to `succeeded`, honoring the explicit state machine.
fn reach_succeeded(current: JobState) -> Result<JobState, SchedulerError> {
    let running = match current {
        JobState::Scheduled => current.transition(JobState::Running)?,
        JobState::Running => current,
        other => {
            return Err(SchedulerError::IllegalTransition(
                mesh_protocol::status::IllegalTransition {
                    from: other,
                    to: JobState::Succeeded,
                },
            ))
        }
    };
    Ok(running.transition(JobState::Succeeded)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_bounds() -> DelegationBounds {
        DelegationBounds {
            parent_depth: 0,
            root_max_depth: 3,
            current_fanout: 0,
            parent_max_fanout: 4,
            parent_budget: 10,
            parent_reserved: 0,
            child_budget: 1,
            parent_deadline: Utc::now() + Duration::seconds(3600),
            child_deadline: Utc::now() + Duration::seconds(600),
            delegator_agent: Some(AgentId::new("agent-a")),
            delegatee_agent: AgentId::new("agent-b"),
            ancestor_agents: vec![AgentId::new("agent-a")],
        }
    }

    #[test]
    fn valid_delegation_passes() {
        assert!(check_bounds(&base_bounds()).is_ok());
    }

    #[test]
    fn self_delegation_rejected() {
        let mut b = base_bounds();
        b.delegatee_agent = AgentId::new("agent-a");
        assert_eq!(check_bounds(&b), Err(DelegationError::SelfDelegation));
    }

    #[test]
    fn ancestor_cycle_rejected() {
        let mut b = base_bounds();
        b.delegatee_agent = AgentId::new("agent-root");
        b.ancestor_agents = vec![AgentId::new("agent-a"), AgentId::new("agent-root")];
        assert_eq!(check_bounds(&b), Err(DelegationError::AncestorCycle));
    }

    #[test]
    fn depth_limit_enforced() {
        let mut b = base_bounds();
        b.parent_depth = 3;
        b.root_max_depth = 3;
        assert_eq!(
            check_bounds(&b),
            Err(DelegationError::DepthExceeded { max_depth: 3 })
        );
    }

    #[test]
    fn fanout_limit_enforced() {
        let mut b = base_bounds();
        b.current_fanout = 4;
        b.parent_max_fanout = 4;
        assert_eq!(
            check_bounds(&b),
            Err(DelegationError::FanoutExceeded { max_fanout: 4 })
        );
    }

    #[test]
    fn budget_limit_enforced() {
        let mut b = base_bounds();
        b.parent_budget = 5;
        b.parent_reserved = 4;
        b.child_budget = 2;
        assert_eq!(check_bounds(&b), Err(DelegationError::BudgetExceeded));
    }

    #[test]
    fn widened_deadline_rejected() {
        let mut b = base_bounds();
        b.child_deadline = b.parent_deadline + Duration::seconds(1);
        assert_eq!(check_bounds(&b), Err(DelegationError::DeadlineWidened));
    }

    #[test]
    fn aggregate_hash_is_order_independent() {
        let a = aggregate_hashes(vec![Some("bbb".to_string()), Some("aaa".to_string())]);
        let b = aggregate_hashes(vec![Some("aaa".to_string()), Some("bbb".to_string())]);
        assert_eq!(a, b);
    }
}
