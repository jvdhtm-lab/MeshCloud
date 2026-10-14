//! Participant accounts, wallets, reservations, and history.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::Row;

use mesh_protocol::{
    ContributionView, CreateReservationRequest, ParticipantId, ParticipantView, ReservationView,
    SetWalletRequest,
};

use crate::error::ApiError;
use crate::AppState;

/// Participant routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/participants", get(list_participants))
        .route("/api/v1/participants/:participant_id", get(get_participant))
        .route(
            "/api/v1/participants/:participant_id/wallet",
            post(set_wallet),
        )
        .route(
            "/api/v1/participants/:participant_id/reservations",
            get(list_reservations).post(create_reservation),
        )
        .route(
            "/api/v1/participants/:participant_id/contributions",
            get(participant_contributions),
        )
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    limit: Option<i64>,
}

fn participant_view(row: &sqlx::postgres::PgRow) -> Result<ParticipantView, ApiError> {
    Ok(ParticipantView {
        participant_id: ParticipantId::new(row.try_get::<String, _>("participant_id")?),
        public_key: row.try_get("public_key")?,
        wallet_address: row.try_get("wallet_address")?,
        status: row.try_get("status")?,
        created_at: row.try_get::<DateTime<Utc>, _>("created_at")?,
    })
}

/// List participant accounts.
pub async fn list_participants(
    State(state): State<AppState>,
) -> Result<Json<Vec<ParticipantView>>, ApiError> {
    let rows = sqlx::query(
        "SELECT participant_id, public_key, wallet_address, status, created_at \
         FROM participants ORDER BY created_at DESC",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut participants = Vec::with_capacity(rows.len());
    for row in rows {
        participants.push(participant_view(&row)?);
    }
    Ok(Json(participants))
}

/// Fetch a single participant.
pub async fn get_participant(
    State(state): State<AppState>,
    Path(participant_id): Path<String>,
) -> Result<Json<ParticipantView>, ApiError> {
    let row = sqlx::query(
        "SELECT participant_id, public_key, wallet_address, status, created_at \
         FROM participants WHERE participant_id = $1",
    )
    .bind(&participant_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        return Err(ApiError::not_found("participant not found"));
    };
    Ok(Json(participant_view(&row)?))
}

/// Associate a wallet address with a participant.
pub async fn set_wallet(
    State(state): State<AppState>,
    Path(participant_id): Path<String>,
    Json(req): Json<SetWalletRequest>,
) -> Result<Json<ParticipantView>, ApiError> {
    if req.wallet_address.trim().is_empty() {
        return Err(ApiError::bad_request("wallet_address must not be empty"));
    }
    let affected = sqlx::query(
        "UPDATE participants SET wallet_address = $2, updated_at = now() WHERE participant_id = $1",
    )
    .bind(&participant_id)
    .bind(req.wallet_address.trim())
    .execute(&state.pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(ApiError::not_found("participant not found"));
    }
    let row = sqlx::query(
        "SELECT participant_id, public_key, wallet_address, status, created_at \
         FROM participants WHERE participant_id = $1",
    )
    .bind(&participant_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(participant_view(&row)?))
}

/// List a participant's reservations.
pub async fn list_reservations(
    State(state): State<AppState>,
    Path(participant_id): Path<String>,
) -> Result<Json<Vec<ReservationView>>, ApiError> {
    let participant_id = ParticipantId::new(participant_id);
    let reservations = mesh_accounting::list_reservations(&state.pool, &participant_id).await?;
    Ok(Json(reservations))
}

/// Reserve credits for a participant, subject to the configured quota.
pub async fn create_reservation(
    State(state): State<AppState>,
    Path(participant_id): Path<String>,
    Json(req): Json<CreateReservationRequest>,
) -> Result<(StatusCode, Json<ReservationView>), ApiError> {
    if req.credits <= 0 {
        return Err(ApiError::bad_request("credits must be positive"));
    }
    let participant_id = ParticipantId::new(participant_id);

    if state.config.participant_quota_credits > 0 {
        let outstanding =
            mesh_accounting::outstanding_reserved(&state.pool, &participant_id).await?;
        if outstanding + req.credits > state.config.participant_quota_credits {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "quota_exceeded",
                "reservation would exceed the participant quota",
            ));
        }
    }

    match mesh_accounting::create_reservation(
        &state.pool,
        &participant_id,
        &req.workload_type,
        req.credits,
        req.ttl_seconds,
    )
    .await?
    {
        Some(reservation) => Ok((StatusCode::CREATED, Json(reservation))),
        None => Err(ApiError::new(
            StatusCode::CONFLICT,
            "insufficient_credits",
            "not enough available credits to reserve",
        )),
    }
}

/// A participant's contribution (reward) history.
pub async fn participant_contributions(
    State(state): State<AppState>,
    Path(participant_id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<ContributionView>>, ApiError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let participant_id = ParticipantId::new(participant_id);
    let contributions =
        mesh_accounting::list_contributions(&state.pool, limit, None, Some(&participant_id))
            .await?;
    Ok(Json(contributions))
}
