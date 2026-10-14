//! MeshCloud control plane: HTTP API, node registry, and coordination.
//!
//! The library target exists so integration tests can build the router without
//! starting a real socket.

#![forbid(unsafe_code)]

pub mod config;
pub mod db;
pub mod error;
pub mod routes;
pub mod storage;
pub mod telemetry;

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use sqlx::PgPool;
use tower_http::trace::TraceLayer;

pub use config::Config;

/// Shared application state handed to every request handler.
#[derive(Clone)]
pub struct AppState {
    /// PostgreSQL connection pool.
    pub pool: PgPool,
    /// Parsed configuration.
    pub config: Arc<Config>,
    /// Per-agent message meter.
    pub meter: Arc<mesh_agent_runtime::AgentMeter>,
    /// Discovered mesh peers.
    pub registry: Arc<mesh_network::DiscoveryRegistry>,
    /// Optional libp2p mesh transport (requires the `libp2p` feature).
    #[cfg(feature = "libp2p")]
    pub network: Option<Arc<mesh_network::Libp2pTransport>>,
    /// Master key for storage encryption.
    pub storage_key: [u8; 32],
    /// Directory holding stored shards.
    pub storage_dir: std::path::PathBuf,
    /// Process start time, used to report uptime.
    pub started_at: Instant,
}

impl AppState {
    /// Build state from a pool and configuration.
    pub fn new(pool: PgPool, config: Config) -> Self {
        let meter = mesh_agent_runtime::AgentMeter::new(
            config.agent_message_rate_per_second,
            config.agent_message_burst,
            config.agent_message_bytes_per_second,
            config.agent_message_bytes_burst,
        );
        let storage_dir = std::path::PathBuf::from(&config.storage_dir);
        let storage_key = storage::load_or_create_key(&storage_dir, config.storage_key.as_deref())
            .unwrap_or_else(|err| {
                tracing::warn!(error = %err, "storage key init failed; using an ephemeral key");
                mesh_storage::crypto::random_key()
            });
        Self {
            pool,
            config: Arc::new(config),
            meter: Arc::new(meter),
            registry: Arc::new(mesh_network::DiscoveryRegistry::new()),
            #[cfg(feature = "libp2p")]
            network: None,
            storage_key,
            storage_dir,
            started_at: Instant::now(),
        }
    }

    /// Attach a running libp2p transport.
    #[cfg(feature = "libp2p")]
    pub fn with_network(mut self, network: Arc<mesh_network::Libp2pTransport>) -> Self {
        self.network = Some(network);
        self
    }
}

/// Build the full application router.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", axum::routing::get(routes::ui::index))
        .route("/ui", axum::routing::get(routes::ui::index))
        .merge(routes::health::router())
        .merge(routes::nodes::router())
        .merge(routes::agents::router())
        .merge(routes::jobs::router())
        .merge(routes::workers::router())
        .merge(routes::contributions::router())
        .merge(routes::settlements::router())
        .merge(routes::participants::router())
        .merge(routes::marketplace::router())
        .merge(routes::network::router())
        .merge(routes::storage::router())
        .merge(routes::abuse::router())
        .merge(routes::metrics::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
