//! Distributed storage endpoints: upload, download, repair, and challenges.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use chrono::Utc;
use sqlx::Row;

use mesh_protocol::ids::{AttemptId, NodeId, ParticipantId};
use mesh_protocol::{
    ChallengeObjectResponse, DownloadObjectResponse, RepairObjectResponse, StorageObjectView,
    StorageShardView, UploadObjectRequest,
};

use crate::error::ApiError;
use crate::storage as shard_store;
use crate::AppState;

/// Storage routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/storage/objects",
            get(list_objects).post(upload_object),
        )
        .route("/api/v1/storage/objects/:object_id", get(get_object))
        .route(
            "/api/v1/storage/objects/:object_id/download",
            get(download_object),
        )
        .route(
            "/api/v1/storage/objects/:object_id/repair",
            post(repair_object),
        )
        .route(
            "/api/v1/storage/objects/:object_id/challenge",
            post(challenge_object),
        )
}

/// Upload an object: encrypt, erasure-code, and place shards on online nodes.
pub async fn upload_object(
    State(state): State<AppState>,
    Json(req): Json<UploadObjectRequest>,
) -> Result<(StatusCode, Json<StorageObjectView>), ApiError> {
    let data = base64::engine::general_purpose::STANDARD
        .decode(req.data_base64.as_bytes())
        .map_err(|e| ApiError::bad_request(format!("invalid base64: {e}")))?;

    let k = req
        .data_shards
        .unwrap_or(state.config.storage_data_shards)
        .max(1) as usize;
    let m = req
        .parity_shards
        .unwrap_or(state.config.storage_parity_shards)
        .max(1) as usize;
    let n = k + m;

    let encrypted =
        mesh_storage::crypto::encrypt(&state.storage_key, &data).map_err(ApiError::internal)?;
    let shards =
        mesh_storage::erasure::encode(&encrypted.ciphertext, k, m).map_err(ApiError::internal)?;

    let candidates: Vec<String> = sqlx::query_scalar(
        "SELECT n.node_id FROM nodes n \
         WHERE n.status = 'online' \
           AND EXISTS (SELECT 1 FROM resource_offers o \
                       WHERE o.node_id = n.node_id AND o.revoked_at IS NULL AND o.expires_at > now()) \
         ORDER BY n.node_id",
    )
    .fetch_all(&state.pool)
    .await?;
    let candidate_ids: Vec<NodeId> = candidates.into_iter().map(NodeId::new).collect();
    let placements = mesh_storage::plan(&candidate_ids, n)
        .map_err(|e| ApiError::new(StatusCode::CONFLICT, "insufficient_nodes", e.to_string()))?;

    let object_id = format!("obj-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO storage_objects \
             (object_id, owner_participant, size_bytes, encrypted_bytes, data_shards, \
              parity_shards, nonce, status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'healthy')",
    )
    .bind(&object_id)
    .bind(req.owner_participant.as_str())
    .bind(data.len() as i64)
    .bind(encrypted.ciphertext.len() as i64)
    .bind(k as i32)
    .bind(m as i32)
    .bind(hex::encode(encrypted.nonce))
    .execute(&state.pool)
    .await?;

    let mut views = Vec::with_capacity(n);
    for (index, shard) in shards.iter().enumerate() {
        let node = &placements[index];
        shard_store::write_shard(&state.storage_dir, &object_id, index as i32, shard)
            .map_err(ApiError::internal)?;
        let shard_id = format!("{object_id}-{index}");
        let checksum = mesh_storage::erasure::shard_checksum(shard);
        sqlx::query(
            "INSERT INTO storage_shards \
                 (shard_id, object_id, shard_index, node_id, checksum, status) \
             VALUES ($1, $2, $3, $4, $5, 'present')",
        )
        .bind(&shard_id)
        .bind(&object_id)
        .bind(index as i32)
        .bind(node.as_str())
        .bind(&checksum)
        .execute(&state.pool)
        .await?;
        views.push(StorageShardView {
            shard_id,
            shard_index: index as i32,
            node_id: node.clone(),
            checksum,
            status: "present".to_string(),
            last_verified: None,
        });
    }

    tracing::info!(object_id = %object_id, shards = n, "storage object uploaded");

    Ok((
        StatusCode::CREATED,
        Json(StorageObjectView {
            object_id,
            owner_participant: req.owner_participant,
            size_bytes: data.len() as i64,
            encrypted_bytes: encrypted.ciphertext.len() as i64,
            data_shards: k as i32,
            parity_shards: m as i32,
            status: "healthy".to_string(),
            created_at: Utc::now(),
            shards: views,
        }),
    ))
}

