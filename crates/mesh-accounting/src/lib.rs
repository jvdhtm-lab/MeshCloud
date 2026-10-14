//! Contribution accounting: verification records, receipts, and credit.
//!
//! Rewards are idempotent. A receipt is unique per attempt, so replaying the
//! same completion never issues a second reward or double-credits a balance.

#![forbid(unsafe_code)]

use sqlx::{PgPool, Row};

use mesh_protocol::ids::{AgentId, AttemptId, JobId, NodeId, ParticipantId, ReceiptId};
use mesh_protocol::{
    AgentRewardView, ContributionView, CreditBalanceView, DisputeView, ReservationView,
};

use chrono::{Duration, Utc};

/// Errors raised by the accounting layer.
#[derive(Debug, thiserror::Error)]
pub enum AccountingError {
    /// A database error occurred.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// A versioned reward policy. Amounts are integer credits, never floating point.
#[derive(Debug, Clone, Copy)]
pub struct RewardPolicy {
    /// Policy version recorded on every receipt.
    pub version: i32,
    /// Credits awarded per accepted usage unit to the executor.
    pub rate_per_unit: i64,
    /// Credits awarded per accepted usage unit to a delegating agent.
    pub delegator_rate_per_unit: i64,
}

impl RewardPolicy {
    /// Compute the executor reward for a number of accepted units.
    pub fn reward(&self, accepted_units: i64) -> i64 {
        accepted_units.max(0).saturating_mul(self.rate_per_unit)
    }

