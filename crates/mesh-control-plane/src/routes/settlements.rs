//! Settlement batch endpoints.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use mesh_protocol::BatchView;

use crate::error::ApiError;
use crate::AppState;

/// Settlement routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/settlements", get(list_settlements))
}

#[derive(Debug, Deserialize)]
pub struct BatchQuery {
    limit: Option<i64>,
}

/// List recent reward batches.
pub async fn list_settlements(
    State(state): State<AppState>,
    Query(query): Query<BatchQuery>,
) -> Result<Json<Vec<BatchView>>, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let batches = mesh_settlement::list_batches(&state.pool, limit).await?;
    Ok(Json(batches))
}
