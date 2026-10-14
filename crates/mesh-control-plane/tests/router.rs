//! Router-level tests that do not require a database.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mesh_control_plane::{build_router, config::Config, db, AppState};
use mesh_protocol::HealthResponse;
use tower::ServiceExt;

fn test_state() -> AppState {
    // Lazy pool: no connection is attempted unless a handler touches the DB.
    let pool = db::lazy_pool("postgres://mesh:mesh@localhost:5432/meshcloud_test");
    AppState::new(pool, Config::default())
}

#[tokio::test]
async fn health_endpoint_reports_ok_without_database() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let health: HealthResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(health.status, "ok");
    assert_eq!(health.protocol_version, mesh_protocol::PROTOCOL_VERSION);
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