struct ObjectMeta {
    owner_participant: String,
    size_bytes: i64,
    encrypted_bytes: i64,
    data_shards: i32,
    parity_shards: i32,
    nonce: String,
    status: String,
    created_at: chrono::DateTime<Utc>,
}

async fn load_meta(state: &AppState, object_id: &str) -> Result<ObjectMeta, ApiError> {
    let row = sqlx::query(
        "SELECT owner_participant, size_bytes, encrypted_bytes, data_shards, parity_shards, \
             nonce, status, created_at FROM storage_objects WHERE object_id = $1",
    )
    .bind(object_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("storage object not found"))?;
    Ok(ObjectMeta {
        owner_participant: row.try_get("owner_participant")?,
        size_bytes: row.try_get("size_bytes")?,
        encrypted_bytes: row.try_get("encrypted_bytes")?,
        data_shards: row.try_get("data_shards")?,
        parity_shards: row.try_get("parity_shards")?,
        nonce: row.try_get("nonce")?,
        status: row.try_get("status")?,
        created_at: row.try_get("created_at")?,
    })
}

async fn load_shards(state: &AppState, object_id: &str) -> Result<Vec<StorageShardView>, ApiError> {
    let rows = sqlx::query(
        "SELECT shard_id, shard_index, node_id, checksum, status, last_verified \
         FROM storage_shards WHERE object_id = $1 ORDER BY shard_index",
    )
    .bind(object_id)
    .fetch_all(&state.pool)
    .await?;
    let mut shards = Vec::with_capacity(rows.len());
    for row in rows {
        shards.push(StorageShardView {
            shard_id: row.try_get("shard_id")?,
            shard_index: row.try_get("shard_index")?,
            node_id: NodeId::new(row.try_get::<String, _>("node_id")?),
            checksum: row.try_get("checksum")?,
            status: row.try_get("status")?,
            last_verified: row.try_get("last_verified")?,
        });
    }
    Ok(shards)
}

fn to_view(
    object_id: String,
    meta: &ObjectMeta,
    shards: Vec<StorageShardView>,
) -> StorageObjectView {
    StorageObjectView {
        object_id,
        owner_participant: ParticipantId::new(meta.owner_participant.clone()),
        size_bytes: meta.size_bytes,
        encrypted_bytes: meta.encrypted_bytes,
        data_shards: meta.data_shards,
        parity_shards: meta.parity_shards,
        status: meta.status.clone(),
        created_at: meta.created_at,
        shards,
    }
}

/// Fetch an object and its shards.
pub async fn get_object(
    State(state): State<AppState>,
    Path(object_id): Path<String>,
) -> Result<Json<StorageObjectView>, ApiError> {
    let meta = load_meta(&state, &object_id).await?;
    let shards = load_shards(&state, &object_id).await?;
    Ok(Json(to_view(object_id, &meta, shards)))
}

/// List stored objects.
pub async fn list_objects(
    State(state): State<AppState>,
) -> Result<Json<Vec<StorageObjectView>>, ApiError> {
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT object_id FROM storage_objects ORDER BY created_at DESC")
            .fetch_all(&state.pool)
            .await?;
    let mut objects = Vec::with_capacity(ids.len());
    for id in ids {
        let meta = load_meta(&state, &id).await?;
        let shards = load_shards(&state, &id).await?;
        objects.push(to_view(id, &meta, shards));
    }
    Ok(Json(objects))
}

