//! Node registry endpoints: registration, heartbeat, and listing.
//!
//! Every registration and heartbeat carries a freshly signed resource offer.
//! The control plane verifies the signature over the canonical payload and the
//! offer's expiry before trusting the advertised capabilities.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use sqlx::types::Json as SqlJson;
use sqlx::Row;

use mesh_identity::NodeIdentity;
use mesh_protocol::{
    HeartbeatRequest, HeartbeatResponse, NodeCapabilities, NodeId, NodeStatus, NodeView,
    ParticipantId, RegisterNodeRequest, RegisterNodeResponse, SignedResourceOffer,
};

use crate::error::ApiError;
use crate::AppState;

/// Node registry routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/nodes", get(list_nodes))
        .route("/api/v1/nodes/register", post(register_node))
        .route("/api/v1/nodes/:node_id/heartbeat", post(heartbeat))
        .route("/api/v1/nodes/:node_id/offer", get(get_offer))
        .route("/api/v1/nodes/:node_id/offer/revoke", post(revoke_offer))
}

/// Fetch a node's current signed offer.
pub async fn get_offer(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let row = sqlx::query(
        "SELECT offer, public_key, signature, expires_at, revoked_at \
         FROM resource_offers WHERE node_id = $1",
    )
    .bind(&node_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        return Err(ApiError::not_found("no offer for node"));
    };
    let offer: SqlJson<serde_json::Value> = row.try_get("offer")?;
    let expires_at: DateTime<Utc> = row.try_get("expires_at")?;
    let revoked_at: Option<DateTime<Utc>> = row.try_get("revoked_at")?;
    let public_key: String = row.try_get("public_key")?;
    let signature: String = row.try_get("signature")?;
    Ok(Json(serde_json::json!({
        "node_id": node_id,
        "offer": offer.0,
        "public_key": public_key,
        "signature": signature,
        "expires_at": expires_at,
        "revoked_at": revoked_at,
        "active": revoked_at.is_none() && expires_at > Utc::now(),
    })))
}

/// Revoke a node's offer so it is no longer scheduled.
pub async fn revoke_offer(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let affected = sqlx::query(
        "UPDATE resource_offers SET revoked_at = now(), updated_at = now() WHERE node_id = $1",
    )
    .bind(&node_id)
    .execute(&state.pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(ApiError::not_found("no offer for node"));
    }
    tracing::info!(node_id = %node_id, "resource offer revoked");
    Ok(Json(
        serde_json::json!({ "node_id": node_id, "revoked": true }),
    ))
}

/// Register (or re-register) a worker node.
pub async fn register_node(
    State(state): State<AppState>,
    Json(req): Json<RegisterNodeRequest>,
) -> Result<(StatusCode, Json<RegisterNodeResponse>), ApiError> {
    if req.public_key.trim().is_empty() {
        return Err(ApiError::bad_request("public_key must not be empty"));
    }
    if req.offer.offer.node_id != req.node_id {
        return Err(ApiError::bad_request(
            "offer node_id does not match request node_id",
        ));
    }
    verify_offer(&req.public_key, &req.offer)?;

    sqlx::query(
        "INSERT INTO participants (participant_id, public_key, status) \
         VALUES ($1, $2, 'active') \
         ON CONFLICT (participant_id) DO NOTHING",
    )
    .bind(req.participant_id.as_str())
    .bind(&req.public_key)
    .execute(&state.pool)
    .await?;

    sqlx::query(
        "INSERT INTO nodes (node_id, participant_id, public_key, capabilities, status, last_heartbeat) \
         VALUES ($1, $2, $3, $4, 'online', now()) \
         ON CONFLICT (node_id) DO UPDATE SET \
             participant_id = EXCLUDED.participant_id, \
             public_key = EXCLUDED.public_key, \
             capabilities = EXCLUDED.capabilities, \
             status = 'online', \
             last_heartbeat = now(), \
             updated_at = now()",
    )
    .bind(req.node_id.as_str())
    .bind(req.participant_id.as_str())
    .bind(&req.public_key)
    .bind(SqlJson(&req.offer.offer.capabilities))
    .execute(&state.pool)
    .await?;

    store_offer(&state, &req.node_id, &req.offer).await?;

    tracing::info!(node_id = %req.node_id, participant_id = %req.participant_id, "node registered");

    Ok((
        StatusCode::CREATED,
        Json(RegisterNodeResponse {
            node_id: req.node_id,
            status: NodeStatus::Online,
            heartbeat_interval_seconds: state.config.heartbeat_interval_seconds,
        }),
    ))
}

