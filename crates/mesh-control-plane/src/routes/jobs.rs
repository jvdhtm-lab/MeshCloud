//! Job submission, inspection, completion, and failure.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use sqlx::Row;

use mesh_protocol::{
    AgentId, AttemptId, CancelJobResponse, CompleteJobRequest, CompleteJobResponse,
    DelegateJobRequest, DelegateJobResponse, FailJobRequest, FailJobResponse, JobDetailResponse,
    JobId, JobTreeResponse, JobView, NodeId, ParticipantId, SubmitJobRequest, SubmitJobResponse,
    WaitJobResponse,
};
use mesh_scheduler::NewJob;

use crate::error::ApiError;
use crate::AppState;

/// Job lifecycle routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/jobs", get(list_jobs).post(submit_job))
        .route("/api/v1/jobs/:job_id", get(job_status))
        .route("/api/v1/jobs/:job_id/complete", post(complete_job))
        .route("/api/v1/jobs/:job_id/fail", post(fail_job))
        .route("/api/v1/jobs/:job_id/delegate", post(delegate_job))
        .route("/api/v1/jobs/:job_id/tree", get(job_tree))
        .route("/api/v1/jobs/:job_id/cancel", post(cancel_job))
        .route("/api/v1/jobs/:job_id/wait", post(wait_job))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    limit: Option<i64>,
}

/// Submit a new job.
pub async fn submit_job(
    State(state): State<AppState>,
    Json(req): Json<SubmitJobRequest>,
) -> Result<(StatusCode, Json<SubmitJobResponse>), ApiError> {
    if req.workload_type.trim().is_empty() {
        return Err(ApiError::bad_request("workload_type must not be empty"));
    }

    let new = NewJob {
        workload_type: req.workload_type,
        input: req.input,
        kind: req.kind,
        assignee_agent_id: req.assignee_agent_id,
        required_cpu_cores: req.required_cpu_cores,
        required_ram_gb: req.required_ram_gb,
        max_attempts: req.max_attempts,
        deadline_seconds: req.deadline_seconds,
        max_depth: req.max_depth,
        max_fanout: req.max_fanout,
        budget_credits: req.budget_credits,
    };

    let response = mesh_scheduler::submit_job(&state.pool, "local", new).await?;
    tracing::info!(job_id = %response.job_id, "job submitted");
    Ok((StatusCode::CREATED, Json(response)))
}

/// List recent jobs.
pub async fn list_jobs(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<JobView>>, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let jobs = mesh_scheduler::list_jobs(&state.pool, limit).await?;
    Ok(Json(jobs))
}

