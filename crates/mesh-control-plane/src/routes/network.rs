//! Mesh networking views: replica role, peer id, and discovered peers.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use mesh_network::NodeAdvertisement;

use crate::AppState;

/// Network routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/network/status", get(status))
        .route("/api/v1/network/peers", get(peers))
}

#[derive(Debug, Serialize)]
pub struct NetworkStatus {
    role: String,
    writers_enabled: bool,
    peer_id: Option<String>,
    peer_count: usize,
    topic: String,
}

/// Report the replica role and mesh status.
pub async fn status(State(state): State<AppState>) -> Json<NetworkStatus> {
    let role = state.config.replica_role.clone();
    let writers_enabled = role.eq_ignore_ascii_case("primary");
    #[cfg(feature = "libp2p")]
    let peer_id = state
        .network
        .as_ref()
        .map(|network| network.peer_id().to_base58());
    #[cfg(not(feature = "libp2p"))]
    let peer_id: Option<String> = None;
    Json(NetworkStatus {
        role,
        writers_enabled,
        peer_id,
        peer_count: state.registry.len(),
        topic: state.config.network_topic.clone(),
    })
}

/// List discovered peers from the mesh.
pub async fn peers(State(state): State<AppState>) -> Json<Vec<NodeAdvertisement>> {
    Json(state.registry.peers())
}
