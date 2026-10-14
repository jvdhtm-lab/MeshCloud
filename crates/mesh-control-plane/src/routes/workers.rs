//! Worker-facing routes: claim eligible work.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use sqlx::types::Json as SqlJson;

use mesh_protocol::{ClaimedJobResponse, NodeCapabilities, NodeId, NodeStatus};
use mesh_scheduler::WorkerContext;

use crate::error::ApiError;
use crate::AppState;

/// Worker routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/workers/:worker_id/jobs/next", get(next_job))
}

#[derive(Debug, Deserialize)]
pub struct ClaimQuery {
    /// Comma-separated list of workload types the worker supports.
    workloads: Option<String>,
}

/// Claim the next eligible job for a worker, or 204 when there is none.
pub async fn next_job(
    State(state): State<AppState>,
    Path(worker_id): Path<String>,
    Query(query): Query<ClaimQuery>,
) -> Result<Response, ApiError> {
    let worker_id = NodeId::new(worker_id);

    let row = sqlx::query("SELECT capabilities, status FROM nodes WHERE node_id = $1")
        .bind(worker_id.as_str())
        .fetch_optional(&state.pool)
        .await?;
    let Some(row) = row else {
        return Err(ApiError::not_found("worker is not registered"));
    };

    use sqlx::Row;
    let capabilities: SqlJson<NodeCapabilities> = row.try_get("capabilities")?;
    let status_raw: String = row.try_get("status")?;
    let status: NodeStatus = status_raw.parse().map_err(ApiError::internal)?;
    if !status.accepts_work() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "worker_offline",
            "worker is not online",
        ));
    }

    let supported_workloads = query
        .workloads
        .unwrap_or_else(|| "echo".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let context = WorkerContext {
        worker_id,
        cpu_cores: capabilities.0.cpu_cores,
        ram_gb: capabilities.0.ram_gb,
        supported_workloads,
    };

    match mesh_scheduler::claim_job(
        &state.pool,
        &context,
        state.config.job_lease_seconds as i64,
        &mesh_scheduler::ClaimFilter::worker_jobs(),
    )
    .await?
    {
        Some(job) => Ok((StatusCode::OK, Json::<ClaimedJobResponse>(job)).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}