/// Fetch a job with its attempts.
pub async fn job_status(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<JobDetailResponse>, ApiError> {
    let job_id = JobId::new(job_id);
    mesh_scheduler::job_detail(&state.pool, &job_id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("job not found"))
}

/// Complete a job with a result hash.
///
/// The claimed result is verified by independent re-execution before the job is
/// marked succeeded. A rejected result fails the attempt and earns no reward.
pub async fn complete_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    Json(req): Json<CompleteJobRequest>,
) -> Result<Json<CompleteJobResponse>, ApiError> {
    let job_id = JobId::new(job_id);

    let ctx = mesh_scheduler::completion_context(&state.pool, &job_id)
        .await?
        .ok_or_else(|| ApiError::not_found("job not found"))?;

    if ctx.state == mesh_protocol::JobState::Succeeded {
        return Ok(Json(CompleteJobResponse {
            job_id,
            state: ctx.state,
        }));
    }
    if ctx.state.is_terminal() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            format!("job is {}", ctx.state),
        ));
    }

    let verify_config = mesh_verifier::VerifyConfig {
        max_memory_mb: state.config.verification_memory_mb,
        fuel: state.config.verification_fuel,
        timeout: std::time::Duration::from_secs(state.config.verification_timeout_seconds.max(1)),
    };
    let verification = mesh_verifier::verify(
        &ctx.workload_type,
        &ctx.input,
        &req.result_hash,
        &verify_config,
    );

    let outcome = if verification.passed {
        "accepted"
    } else {
        "rejected"
    };
    mesh_accounting::record_verification(
        &state.pool,
        &job_id,
        &req.attempt_id,
        &verification.method,
        outcome,
        Some(verification.evidence_hash.as_str()),
        &verification.detail,
    )
    .await?;

    if !verification.passed {
        let failed = mesh_scheduler::fail_job(
            &state.pool,
            &job_id,
            &req.attempt_id,
            &format!("verification failed: {}", verification.detail),
        )
        .await?;
        tracing::warn!(job_id = %job_id, state = %failed.state, "verification rejected result");
        return Ok(Json(CompleteJobResponse {
            job_id,
            state: failed.state,
        }));
    }

    let completed = mesh_scheduler::complete_job(
        &state.pool,
        &job_id,
        &req.attempt_id,
        &req.result_hash,
        req.log.as_deref(),
    )
    .await?;

    if completed.state == mesh_protocol::JobState::Succeeded {
        let policy = mesh_accounting::RewardPolicy {
            version: state.config.reward_policy_version,
            rate_per_unit: state.config.reward_rate_per_unit,
            delegator_rate_per_unit: state.config.delegator_reward_rate_per_unit,
        };

        if ctx.kind == "agent_task" {
            // Credit the delegatee agent, and the delegator agent if this is a child.
            if let Some(delegatee) = ctx.assignee_agent_id.clone() {
                if let Some((participant_id, node_id)) =
                    agent_participant_node(&state.pool, &delegatee).await?
                {
                    record_receipt(
                        &state,
                        &policy,
                        &ctx,
                        &job_id,
                        &req.attempt_id,
                        participant_id,
                        node_id,
                        Some(delegatee.clone()),
                        "delegatee",
                        &verification,
                    )
                    .await;
                }
                if let Some(parent_id) = ctx.parent_job_id.clone() {
                    if let Some(delegator) = delegator_agent(&state.pool, &parent_id).await? {
                        if delegator != delegatee {
                            if let Some((participant_id, node_id)) =
                                agent_participant_node(&state.pool, &delegator).await?
                            {
                                record_receipt(
                                    &state,
                                    &policy,
                                    &ctx,
                                    &job_id,
                                    &req.attempt_id,
                                    participant_id,
                                    node_id,
                                    Some(delegator),
                                    "delegator",
                                    &verification,
                                )
                                .await;
                            }
                        }
                    }
                }
            }
        } else if let (Some(node_id), Some(participant_id)) =
            (ctx.worker_id.clone(), ctx.participant_id.clone())
        {
            record_receipt(
                &state,
                &policy,
                &ctx,
                &job_id,
                &req.attempt_id,
                participant_id,
                node_id,
                None,
                "worker",
                &verification,
            )
            .await;
        }
    }

    tracing::info!(job_id = %job_id, state = %completed.state, "job completed");
    Ok(Json(completed))
}

/// Issue one contribution receipt for a role, logging the outcome.
#[allow(clippy::too_many_arguments)]
async fn record_receipt(
    state: &AppState,
    policy: &mesh_accounting::RewardPolicy,
    ctx: &mesh_scheduler::CompletionContext,
    job_id: &JobId,
    attempt_id: &AttemptId,
    participant_id: ParticipantId,
    node_id: NodeId,
    agent_id: Option<AgentId>,
    role: &str,
    verification: &mesh_verifier::Verification,
) {
    let contribution = mesh_accounting::ContributionInput {
        job_id: Some(job_id.clone()),
        root_job_id: ctx.root_job_id.clone(),
        attempt_id: attempt_id.clone(),
        participant_id,
        node_id,
        agent_id,
        role: role.to_string(),
        workload_type: ctx.workload_type.clone(),
        accepted_units: verification.accepted_units,
        verification_method: verification.method.clone(),
        evidence_hash: verification.evidence_hash.clone(),
        policy_version: policy.version,
    };
    match mesh_accounting::record_contribution(&state.pool, policy, &contribution).await {
        Ok(Some(recorded)) => tracing::info!(
            job_id = %job_id,
            role,
            receipt = %recorded.receipt_id,
            reward = recorded.reward_amount,
            "contribution recorded"
        ),
        Ok(None) => tracing::info!(job_id = %job_id, role, "duplicate contribution ignored"),
        Err(err) => {
            tracing::error!(job_id = %job_id, role, error = %err, "failed to record contribution")
        }
    }
}

