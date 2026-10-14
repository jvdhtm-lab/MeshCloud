//! Control-plane configuration loaded from defaults, a TOML file, and env vars.
//!
//! Precedence (lowest to highest): built-in defaults -> TOML file -> `MESH_`
//! environment variables. Nested keys use a double underscore, for example
//! `MESH_LOG__LEVEL=debug`.

use std::net::SocketAddr;
use std::path::Path;

use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};

/// Log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human-readable, colored output for local development.
    Pretty,
    /// Structured JSON, for production log aggregation.
    Json,
}

/// Logging configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// `tracing` env-filter directive, e.g. `info` or `mesh_control_plane=debug`.
    pub level: String,
    /// Output format.
    pub format: LogFormat,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: LogFormat::Pretty,
        }
    }
}

/// Control-plane configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Address the HTTP API binds to.
    pub bind_addr: SocketAddr,
    /// PostgreSQL connection string.
    pub database_url: String,
    /// Maximum pooled database connections.
    pub database_max_connections: u32,
    /// Heartbeat period advertised to workers, in seconds.
    pub heartbeat_interval_seconds: u64,
    /// Mark a node offline when it has not heartbeated for this many seconds.
    pub node_offline_after_seconds: u64,
    /// Lease granted to a worker when it claims a job, in seconds.
    pub job_lease_seconds: u64,
    /// How often the job sweeper runs, in seconds.
    pub job_sweep_interval_seconds: u64,
    /// Memory ceiling for verification re-execution, in megabytes.
    pub verification_memory_mb: u64,
    /// Fuel budget for verification re-execution.
    pub verification_fuel: u64,
    /// Wall-clock timeout for verification re-execution, in seconds.
    pub verification_timeout_seconds: u64,
    /// Version of the reward policy in force.
    pub reward_policy_version: i32,
    /// Credits awarded per accepted usage unit.
    pub reward_rate_per_unit: i64,
    /// Credits awarded per accepted usage unit to a delegating agent.
    pub delegator_reward_rate_per_unit: i64,
    /// Settlement backend: `local` or `evm`.
    pub settlement_backend: String,
    /// Chain id recorded on settlement batches.
    pub settlement_chain_id: i64,
    /// Maximum receipts per settlement batch.
    pub settlement_batch_size: u32,
    /// How often the settlement worker runs, in seconds.
    pub settlement_interval_seconds: u64,
    /// EVM JSON-RPC endpoint (for the `evm` backend).
    pub settlement_rpc_url: Option<String>,
    /// Deployed reward contract address (for the `evm` backend).
    pub settlement_contract_address: Option<String>,
    /// Issuer private key (hex) for the `evm` backend.
    pub settlement_issuer_key: Option<String>,
    /// Per-agent message rate limit (messages per second).
    pub agent_message_rate_per_second: f64,
    /// Per-agent message burst capacity.
    pub agent_message_burst: f64,
    /// Per-agent byte rate limit (bytes per second).
    pub agent_message_bytes_per_second: f64,
    /// Per-agent byte burst capacity.
    pub agent_message_bytes_burst: f64,
    /// Maximum credits a participant may reserve at once (0 = unlimited).
    pub participant_quota_credits: i64,
    /// Control-plane replica role (`primary` or `replica`).
    pub replica_role: String,
    /// libp2p listen multiaddress for the mesh (disabled when unset).
    pub network_listen: Option<String>,
    /// Gossip topic for node advertisements.
    pub network_topic: String,
    /// Directory for stored shards and the master key.
    pub storage_dir: String,
    /// Default number of data shards.
    pub storage_data_shards: u32,
    /// Default number of parity shards.
    pub storage_parity_shards: u32,
    /// Master encryption key (hex); generated and persisted when unset.
    pub storage_key: Option<String>,
    /// How often the storage availability sweeper runs, in seconds.
    pub storage_challenge_interval_seconds: u64,
    /// Number of abuse reports against a subject that triggers quarantine.
    pub abuse_suspend_threshold: i64,
    /// Logging configuration.
    pub log: LogConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8080".parse().expect("valid default bind address"),
            database_url: "postgres://mesh:mesh_dev_password@localhost:5432/meshcloud".to_string(),
            database_max_connections: 10,
            heartbeat_interval_seconds: 30,
            node_offline_after_seconds: 90,
            job_lease_seconds: 60,
            job_sweep_interval_seconds: 10,
            verification_memory_mb: 256,
            verification_fuel: 1_000_000_000,
            verification_timeout_seconds: 10,
            reward_policy_version: 1,
            reward_rate_per_unit: 1,
            delegator_reward_rate_per_unit: 1,
            settlement_backend: "local".to_string(),
            settlement_chain_id: 31337,
            settlement_batch_size: 100,
            settlement_interval_seconds: 10,
            settlement_rpc_url: None,
            settlement_contract_address: None,
            settlement_issuer_key: None,
            agent_message_rate_per_second: 10.0,
            agent_message_burst: 20.0,
            agent_message_bytes_per_second: 100_000.0,
            agent_message_bytes_burst: 200_000.0,
            participant_quota_credits: 0,
            replica_role: "primary".to_string(),
            network_listen: None,
            network_topic: "meshcloud/nodes".to_string(),
            storage_dir: "storage".to_string(),
            storage_data_shards: 4,
            storage_parity_shards: 2,
            storage_key: None,
            storage_challenge_interval_seconds: 60,
            abuse_suspend_threshold: 3,
            log: LogConfig::default(),
        }
    }
}

impl Config {
    /// Load configuration from `MESH_CONFIG` (or the default path) and env vars.
    pub fn load() -> anyhow::Result<Self> {
        let path = std::env::var("MESH_CONFIG")
            .unwrap_or_else(|_| "configs/mesh-control-plane.toml".to_string());

        let mut figment = Figment::from(Serialized::defaults(Config::default()));
        if Path::new(&path).exists() {
            figment = figment.merge(Toml::file(&path));
        }
        figment = figment.merge(
            Env::prefixed("MESH_")
                .split("__")
                .map(|key| key.as_str().to_lowercase().into()),
        );
        Ok(figment.extract()?)
    }
}
