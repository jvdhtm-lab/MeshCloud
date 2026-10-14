//! Liveness and readiness endpoints.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};

use mesh_protocol::{HealthResponse, ReadyResponse, PROTOCOL_VERSION};

use crate::AppState;

/// Health and readiness routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/ready", get(ready))
}

/// Process liveness. Never touches the database.
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: PROTOCOL_VERSION,
        uptime_seconds: state.started_at.elapsed().as_secs(),
    })
}

/// Readiness: every dependency must be usable.
pub async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    let mut checks = std::collections::BTreeMap::new();
    let mut healthy = true;

    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
    {
        Ok(_) => {
            checks.insert("database".to_string(), "ok".to_string());
        }
        Err(err) => {
            healthy = false;
            tracing::warn!(error = %err, "readiness check failed");
            checks.insert("database".to_string(), "unavailable".to_string());
        }
    }

    let status = if healthy { "ok" } else { "degraded" };
    let code = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        code,
        Json(ReadyResponse {
            status: status.to_string(),
            checks,
        }),
    )
}
