//! Agent registry endpoints: registration, heartbeat, listing, inspection.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::types::Json as SqlJson;
use sqlx::Row;

use mesh_agent_protocol::{
    manifest_hash, AgentCapabilities, AgentHeartbeatRequest, AgentHeartbeatResponse, AgentPolicy,
    AgentStatus, AgentView, RegisterAgentRequest, RegisterAgentResponse, SignedAgentManifest,
};
use mesh_identity::NodeIdentity;
use mesh_protocol::ids::{AgentId, NodeId, ParticipantId};
use mesh_protocol::{AgentRewardView, NodeCapabilities};
use mesh_scheduler::WorkerContext;

use crate::error::ApiError;
use crate::AppState;

/// Agent registry routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/agents", get(list_agents))
        .route("/api/v1/agents/register", post(register_agent))
        .route("/api/v1/agents/:agent_id", get(inspect_agent))
        .route("/api/v1/agents/:agent_id/heartbeat", post(heartbeat))
        .route("/api/v1/agents/:agent_id/jobs/next", get(next_agent_job))
        .route("/api/v1/agents/:agent_id/rewards", get(agent_rewards))
        .route("/api/v1/agents/:agent_id/manifest", get(agent_manifest))
        .route(
            "/api/v1/agents/:agent_id/delegations",
            get(agent_delegations),
        )
}

/// Fetch the stored capability manifest for an agent.
pub async fn agent_manifest(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let manifest: Option<SqlJson<serde_json::Value>> =
        sqlx::query_scalar("SELECT manifest FROM agents WHERE agent_id = $1")
            .bind(&agent_id)
            .fetch_optional(&state.pool)
            .await?;
    manifest
        .map(|m| Json(m.0))
        .ok_or_else(|| ApiError::not_found("agent not found"))
}

