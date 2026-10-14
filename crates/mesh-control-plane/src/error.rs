//! Shared API error type.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use mesh_protocol::ErrorResponse;

/// An error that renders as a JSON [`ErrorResponse`] with a status code.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    /// Construct an error with an explicit status and code.
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    /// A 400 for malformed client input.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    /// A 404 for missing resources.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    /// A 500 that logs the underlying cause without leaking it to clients.
    pub fn internal(cause: impl std::fmt::Display) -> Self {
        tracing::error!(error = %cause, "internal server error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal server error",
        )
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        ApiError::internal(err)
    }
}

impl From<mesh_scheduler::SchedulerError> for ApiError {
    fn from(err: mesh_scheduler::SchedulerError) -> Self {
        use mesh_scheduler::SchedulerError;
        let message = err.to_string();
        match err {
            SchedulerError::NotFound(_) => ApiError::not_found(message),
            SchedulerError::AttemptNotActive
            | SchedulerError::IllegalTransition(_)
            | SchedulerError::Delegation(_) => {
                ApiError::new(StatusCode::CONFLICT, "conflict", message)
            }
            SchedulerError::InvalidState(_) | SchedulerError::Database(_) => {
                ApiError::internal(message)
            }
        }
    }
}

impl From<mesh_accounting::AccountingError> for ApiError {
    fn from(err: mesh_accounting::AccountingError) -> Self {
        ApiError::internal(err)
    }
}

impl From<mesh_settlement::SettlementError> for ApiError {
    fn from(err: mesh_settlement::SettlementError) -> Self {
        ApiError::internal(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            code: self.code.to_string(),
            message: self.message,
        });
        (self.status, body).into_response()
    }
}