    /// Compute the reward for a given role (`delegator` uses the coordination rate).
    pub fn reward_for_role(&self, accepted_units: i64, role: &str) -> i64 {
        let rate = if role == "delegator" {
            self.delegator_rate_per_unit
        } else {
            self.rate_per_unit
        };
        accepted_units.max(0).saturating_mul(rate)
    }
}

/// The data needed to issue a contribution receipt.
#[derive(Debug, Clone)]
pub struct ContributionInput {
    /// Job that produced the contribution, if any.
    pub job_id: Option<JobId>,
    /// Root of the delegation tree, if any.
    pub root_job_id: Option<JobId>,
    /// Attempt that produced the contribution.
    pub attempt_id: AttemptId,
    /// Participant to credit.
    pub participant_id: ParticipantId,
    /// Node to credit.
    pub node_id: NodeId,
    /// Agent to credit, if agent-attributed.
    pub agent_id: Option<AgentId>,
    /// Role the agent played (`worker`, `delegatee`, `delegator`).
    pub role: String,
    /// Workload type.
    pub workload_type: String,
    /// Accepted usage units.
    pub accepted_units: i64,
    /// Verification method used.
    pub verification_method: String,
    /// Hash of the verified result.
    pub evidence_hash: String,
    /// Reward policy version.
    pub policy_version: i32,
}

/// A newly issued receipt.
#[derive(Debug, Clone)]
pub struct RecordedContribution {
    /// Receipt identifier.
    pub receipt_id: ReceiptId,
    /// Reward amount credited.
    pub reward_amount: i64,
}

/// Record the outcome of a verification attempt for audit.
pub async fn record_verification(
    pool: &PgPool,
    job_id: &JobId,
    attempt_id: &AttemptId,
    method: &str,
    outcome: &str,
    evidence_hash: Option<&str>,
    detail: &str,
) -> Result<(), AccountingError> {
    sqlx::query(
        "INSERT INTO verification_records \
             (job_id, attempt_id, method, outcome, evidence_hash, detail) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(job_id.as_str())
    .bind(attempt_id.as_str())
    .bind(method)
    .bind(outcome)
    .bind(evidence_hash)
    .bind(detail)
    .execute(pool)
    .await?;
    Ok(())
}

/// Issue a contribution receipt and credit the participant's balance.
///
/// Returns `Some` when a new receipt is issued, or `None` when the attempt has
/// already been recorded (a replay), in which case no credit is added.
pub async fn record_contribution(
    pool: &PgPool,
    policy: &RewardPolicy,
    input: &ContributionInput,
) -> Result<Option<RecordedContribution>, AccountingError> {
    let reward_amount = policy.reward_for_role(input.accepted_units, &input.role);
    let receipt_id = ReceiptId::generate();
    let mut tx = pool.begin().await?;

    let inserted = sqlx::query(
        "INSERT INTO contribution_receipts \
             (receipt_id, subject_type, job_id, root_job_id, attempt_id, participant_id, node_id, \
              agent_id, role, workload_type, accepted_units, verification_method, evidence_hash, \
              policy_version, reward_amount, status) \
         VALUES ($1, 'job_attempt', $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, 'issued') \
         ON CONFLICT (attempt_id, role) DO NOTHING",
    )
    .bind(receipt_id.as_str())
    .bind(input.job_id.as_ref().map(JobId::as_str))
    .bind(input.root_job_id.as_ref().map(JobId::as_str))
    .bind(input.attempt_id.as_str())
    .bind(input.participant_id.as_str())
    .bind(input.node_id.as_str())
    .bind(input.agent_id.as_ref().map(AgentId::as_str))
    .bind(&input.role)
    .bind(&input.workload_type)
    .bind(input.accepted_units)
    .bind(&input.verification_method)
    .bind(&input.evidence_hash)
    .bind(input.policy_version)
    .bind(reward_amount)
    .execute(&mut *tx)
    .await?;

    if inserted.rows_affected() == 0 {
        tx.commit().await?;
        return Ok(None);
    }

    sqlx::query(
        "INSERT INTO credit_balances (participant_id, earned, spent, reserved, available) \
         VALUES ($1, $2, 0, 0, $2) \
         ON CONFLICT (participant_id) DO UPDATE SET \
             earned = credit_balances.earned + EXCLUDED.earned, \
             available = credit_balances.available + EXCLUDED.available, \
             updated_at = now()",
    )
    .bind(input.participant_id.as_str())
    .bind(reward_amount)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Some(RecordedContribution {
        receipt_id,
        reward_amount,
    }))
}

/// List recent contribution receipts, newest first, optionally filtered by
/// agent and/or participant.
pub async fn list_contributions(
    pool: &PgPool,
    limit: i64,
    agent: Option<&AgentId>,
    participant: Option<&ParticipantId>,
) -> Result<Vec<ContributionView>, AccountingError> {
    let rows = sqlx::query(
        "SELECT receipt_id, subject_type, job_id, root_job_id, attempt_id, participant_id, \
             node_id, agent_id, role, workload_type, accepted_units, verification_method, \
             evidence_hash, policy_version, reward_amount, status, created_at \
         FROM contribution_receipts \
         WHERE ($2::text IS NULL OR agent_id = $2) \
           AND ($3::text IS NULL OR participant_id = $3) \
         ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .bind(agent.map(AgentId::as_str))
    .bind(participant.map(ParticipantId::as_str))
    .fetch_all(pool)
    .await?;

    let mut contributions = Vec::with_capacity(rows.len());
    for row in rows {
        contributions.push(ContributionView {
            receipt_id: ReceiptId::new(row.try_get::<String, _>("receipt_id")?),
            subject_type: row.try_get("subject_type")?,
            job_id: row.try_get::<Option<String>, _>("job_id")?.map(JobId::new),
            root_job_id: row
                .try_get::<Option<String>, _>("root_job_id")?
                .map(JobId::new),
            attempt_id: AttemptId::new(row.try_get::<String, _>("attempt_id")?),
            participant_id: ParticipantId::new(row.try_get::<String, _>("participant_id")?),
            node_id: NodeId::new(row.try_get::<String, _>("node_id")?),
            agent_id: row
                .try_get::<Option<String>, _>("agent_id")?
                .map(AgentId::new),
            role: row.try_get("role")?,
            workload_type: row.try_get("workload_type")?,
            accepted_units: row.try_get("accepted_units")?,
            verification_method: row.try_get("verification_method")?,
            evidence_hash: row.try_get("evidence_hash")?,
            policy_version: row.try_get("policy_version")?,
            reward_amount: row.try_get("reward_amount")?,
            status: row.try_get("status")?,
            created_at: row.try_get("created_at")?,
        });
    }
    Ok(contributions)
}

/// Aggregate rewards credited to an agent.
pub async fn agent_rewards(
    pool: &PgPool,
    agent_id: &AgentId,
) -> Result<AgentRewardView, AccountingError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS receipt_count, \
             COALESCE(SUM(reward_amount), 0)::bigint AS total_reward \
         FROM contribution_receipts WHERE agent_id = $1",
    )
    .bind(agent_id.as_str())
    .fetch_one(pool)
    .await?;