/// Record a heartbeat and refresh the node's advertised capabilities.
pub async fn heartbeat(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<HeartbeatResponse>, ApiError> {
    if req.offer.offer.node_id.as_str() != node_id {
        return Err(ApiError::bad_request(
            "offer node_id does not match path node_id",
        ));
    }

    let stored_key: Option<String> =
        sqlx::query_scalar("SELECT public_key FROM nodes WHERE node_id = $1")
            .bind(&node_id)
            .fetch_optional(&state.pool)
            .await?;
    let stored_key = stored_key.ok_or_else(|| ApiError::not_found("node not registered"))?;

    if stored_key != req.offer.public_key {
        return Err(ApiError::bad_request(
            "offer public_key does not match registered node",
        ));
    }
    verify_offer(&stored_key, &req.offer)?;

    sqlx::query(
        "UPDATE nodes SET capabilities = $2, status = 'online', \
             last_heartbeat = now(), updated_at = now() \
         WHERE node_id = $1",
    )
    .bind(&node_id)
    .bind(SqlJson(&req.offer.offer.capabilities))
    .execute(&state.pool)
    .await?;

    store_offer(&state, &NodeId::new(node_id.clone()), &req.offer).await?;

    Ok(Json(HeartbeatResponse {
        node_id: NodeId::new(node_id),
        status: NodeStatus::Online,
        server_time: Utc::now(),
        heartbeat_interval_seconds: state.config.heartbeat_interval_seconds,
    }))
}

/// List all registered nodes.
pub async fn list_nodes(State(state): State<AppState>) -> Result<Json<Vec<NodeView>>, ApiError> {
    let rows = sqlx::query(
        "SELECT node_id, participant_id, capabilities, status, last_heartbeat \
         FROM nodes ORDER BY node_id",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut nodes = Vec::with_capacity(rows.len());
    for row in rows {
        let node_id: String = row.try_get("node_id")?;
        let participant_id: String = row.try_get("participant_id")?;
        let capabilities: SqlJson<NodeCapabilities> = row.try_get("capabilities")?;
        let status_raw: String = row.try_get("status")?;
        let last_heartbeat: Option<DateTime<Utc>> = row.try_get("last_heartbeat")?;

        nodes.push(NodeView {
            node_id: NodeId::new(node_id),
            participant_id: ParticipantId::new(participant_id),
            capabilities: capabilities.0,
            status: status_raw.parse().map_err(ApiError::internal)?,
            last_heartbeat,
        });
    }

    Ok(Json(nodes))
}

/// Verify a signed offer: key match, expiry, and signature over canonical bytes.
fn verify_offer(expected_public_key: &str, signed: &SignedResourceOffer) -> Result<(), ApiError> {
    if signed.public_key != expected_public_key {
        return Err(ApiError::bad_request(
            "offer public_key does not match registered key",
        ));
    }
    if signed.offer.is_expired_at(Utc::now()) {
        return Err(ApiError::bad_request("offer has expired"));
    }
    NodeIdentity::verify_canonical(&signed.public_key, &signed.offer, &signed.signature)
        .map_err(|_| ApiError::bad_request("invalid offer signature"))
}

/// Persist the latest offer for a node, replacing any previous one.
async fn store_offer(
    state: &AppState,
    node_id: &NodeId,
    signed: &SignedResourceOffer,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO resource_offers (node_id, offer, public_key, signature, expires_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, now()) \
         ON CONFLICT (node_id) DO UPDATE SET \
             offer = EXCLUDED.offer, \
             public_key = EXCLUDED.public_key, \
             signature = EXCLUDED.signature, \
             expires_at = EXCLUDED.expires_at, \
             updated_at = now()",
    )
    .bind(node_id.as_str())
    .bind(SqlJson(&signed.offer))
    .bind(&signed.public_key)
    .bind(&signed.signature)
    .bind(signed.offer.expires_at)
    .execute(&state.pool)
    .await?;
    Ok(())
}
