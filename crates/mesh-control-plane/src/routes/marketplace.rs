//! Marketplace views: filter available compute and agents, and estimate prices.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::types::Json as SqlJson;
use sqlx::Row;

use mesh_agent_protocol::{AgentCapabilities, AgentPolicy, AgentStatus, AgentView};
use mesh_protocol::ids::{AgentId, NodeId, ParticipantId};
use mesh_protocol::{NodeCapabilities, NodeView, PriceEstimate};

use crate::error::ApiError;
use crate::AppState;

/// Marketplace routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/marketplace/nodes", get(list_nodes))
        .route("/api/v1/marketplace/agents", get(list_agents))
        .route("/api/v1/marketplace/price", get(price))
}

#[derive(Debug, Deserialize)]
pub struct NodesQuery {
    min_cpu_cores: Option<u32>,
    min_ram_gb: Option<u32>,
    gpu: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct AgentsQuery {
    skill: Option<String>,
    tool: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PriceQuery {
    workload_type: String,
    units: Option<i64>,
}

/// List online nodes with a valid, unrevoked offer matching the filters.
pub async fn list_nodes(
    State(state): State<AppState>,
    Query(query): Query<NodesQuery>,
) -> Result<Json<Vec<NodeView>>, ApiError> {
    let rows = sqlx::query(
        "SELECT n.node_id, n.participant_id, n.capabilities, n.status, n.last_heartbeat \
         FROM nodes n \
         WHERE n.status = 'online' \
           AND EXISTS ( \
               SELECT 1 FROM resource_offers o \
               WHERE o.node_id = n.node_id AND o.revoked_at IS NULL AND o.expires_at > now()) \
           AND (n.capabilities->>'cpu_cores')::int >= $1 \
           AND (n.capabilities->>'ram_gb')::int >= $2 \
           AND ($3::bool IS NULL OR (n.capabilities->>'gpu')::bool = $3) \
         ORDER BY n.node_id",
    )
    .bind(query.min_cpu_cores.unwrap_or(0) as i32)
    .bind(query.min_ram_gb.unwrap_or(0) as i32)
    .bind(query.gpu)
    .fetch_all(&state.pool)
    .await?;

    let mut nodes = Vec::with_capacity(rows.len());
    for row in rows {
        let capabilities: SqlJson<NodeCapabilities> = row.try_get("capabilities")?;
        let status_raw: String = row.try_get("status")?;
        nodes.push(NodeView {
            node_id: NodeId::new(row.try_get::<String, _>("node_id")?),
            participant_id: ParticipantId::new(row.try_get::<String, _>("participant_id")?),
            capabilities: capabilities.0,
            status: status_raw.parse().map_err(ApiError::internal)?,
            last_heartbeat: row.try_get::<Option<DateTime<Utc>>, _>("last_heartbeat")?,
        });
    }
    Ok(Json(nodes))
}

/// List online agents matching the capability filters.
pub async fn list_agents(
    State(state): State<AppState>,
    Query(query): Query<AgentsQuery>,
) -> Result<Json<Vec<AgentView>>, ApiError> {
    let skill = query.skill.map(|s| SqlJson(serde_json::json!([s])));
    let tool = query.tool.map(|t| SqlJson(serde_json::json!([t])));
    let model = query.model.map(|m| SqlJson(serde_json::json!([m])));

    let rows = sqlx::query(
        "SELECT a.agent_id, a.node_id, a.participant_id, a.status, a.manifest_hash, a.backend, \
             a.last_heartbeat, c.skills, c.tools, c.models, c.max_concurrent_tasks, \
             c.max_depth_supported, c.accepts_delegation, c.min_budget_credits, c.max_task_seconds \
         FROM agents a JOIN agent_capabilities c ON c.agent_id = a.agent_id \
         WHERE a.status = 'online' \
           AND ($1::jsonb IS NULL OR c.skills @> $1::jsonb) \
           AND ($2::jsonb IS NULL OR c.tools @> $2::jsonb) \
           AND ($3::jsonb IS NULL OR c.models @> $3::jsonb) \
         ORDER BY a.agent_id",
    )
    .bind(skill)
    .bind(tool)
    .bind(model)
    .fetch_all(&state.pool)
    .await?;

    let mut agents = Vec::with_capacity(rows.len());
    for row in rows {
        let skills: SqlJson<Vec<String>> = row.try_get("skills")?;
        let tools: SqlJson<Vec<String>> = row.try_get("tools")?;
        let models: SqlJson<Vec<String>> = row.try_get("models")?;
        let status_raw: String = row.try_get("status")?;
        agents.push(AgentView {
            agent_id: AgentId::new(row.try_get::<String, _>("agent_id")?),
            node_id: NodeId::new(row.try_get::<String, _>("node_id")?),
            participant_id: ParticipantId::new(row.try_get::<String, _>("participant_id")?),
            status: status_raw
                .parse::<AgentStatus>()
                .map_err(ApiError::internal)?,
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
            last_heartbeat: row.try_get::<Option<DateTime<Utc>>, _>("last_heartbeat")?,
        });
    }
    Ok(Json(agents))
}

/// Estimate the price of a workload.
pub async fn price(
    State(state): State<AppState>,
    Query(query): Query<PriceQuery>,
) -> Result<Json<PriceEstimate>, ApiError> {
    let units = query.units.unwrap_or(1).max(0);
    let unit_rate = state.config.reward_rate_per_unit;
    Ok(Json(PriceEstimate {
        workload_type: query.workload_type,
        unit_rate,
        units,
        estimated_credits: units.saturating_mul(unit_rate),
    }))
}