/// List delegation edges involving an agent.
pub async fn agent_delegations(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Vec<mesh_protocol::DelegationView>>, ApiError> {
    let rows = sqlx::query(
        "SELECT j.job_id, j.parent_job_id, j.root_job_id, j.status, j.created_at, \
             'delegatee' AS role \
         FROM jobs j WHERE j.kind = 'agent_task' AND j.assignee_agent_id = $1 \
         UNION ALL \
         SELECT j.job_id, j.parent_job_id, j.root_job_id, j.status, j.created_at, \
             'delegator' AS role \
         FROM jobs j JOIN jobs p ON p.job_id = j.parent_job_id \
         WHERE j.kind = 'agent_task' AND p.assignee_agent_id = $1 \
         ORDER BY created_at DESC",
    )
    .bind(&agent_id)
    .fetch_all(&state.pool)
    .await?;

    let mut delegations = Vec::with_capacity(rows.len());
    for row in rows {
        let status_raw: String = row.try_get("status")?;
        delegations.push(mesh_protocol::DelegationView {
            job_id: mesh_protocol::JobId::new(row.try_get::<String, _>("job_id")?),
            parent_job_id: row
                .try_get::<Option<String>, _>("parent_job_id")?
                .map(mesh_protocol::JobId::new),
            root_job_id: row
                .try_get::<Option<String>, _>("root_job_id")?
                .map(mesh_protocol::JobId::new),
            role: row.try_get("role")?,
            state: status_raw.parse().map_err(ApiError::internal)?,
            created_at: row.try_get("created_at")?,
        });
    }
    Ok(Json(delegations))
}

/// Aggregated rewards credited to an agent.
pub async fn agent_rewards(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<AgentRewardView>, ApiError> {
    let agent_id = AgentId::new(agent_id);
    let rewards = mesh_accounting::agent_rewards(&state.pool, &agent_id).await?;
    Ok(Json(rewards))
}

#[derive(Debug, Deserialize)]
pub struct AgentClaimQuery {
    workloads: Option<String>,
}

/// Claim the next agent task assigned to this agent, or 204 when there is none.
pub async fn next_agent_job(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(query): Query<AgentClaimQuery>,
) -> Result<Response, ApiError> {
    let agent_id = AgentId::new(agent_id);

    let row = sqlx::query(
        "SELECT a.node_id, a.status, n.capabilities \
         FROM agents a JOIN nodes n ON n.node_id = a.node_id \
         WHERE a.agent_id = $1",
    )
    .bind(agent_id.as_str())
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        return Err(ApiError::not_found("agent is not registered"));
    };

    let status_raw: String = row.try_get("status")?;
    let status: AgentStatus = status_raw.parse().map_err(ApiError::internal)?;
    if !status.accepts_work() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "agent_offline",
            "agent is not online",
        ));
    }

    let node_id: String = row.try_get("node_id")?;
    let capabilities: SqlJson<NodeCapabilities> = row.try_get("capabilities")?;
    let supported_workloads = query
        .workloads
        .unwrap_or_else(|| "echo,wasm".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let context = WorkerContext {
        worker_id: NodeId::new(node_id),
        cpu_cores: capabilities.0.cpu_cores,
        ram_gb: capabilities.0.ram_gb,
        supported_workloads,
    };

    match mesh_scheduler::claim_job(
        &state.pool,
        &context,
        state.config.job_lease_seconds as i64,
        &mesh_scheduler::ClaimFilter::agent_tasks(agent_id),
    )
    .await?
    {
        Some(job) => Ok((StatusCode::OK, Json(job)).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

/// Register (or re-register) an agent on a worker.
pub async fn register_agent(
    State(state): State<AppState>,
    Json(req): Json<RegisterAgentRequest>,
) -> Result<(StatusCode, Json<RegisterAgentResponse>), ApiError> {
    if req.manifest.manifest.node_id != req.node_id {
        return Err(ApiError::bad_request(
            "manifest node_id does not match request node_id",
        ));
    }

    let participant_id: Option<String> =
        sqlx::query_scalar("SELECT participant_id FROM nodes WHERE node_id = $1")
            .bind(req.node_id.as_str())
            .fetch_optional(&state.pool)
            .await?;
    let participant_id =
        participant_id.ok_or_else(|| ApiError::bad_request("node is not registered"))?;

    verify_manifest(&req.manifest)?;

    persist_agent(&state, &participant_id, &req.manifest).await?;

    tracing::info!(
        agent_id = %req.manifest.manifest.agent_id,
        node_id = %req.node_id,
        "agent registered"
    );

    Ok((
        StatusCode::CREATED,
        Json(RegisterAgentResponse {
            agent_id: req.manifest.manifest.agent_id.clone(),
            status: AgentStatus::Online,
            heartbeat_interval_seconds: state.config.heartbeat_interval_seconds,
        }),
    ))
}

/// Record an agent heartbeat.
pub async fn heartbeat(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<AgentHeartbeatRequest>,
) -> Result<Json<AgentHeartbeatResponse>, ApiError> {
    if req.manifest.manifest.agent_id.as_str() != agent_id {
        return Err(ApiError::bad_request(
            "manifest agent_id does not match path agent_id",
        ));
    }

    let row = sqlx::query("SELECT public_key, participant_id FROM agents WHERE agent_id = $1")
        .bind(&agent_id)
        .fetch_optional(&state.pool)
        .await?;
    let row = row.ok_or_else(|| ApiError::not_found("agent is not registered"))?;
    let stored_key: String = row.try_get("public_key")?;
    let participant_id: String = row.try_get("participant_id")?;

    if stored_key != req.manifest.public_key {
        return Err(ApiError::bad_request(
            "manifest public_key does not match registered agent",
        ));
    }
    verify_manifest(&req.manifest)?;

    persist_agent(&state, &participant_id, &req.manifest).await?;

    Ok(Json(AgentHeartbeatResponse {
        agent_id: AgentId::new(agent_id),
        status: AgentStatus::Online,
        server_time: Utc::now(),
        heartbeat_interval_seconds: state.config.heartbeat_interval_seconds,
    }))
}

/// List all registered agents.
pub async fn list_agents(State(state): State<AppState>) -> Result<Json<Vec<AgentView>>, ApiError> {
    let rows = agent_query(&state, None).await?;
    Ok(Json(rows))
}

/// Inspect a single agent.
pub async fn inspect_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<AgentView>, ApiError> {
    let mut agents = agent_query(&state, Some(&agent_id)).await?;
    agents
        .pop()
        .map(Json)
        .ok_or_else(|| ApiError::not_found("agent not found"))
}

/// Verify the manifest's signature and expiry.
fn verify_manifest(signed: &SignedAgentManifest) -> Result<(), ApiError> {
    if signed.manifest.is_expired_at(Utc::now()) {
        return Err(ApiError::bad_request("manifest has expired"));
    }
    NodeIdentity::verify_canonical(&signed.public_key, &signed.manifest, &signed.signature)
        .map_err(|_| ApiError::bad_request("invalid manifest signature"))
}

async fn persist_agent(
    state: &AppState,
    participant_id: &str,
    signed: &SignedAgentManifest,
) -> Result<(), ApiError> {
    let manifest = &signed.manifest;
    let hash = manifest_hash(manifest);
    let backend = manifest.capabilities.models.first().cloned();

    sqlx::query(
        "INSERT INTO agents \
             (agent_id, node_id, participant_id, public_key, manifest, manifest_hash, backend, \
              status, last_heartbeat) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'online', now()) \
         ON CONFLICT (agent_id) DO UPDATE SET \
             node_id = EXCLUDED.node_id, \
             participant_id = EXCLUDED.participant_id, \
             public_key = EXCLUDED.public_key, \
             manifest = EXCLUDED.manifest, \
             manifest_hash = EXCLUDED.manifest_hash, \
             backend = EXCLUDED.backend, \
             status = 'online', \
             last_heartbeat = now(), \
             updated_at = now()",
    )
    .bind(manifest.agent_id.as_str())
    .bind(manifest.node_id.as_str())
    .bind(participant_id)
    .bind(&signed.public_key)
    .bind(SqlJson(manifest))
    .bind(&hash)
    .bind(&backend)
    .execute(&state.pool)
    .await?;

    let caps = &manifest.capabilities;
    sqlx::query(
        "INSERT INTO agent_capabilities \
             (agent_id, skills, tools, models, max_concurrent_tasks, max_depth_supported, \
              accepts_delegation, min_budget_credits, max_task_seconds, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now()) \
         ON CONFLICT (agent_id) DO UPDATE SET \
             skills = EXCLUDED.skills, \
             tools = EXCLUDED.tools, \
             models = EXCLUDED.models, \
             max_concurrent_tasks = EXCLUDED.max_concurrent_tasks, \
             max_depth_supported = EXCLUDED.max_depth_supported, \
             accepts_delegation = EXCLUDED.accepts_delegation, \
             min_budget_credits = EXCLUDED.min_budget_credits, \
             max_task_seconds = EXCLUDED.max_task_seconds, \
             updated_at = now()",
    )
    .bind(manifest.agent_id.as_str())
    .bind(SqlJson(&caps.skills))
    .bind(SqlJson(&caps.tools))
    .bind(SqlJson(&caps.models))
    .bind(caps.max_concurrent_tasks as i32)
    .bind(caps.max_depth_supported as i32)
    .bind(manifest.policy.accepts_delegation)
    .bind(manifest.policy.min_budget_credits)
    .bind(manifest.policy.max_task_seconds as i64)
    .execute(&state.pool)
    .await?;

    Ok(())
}

async fn agent_query(state: &AppState, agent_id: Option<&str>) -> Result<Vec<AgentView>, ApiError> {
    let base = "SELECT a.agent_id, a.node_id, a.participant_id, a.status, a.manifest_hash, \
                       a.backend, a.last_heartbeat, \
                       c.skills, c.tools, c.models, c.max_concurrent_tasks, c.max_depth_supported, \
                       c.accepts_delegation, c.min_budget_credits, c.max_task_seconds \
                FROM agents a JOIN agent_capabilities c ON c.agent_id = a.agent_id";

    let rows = match agent_id {
        Some(id) => {
            sqlx::query(&format!("{base} WHERE a.agent_id = $1"))
                .bind(id)
                .fetch_all(&state.pool)
                .await?
        }
        None => {
            sqlx::query(&format!("{base} ORDER BY a.agent_id"))
                .fetch_all(&state.pool)
                .await?
        }
    };

    let mut agents = Vec::with_capacity(rows.len());
    for row in rows {
        let skills: SqlJson<Vec<String>> = row.try_get("skills")?;
        let tools: SqlJson<Vec<String>> = row.try_get("tools")?;
        let models: SqlJson<Vec<String>> = row.try_get("models")?;
        let status_raw: String = row.try_get("status")?;
        let last_heartbeat: Option<DateTime<Utc>> = row.try_get("last_heartbeat")?;

        agents.push(AgentView {
            agent_id: AgentId::new(row.try_get::<String, _>("agent_id")?),
            node_id: NodeId::new(row.try_get::<String, _>("node_id")?),
            participant_id: ParticipantId::new(row.try_get::<String, _>("participant_id")?),
            status: status_raw.parse().map_err(ApiError::internal)?,
            capabilities: AgentCapabilities {
                skills: skills.0,
                tools: tools.0,
                models: models.0,
                max_concurrent_tasks: row.try_get::<i32, _>("max_concurrent_tasks")? as u32,
                max_depth_supported: row.try_get::<i32, _>("max_depth_supported")? as u32,
            },
            policy: AgentPolicy {
                accepts_delegation: row.try_get("accepts_delegation")?,
                min_budget_credits: row.try_get("min_budget_credits")?,
                max_task_seconds: row.try_get::<i64, _>("max_task_seconds")? as u64,
            },
            manifest_hash: row.try_get("manifest_hash")?,
            backend: row.try_get("backend")?,
            last_heartbeat,
        });
    }
    Ok(agents)
}
