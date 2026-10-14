//! Contribution receipts and credit balances.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use mesh_protocol::ids::ReceiptId;
use mesh_protocol::{AgentId, ContributionView, CreditBalanceView, DisputeRequest, DisputeView, ParticipantId};

use crate::error::ApiError;
use crate::AppState;

/// Contribution routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/contributions", get(list_contributions))
        .route(
            "/api/v1/contributions/:receipt_id/dispute",
            post(dispute_contribution),
        )
        .route(
            "/api/v1/participants/:participant_id/balance",
            get(participant_balance),
        )
}

#[derive(Debug, Deserialize)]
pub struct ContributionQuery {
    limit: Option<i64>,
    agent: Option<String>,
}

/// List recent contribution receipts, optionally filtered by agent.
pub async fn list_contributions(
    State(state): State<AppState>,
    Query(query): Query<ContributionQuery>,
) -> Result<Json<Vec<ContributionView>>, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let agent = query.agent.map(AgentId::new);
    let contributions =
        mesh_accounting::list_contributions(&state.pool, limit, agent.as_ref(), None).await?;
    Ok(Json(contributions))
}

/// Fetch a participant's credit balance.
pub async fn participant_balance(
    State(state): State<AppState>,
    Path(participant_id): Path<String>,
) -> Result<Json<CreditBalanceView>, ApiError> {
    let participant_id = ParticipantId::new(participant_id);
    mesh_accounting::participant_balance(&state.pool, &participant_id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("participant has no balance"))
}

/// Dispute a receipt: void it and reverse its credit.
pub async fn dispute_contribution(
    State(state): State<AppState>,
    Path(receipt_id): Path<String>,
    Json(req): Json<DisputeRequest>,
) -> Result<(StatusCode, Json<DisputeView>), ApiError> {
    if req.reason.trim().is_empty() {
        return Err(ApiError::bad_request("reason must not be empty"));
    }
    let receipt_id = ReceiptId::new(receipt_id);
    match mesh_accounting::dispute_contribution(&state.pool, &receipt_id, &req.reason).await? {
        Some(dispute) => {
            tracing::warn!(receipt = %receipt_id, "contribution disputed and voided");
            Ok((StatusCode::CREATED, Json(dispute)))
        }
        None => Err(ApiError::not_found("receipt not found or already void")),
    }
}