/// Look up an agent's participant and node.
async fn agent_participant_node(
    pool: &sqlx::PgPool,
    agent_id: &AgentId,
) -> Result<Option<(ParticipantId, NodeId)>, ApiError> {
    let row = sqlx::query("SELECT participant_id, node_id FROM agents WHERE agent_id = $1")
        .bind(agent_id.as_str())
        .fetch_optional(pool)
        .await?;
    match row {
        Some(row) => Ok(Some((
            ParticipantId::new(row.try_get::<String, _>("participant_id")?),
            NodeId::new(row.try_get::<String, _>("node_id")?),
        ))),
        None => Ok(None),
    }
}

/// The agent assigned to a parent job, if any.
async fn delegator_agent(
    pool: &sqlx::PgPool,
    parent_id: &JobId,
) -> Result<Option<AgentId>, ApiError> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT assignee_agent_id FROM jobs WHERE job_id = $1")
            .bind(parent_id.as_str())
            .fetch_one(pool)
            .await?;
    Ok(value.map(AgentId::new))
}

/// Report a job failure.
pub async fn fail_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    Json(req): Json<FailJobRequest>,
) -> Result<Json<FailJobResponse>, ApiError> {
    let job_id = JobId::new(job_id);
    let response =
        mesh_scheduler::fail_job(&state.pool, &job_id, &req.attempt_id, &req.error).await?;
    tracing::warn!(
        job_id = %job_id,
        state = %response.state,
        will_retry = response.will_retry,
        "job failed"
    );
    Ok(Json(response))
}

/// Delegate a child job from a parent, enforcing DAG bounds and message limits.
pub async fn delegate_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    Json(req): Json<DelegateJobRequest>,
) -> Result<(StatusCode, Json<DelegateJobResponse>), ApiError> {
    let parent = JobId::new(job_id);

    // Meter delegation attempts per delegating agent.
    let delegator: Option<String> =
        sqlx::query_scalar("SELECT assignee_agent_id FROM jobs WHERE job_id = $1")
            .bind(parent.as_str())
            .fetch_optional(&state.pool)
            .await?;
    if let Some(agent) = delegator {
        let bytes = serde_json::to_vec(&req)
            .map(|v| v.len() as u64)
            .unwrap_or(0);
        if !state.meter.allow(&agent, bytes) {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "agent message rate limit exceeded",
            ));
        }
    }

    let response = mesh_scheduler::delegate_job(&state.pool, &parent, &req).await?;
    tracing::info!(
        parent = %parent,
        child = %response.child_job_id,
        delegatee = %req.delegatee_agent_id,
        "job delegated"
    );
    Ok((StatusCode::CREATED, Json(response)))
}

/// Return the delegation tree for a job.
pub async fn job_tree(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<JobTreeResponse>, ApiError> {
    mesh_scheduler::job_tree(&state.pool, &JobId::new(job_id))
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("job not found"))
}

/// Cancel a job and all of its live descendants.
pub async fn cancel_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<CancelJobResponse>, ApiError> {
    let response = mesh_scheduler::cancel_job(&state.pool, &JobId::new(job_id)).await?;
    tracing::info!(job_id = %response.job_id, cancelled = response.cancelled, "job cancelled");
    Ok(Json(response))
}

/// Mark a job as waiting on its delegated children.
pub async fn wait_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<WaitJobResponse>, ApiError> {
    let response =
        mesh_scheduler::mark_waiting_on_children(&state.pool, &JobId::new(job_id)).await?;
    Ok(Json(response))
}