/// Recover and return the object bytes.
pub async fn download_object(
    State(state): State<AppState>,
    Path(object_id): Path<String>,
) -> Result<Json<DownloadObjectResponse>, ApiError> {
    let meta = load_meta(&state, &object_id).await?;
    let k = meta.data_shards as usize;
    let m = meta.parity_shards as usize;

    let mut shards: Vec<Option<Vec<u8>>> = Vec::with_capacity(k + m);
    let mut present = 0u32;
    for index in 0..(k + m) {
        if shard_store::shard_exists(&state.storage_dir, &object_id, index as i32) {
            let bytes = shard_store::read_shard(&state.storage_dir, &object_id, index as i32)
                .map_err(ApiError::internal)?;
            present += 1;
            shards.push(Some(bytes));
        } else {
            shards.push(None);
        }
    }

    let ciphertext =
        mesh_storage::erasure::reconstruct(&shards, k, m, meta.encrypted_bytes as usize).map_err(
            |_| {
                ApiError::new(
                    StatusCode::CONFLICT,
                    "insufficient_shards",
                    "not enough shards to reconstruct the object",
                )
            },
        )?;
    let nonce = decode_nonce(&meta.nonce)?;
    let data = mesh_storage::crypto::decrypt(&state.storage_key, &nonce, &ciphertext)
        .map_err(ApiError::internal)?;

    Ok(Json(DownloadObjectResponse {
        object_id,
        data_base64: base64::engine::general_purpose::STANDARD.encode(data),
        shards_used: present,
    }))
}

/// Repair missing shards by reconstructing and re-placing them.
pub async fn repair_object(
    State(state): State<AppState>,
    Path(object_id): Path<String>,
) -> Result<Json<RepairObjectResponse>, ApiError> {
    let meta = load_meta(&state, &object_id).await?;
    let k = meta.data_shards as usize;
    let m = meta.parity_shards as usize;

    let shards = load_shards(&state, &object_id).await?;
    let mut missing: Vec<i32> = Vec::new();
    let mut present: Vec<Option<Vec<u8>>> = Vec::with_capacity(k + m);
    for index in 0..(k + m) {
        if shard_store::shard_exists(&state.storage_dir, &object_id, index as i32) {
            present.push(Some(
                shard_store::read_shard(&state.storage_dir, &object_id, index as i32)
                    .map_err(ApiError::internal)?,
            ));
        } else {
            present.push(None);
            missing.push(index as i32);
        }
    }

    if missing.is_empty() {
        return Ok(Json(RepairObjectResponse {
            object_id,
            repaired: 0,
            status: "healthy".to_string(),
        }));
    }

    let ciphertext =
        match mesh_storage::erasure::reconstruct(&present, k, m, meta.encrypted_bytes as usize) {
            Ok(ciphertext) => ciphertext,
            Err(_) => {
                sqlx::query("UPDATE storage_objects SET status = 'lost' WHERE object_id = $1")
                    .bind(&object_id)
                    .execute(&state.pool)
                    .await?;
                return Ok(Json(RepairObjectResponse {
                    object_id,
                    repaired: 0,
                    status: "lost".to_string(),
                }));
            }
        };

    // Re-encode to regenerate the missing shards.
    let all = mesh_storage::erasure::encode(&ciphertext, k, m).map_err(ApiError::internal)?;

    // Candidate nodes not already holding a shard of this object.
    let holders: Vec<String> = shards.iter().map(|s| s.node_id.to_string()).collect();
    let candidates: Vec<String> = sqlx::query_scalar(
        "SELECT n.node_id FROM nodes n \
         WHERE n.status = 'online' \
           AND EXISTS (SELECT 1 FROM resource_offers o \
                       WHERE o.node_id = n.node_id AND o.revoked_at IS NULL AND o.expires_at > now()) \
         ORDER BY n.node_id",
    )
    .fetch_all(&state.pool)
    .await?;
    let available: Vec<String> = candidates
        .into_iter()
        .filter(|node| !holders.contains(node))
        .collect();

    let mut repaired = 0u32;
    for (slot, index) in missing.iter().enumerate() {
        let node = available
            .get(slot)
            .cloned()
            .unwrap_or_else(|| holders.first().cloned().unwrap_or_default());
        let shard = &all[*index as usize];
        shard_store::write_shard(&state.storage_dir, &object_id, *index, shard)
            .map_err(ApiError::internal)?;
        let checksum = mesh_storage::erasure::shard_checksum(shard);
        sqlx::query(
            "UPDATE storage_shards SET node_id = $2, checksum = $3, status = 'present' \
             WHERE object_id = $1 AND shard_index = $4",
        )
        .bind(&object_id)
        .bind(&node)
        .bind(&checksum)
        .bind(*index)
        .execute(&state.pool)
        .await?;
        repaired += 1;
    }

    sqlx::query("UPDATE storage_objects SET status = 'healthy' WHERE object_id = $1")
        .bind(&object_id)
        .execute(&state.pool)
        .await?;

    tracing::info!(object_id = %object_id, repaired, "storage object repaired");
    Ok(Json(RepairObjectResponse {
        object_id,
        repaired,
        status: "healthy".to_string(),
    }))
}