    Ok(AgentRewardView {
        agent_id: agent_id.clone(),
        receipt_count: row.try_get("receipt_count")?,
        total_reward: row.try_get("total_reward")?,
    })
}

/// Dispute a receipt: void it and reverse its credit. Returns the dispute, or
/// `None` if the receipt is already void.
pub async fn dispute_contribution(
    pool: &PgPool,
    receipt_id: &ReceiptId,
    reason: &str,
) -> Result<Option<DisputeView>, AccountingError> {
    let mut tx = pool.begin().await?;

    let row = sqlx::query(
        "SELECT participant_id, reward_amount, status FROM contribution_receipts \
         WHERE receipt_id = $1 FOR UPDATE",
    )
    .bind(receipt_id.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.rollback().await?;
        return Ok(None);
    };
    let status: String = row.try_get("status")?;
    if status == "void" {
        tx.commit().await?;
        return Ok(None);
    }
    let participant_id: String = row.try_get("participant_id")?;
    let reward: i64 = row.try_get("reward_amount")?;

    sqlx::query("UPDATE contribution_receipts SET status = 'void' WHERE receipt_id = $1")
        .bind(receipt_id.as_str())
        .execute(&mut *tx)
        .await?;

    // Reverse the credit that was granted when the receipt was issued.
    sqlx::query(
        "UPDATE credit_balances \
         SET earned = GREATEST(earned - $2, 0), available = GREATEST(available - $2, 0), \
             updated_at = now() \
         WHERE participant_id = $1",
    )
    .bind(&participant_id)
    .bind(reward)
    .execute(&mut *tx)
    .await?;

    let dispute_id = format!("dsp-{}", uuid::Uuid::new_v4());
    let created_at = Utc::now();
    sqlx::query(
        "INSERT INTO contribution_disputes (dispute_id, receipt_id, reason, status, created_at) \
         VALUES ($1, $2, $3, 'open', $4)",
    )
    .bind(&dispute_id)
    .bind(receipt_id.as_str())
    .bind(reason)
    .bind(created_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Some(DisputeView {
        dispute_id,
        receipt_id: receipt_id.clone(),
        reason: reason.to_string(),
        status: "open".to_string(),
        created_at,
    }))
}

/// Total credits currently reserved by active reservations for a participant.
pub async fn outstanding_reserved(
    pool: &PgPool,
    participant_id: &ParticipantId,
) -> Result<i64, AccountingError> {
    let total: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(credits), 0)::bigint FROM reservations \
         WHERE participant_id = $1 AND status = 'active'",
    )
    .bind(participant_id.as_str())
    .fetch_one(pool)
    .await?;
    Ok(total)
}

