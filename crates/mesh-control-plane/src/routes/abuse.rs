//! Abuse reporting and quarantine.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use sqlx::Row;

use mesh_protocol::{
    AbuseReportRequest, AbuseReportResponse, AbuseReportView, ParticipantId,
};

use crate::error::ApiError;
use crate::AppState;

/// Abuse routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/abuse/reports", get(list_reports).post(report_abuse))
}

/// Record an abuse report; quarantine the subject past the configured threshold.
pub async fn report_abuse(
    State(state): State<AppState>,
    Json(req): Json<AbuseReportRequest>,
) -> Result<(StatusCode, Json<AbuseReportResponse>), ApiError> {
    if req.subject_type != "node" && req.subject_type != "agent" {
        return Err(ApiError::bad_request("subject_type must be `node` or `agent`"));
    }
    if req.subject_id.trim().is_empty() || req.category.trim().is_empty() {
        return Err(ApiError::bad_request("subject_id and category are required"));
    }

    let report_id = format!("abr-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO abuse_reports \
             (report_id, reporter_participant, subject_type, subject_id, category, detail) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&report_id)
    .bind(req.reporter_participant.as_ref().map(ParticipantId::as_str))
    .bind(&req.subject_type)
    .bind(&req.subject_id)
    .bind(&req.category)
    .bind(&req.detail)
    .execute(&state.pool)
    .await?;

    let report_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM abuse_reports WHERE subject_type = $1 AND subject_id = $2",
    )
    .bind(&req.subject_type)
    .bind(&req.subject_id)
    .fetch_one(&state.pool)
    .await?;

    let mut quarantined = false;
    if report_count >= state.config.abuse_suspend_threshold {
        quarantined = quarantine(&state, &req.subject_type, &req.subject_id).await?;
        if quarantined {
            tracing::warn!(
                subject_type = %req.subject_type,
                subject_id = %req.subject_id,
                report_count,
                "subject quarantined after abuse reports"
            );
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(AbuseReportResponse {
            report_id,
            subject_type: req.subject_type,
            subject_id: req.subject_id,
            report_count,
            quarantined,
        }),
    ))
}

/// List recent abuse reports.
pub async fn list_reports(State(state): State<AppState>) -> Result<Json<Vec<AbuseReportView>>, ApiError> {
    let rows = sqlx::query(
        "SELECT report_id, reporter_participant, subject_type, subject_id, category, detail, \
             created_at FROM abuse_reports ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut reports = Vec::with_capacity(rows.len());
    for row in rows {
        reports.push(AbuseReportView {
            report_id: row.try_get("report_id")?,
            reporter_participant: row
                .try_get::<Option<String>, _>("reporter_participant")?
                .map(ParticipantId::new),
            subject_type: row.try_get("subject_type")?,
            subject_id: row.try_get("subject_id")?,
            category: row.try_get("category")?,
            detail: row.try_get("detail")?,
            created_at: row.try_get("created_at")?,
        });
    }
    Ok(Json(reports))
}

async fn quarantine(
    state: &AppState,
    subject_type: &str,
    subject_id: &str,
) -> Result<bool, ApiError> {
    match subject_type {
        "node" => {
            let affected = sqlx::query(
                "UPDATE nodes SET status = 'decommissioned', updated_at = now() \
                 WHERE node_id = $1 AND status <> 'decommissioned'",
            )
            .bind(subject_id)
            .execute(&state.pool)
            .await?
            .rows_affected();
            // Stop scheduling anything on the node.
            sqlx::query("UPDATE resource_offers SET revoked_at = now() WHERE node_id = $1")
                .bind(subject_id)
                .execute(&state.pool)
                .await?;
            sqlx::query("UPDATE agents SET status = 'offline' WHERE node_id = $1")
                .bind(subject_id)
                .execute(&state.pool)
                .await?;
            Ok(affected > 0)
        }
        "agent" => {
            let affected = sqlx::query(
                "UPDATE agents SET status = 'offline', updated_at = now() \
                 WHERE agent_id = $1 AND status <> 'offline'",
            )
            .bind(subject_id)
            .execute(&state.pool)
            .await?
            .rows_affected();
            Ok(affected > 0)
        }
        _ => Ok(false),
    }
}
