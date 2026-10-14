//! Prometheus-style metrics.

use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use crate::error::ApiError;
use crate::AppState;

/// Metrics route.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/metrics", get(metrics))
}

/// Render process and network counters in the Prometheus text exposition format.
pub async fn metrics(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let nodes_online: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM nodes WHERE status = 'online'")
            .fetch_one(&state.pool)
            .await?;
    let agents_online: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agents WHERE status = 'online'")
            .fetch_one(&state.pool)
            .await?;
    let jobs_pending: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = 'pending'")
            .fetch_one(&state.pool)
            .await?;
    let jobs_running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE status IN ('scheduled', 'running', 'waiting_on_children')",
    )
    .fetch_one(&state.pool)
    .await?;
    let jobs_succeeded: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = 'succeeded'")
            .fetch_one(&state.pool)
            .await?;
    let jobs_failed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = 'failed'")
        .fetch_one(&state.pool)
        .await?;
    let receipts_issued: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM contribution_receipts WHERE status = 'issued'")
            .fetch_one(&state.pool)
            .await?;
    let receipts_void: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM contribution_receipts WHERE status = 'void'")
            .fetch_one(&state.pool)
            .await?;
    let storage_objects: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM storage_objects")
        .fetch_one(&state.pool)
        .await?;
    let abuse_reports: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM abuse_reports")
        .fetch_one(&state.pool)
        .await?;
    let peers = state.registry.len();

    let body = format!(
        "# TYPE meshcloud_nodes_online gauge\nmeshcloud_nodes_online {nodes_online}\n\
         # TYPE meshcloud_agents_online gauge\nmeshcloud_agents_online {agents_online}\n\
         # TYPE meshcloud_jobs_pending gauge\nmeshcloud_jobs_pending {jobs_pending}\n\
         # TYPE meshcloud_jobs_running gauge\nmeshcloud_jobs_running {jobs_running}\n\
         # TYPE meshcloud_jobs_succeeded_total counter\nmeshcloud_jobs_succeeded_total {jobs_succeeded}\n\
         # TYPE meshcloud_jobs_failed_total counter\nmeshcloud_jobs_failed_total {jobs_failed}\n\
         # TYPE meshcloud_receipts_issued gauge\nmeshcloud_receipts_issued {receipts_issued}\n\
         # TYPE meshcloud_receipts_void gauge\nmeshcloud_receipts_void {receipts_void}\n\
         # TYPE meshcloud_storage_objects gauge\nmeshcloud_storage_objects {storage_objects}\n\
         # TYPE meshcloud_abuse_reports_total counter\nmeshcloud_abuse_reports_total {abuse_reports}\n\
         # TYPE meshcloud_mesh_peers gauge\nmeshcloud_mesh_peers {peers}\n"
    );

    Ok((
        [(CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    ))
}