/// Reserve credits for a participant, moving them from available to reserved.
///
/// Returns `None` when the participant has insufficient available credits.
pub async fn create_reservation(
    pool: &PgPool,
    participant_id: &ParticipantId,
    workload_type: &str,
    credits: i64,
    ttl_seconds: u64,
) -> Result<Option<ReservationView>, AccountingError> {
    if credits <= 0 {
        return Ok(None);
    }
    let mut tx = pool.begin().await?;

    let available: Option<i64> = sqlx::query_scalar(
        "SELECT available FROM credit_balances WHERE participant_id = $1 FOR UPDATE",
    )
    .bind(participant_id.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    if available.unwrap_or(0) < credits {
        tx.rollback().await?;
        return Ok(None);
    }

    sqlx::query(
        "UPDATE credit_balances \
         SET available = available - $2, reserved = reserved + $2, updated_at = now() \
         WHERE participant_id = $1",
    )
    .bind(participant_id.as_str())
    .bind(credits)
    .execute(&mut *tx)
    .await?;

    let reservation_id = format!("res-{}", uuid::Uuid::new_v4());
    let created_at = Utc::now();
    let expires_at = created_at + Duration::seconds(ttl_seconds as i64);

    sqlx::query(
        "INSERT INTO reservations \
             (reservation_id, participant_id, workload_type, credits, status, created_at, expires_at) \
         VALUES ($1, $2, $3, $4, 'active', $5, $6)",
    )
    .bind(&reservation_id)
    .bind(participant_id.as_str())
    .bind(workload_type)
    .bind(credits)
    .bind(created_at)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Some(ReservationView {
        reservation_id,
        participant_id: participant_id.clone(),
        workload_type: workload_type.to_string(),
        credits,
        status: "active".to_string(),
        created_at,
        expires_at,
    }))
}

/// List a participant's reservations, newest first.
pub async fn list_reservations(
    pool: &PgPool,
    participant_id: &ParticipantId,
) -> Result<Vec<ReservationView>, AccountingError> {
    let rows = sqlx::query(
        "SELECT reservation_id, participant_id, workload_type, credits, status, created_at, expires_at \
         FROM reservations WHERE participant_id = $1 ORDER BY created_at DESC",
    )
    .bind(participant_id.as_str())
    .fetch_all(pool)
    .await?;

    let mut reservations = Vec::with_capacity(rows.len());
    for row in rows {
        reservations.push(ReservationView {
            reservation_id: row.try_get("reservation_id")?,
            participant_id: ParticipantId::new(row.try_get::<String, _>("participant_id")?),
            workload_type: row.try_get("workload_type")?,
            credits: row.try_get("credits")?,
            status: row.try_get("status")?,
            created_at: row.try_get("created_at")?,
            expires_at: row.try_get("expires_at")?,
        });
    }
    Ok(reservations)
}

/// Expire reservations whose lifetime has passed, returning credits to available.
pub async fn expire_reservations(pool: &PgPool) -> Result<u64, AccountingError> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "SELECT reservation_id, participant_id, credits FROM reservations \
         WHERE status = 'active' AND expires_at <= now() FOR UPDATE SKIP LOCKED",
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut count = 0u64;
    for row in rows {
        let reservation_id: String = row.try_get("reservation_id")?;
        let participant_id: String = row.try_get("participant_id")?;
        let credits: i64 = row.try_get("credits")?;

        sqlx::query(
            "UPDATE credit_balances \
             SET available = available + $2, reserved = GREATEST(reserved - $2, 0), updated_at = now() \
             WHERE participant_id = $1",
        )
        .bind(&participant_id)
        .bind(credits)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE reservations SET status = 'expired' WHERE reservation_id = $1")
            .bind(&reservation_id)
            .execute(&mut *tx)
            .await?;
        count += 1;
    }

    tx.commit().await?;
    Ok(count)
}

/// Fetch a participant's credit balance, if any.
pub async fn participant_balance(
    pool: &PgPool,
    participant_id: &ParticipantId,
) -> Result<Option<CreditBalanceView>, AccountingError> {
    let row = sqlx::query(
        "SELECT participant_id, earned, spent, reserved, available, updated_at \
         FROM credit_balances WHERE participant_id = $1",
    )
    .bind(participant_id.as_str())
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => Ok(Some(CreditBalanceView {
            participant_id: ParticipantId::new(row.try_get::<String, _>("participant_id")?),
            earned: row.try_get("earned")?,
            spent: row.try_get("spent")?,
            reserved: row.try_get("reserved")?,
            available: row.try_get("available")?,
            updated_at: row.try_get("updated_at")?,
        })),
        None => Ok(None),
    }
}