/// Challenge every present shard and reward verified availability.
pub async fn challenge_object(
    State(state): State<AppState>,
    Path(object_id): Path<String>,
) -> Result<Json<ChallengeObjectResponse>, ApiError> {
    Ok(Json(challenge_object_impl(&state, &object_id).await?))
}

/// Core challenge logic, callable from the background sweeper.
pub async fn challenge_object_impl(
    state: &AppState,
    object_id: &str,
) -> Result<ChallengeObjectResponse, ApiError> {
    let shards = load_shards(state, object_id).await?;
    let mut verified = 0u32;
    let mut failed = 0u32;
    let mut rewarded = 0u32;

    let policy = mesh_accounting::RewardPolicy {
        version: state.config.reward_policy_version,
        rate_per_unit: state.config.reward_rate_per_unit,
        delegator_rate_per_unit: state.config.delegator_reward_rate_per_unit,
    };

    for shard in &shards {
        let nonce = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
        let challenge = mesh_storage::challenge::challenge_for(&shard.shard_id, nonce);
        let challenge_id = format!("chl-{}", uuid::Uuid::new_v4());

        let bytes = if shard_store::shard_exists(&state.storage_dir, object_id, shard.shard_index) {
            shard_store::read_shard(&state.storage_dir, object_id, shard.shard_index)
                .map_err(ApiError::internal)?
        } else {
            Vec::new()
        };
        let response = mesh_storage::challenge::respond(&challenge, &bytes);
        let ok =
            !bytes.is_empty() && mesh_storage::challenge::verify(&challenge, &bytes, &response);

        let outcome = if ok { "verified" } else { "failed" };
        sqlx::query(
            "INSERT INTO storage_challenges (challenge_id, shard_id, nonce, outcome) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&challenge_id)
        .bind(&shard.shard_id)
        .bind(nonce as i64)
        .bind(outcome)
        .execute(&state.pool)
        .await?;

        if ok {
            verified += 1;
            sqlx::query(
                "UPDATE storage_shards SET last_verified = now(), status = 'present' \
                 WHERE shard_id = $1",
            )
            .bind(&shard.shard_id)
            .execute(&state.pool)
            .await?;

            // Reward verified availability, not allocated space.
            if let Some(participant) = node_participant(state, &shard.node_id).await? {
                let input = mesh_accounting::ContributionInput {
                    job_id: None,
                    root_job_id: None,
                    attempt_id: AttemptId::new(challenge_id.clone()),
                    participant_id: participant,
                    node_id: shard.node_id.clone(),
                    agent_id: None,
                    role: "storage".to_string(),
                    workload_type: "storage".to_string(),
                    accepted_units: 1,
                    verification_method: "retrieval-challenge".to_string(),
                    evidence_hash: hex::encode(response),
                    policy_version: policy.version,
                };
                if let Ok(Some(_)) =
                    mesh_accounting::record_contribution(&state.pool, &policy, &input).await
                {
                    rewarded += 1;
                }
            }
        } else {
            failed += 1;
            sqlx::query("UPDATE storage_shards SET status = 'missing' WHERE shard_id = $1")
                .bind(&shard.shard_id)
                .execute(&state.pool)
                .await?;
        }
    }

    Ok(ChallengeObjectResponse {
        object_id: object_id.to_string(),
        verified,
        failed,
        rewarded,
    })
}

async fn node_participant(
    state: &AppState,
    node_id: &NodeId,
) -> Result<Option<ParticipantId>, ApiError> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT participant_id FROM nodes WHERE node_id = $1")
            .bind(node_id.as_str())
            .fetch_optional(&state.pool)
            .await?;
    Ok(value.map(ParticipantId::new))
}

fn decode_nonce(value: &str) -> Result<[u8; 12], ApiError> {
    let bytes = hex::decode(value).map_err(ApiError::internal)?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| ApiError::internal("invalid nonce"))
}
