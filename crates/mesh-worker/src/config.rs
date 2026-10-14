//! Worker configuration.

use std::path::PathBuf;

use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use mesh_protocol::offers::Availability;
use serde::{Deserialize, Serialize};

/// Worker configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    /// Base URL of the control plane.
    pub control_plane_url: String,
    /// Path to the persisted node identity (private key).
    pub identity_path: PathBuf,
    /// Number of AI agents to host (identities stored beside the node key).
    pub agent_count: u32,
    /// Skills advertised by hosted agents.
    #[serde(default = "default_agent_skills")]
    pub agent_skills: Vec<String>,
    /// libp2p listen multiaddress for the mesh (disabled when unset).
    pub network_listen: Option<String>,
    /// Bootstrap peer multiaddress (`/ip4/…/p2p/<peer-id>`).
    pub network_bootstrap: Option<String>,
    /// Gossip topic for node advertisements.
    pub network_topic: String,
    /// Advertisement lifetime in seconds.
    pub network_ttl_seconds: u64,
    /// Per-agent memory cap for WASM execution, in megabytes.
    pub agent_max_memory_mb: u64,
    /// Per-agent fuel budget for WASM execution.
    pub agent_fuel_limit: u64,
    /// Per-agent wall-clock timeout, in seconds.
    pub agent_execution_timeout_seconds: u64,
    /// Owning participant. Derived from the node public key when unset.
    pub participant_id: Option<String>,
    /// How often to heartbeat the control plane, in seconds.
    pub heartbeat_interval_seconds: u64,
    /// Lifetime of each signed resource offer, in seconds.
    pub offer_ttl_seconds: u64,
    /// Override detected CPU core count.
    pub cpu_cores: Option<u32>,
    /// Override detected RAM, in gigabytes.
    pub ram_gb: Option<u32>,
    /// Storage offered, in gigabytes.
    pub storage_gb: u32,
    /// Whether a usable GPU is offered.
    pub gpu: bool,
    /// Cap on CPU share per workload, as a percentage.
    pub max_cpu_percent: u32,
    /// Cap on memory per workload, in megabytes.
    pub max_memory_mb: u32,
    /// When resources are available for sharing.
    pub availability: Availability,
    /// Workload types this worker can execute.
    #[serde(default = "default_workloads")]
    pub supported_workloads: Vec<String>,
    /// How often to poll for new work, in seconds.
    pub poll_interval_seconds: u64,
    /// Fuel budget for a single WASM job.
    pub fuel_limit: u64,
    /// Wall-clock timeout for a single WASM job, in seconds.
    pub execution_timeout_seconds: u64,
    /// Run WASM jobs in a separate sandbox process (defense in depth).
    pub runtime_isolation: bool,
    /// Test hook: exit the process after this many heartbeats to simulate a crash.
    pub simulate_crash_after_heartbeats: Option<u64>,
    /// `tracing` env-filter directive.
    pub log_level: String,
    /// Whether to emit structured JSON logs.
    pub log_json: bool,
}

fn default_workloads() -> Vec<String> {
    vec![
        "echo".to_string(),
        "wasm".to_string(),
        "delegate".to_string(),
    ]
}

fn default_agent_skills() -> Vec<String> {
    vec!["deterministic".to_string(), "planning".to_string()]
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            control_plane_url: "http://localhost:8080".to_string(),
            identity_path: PathBuf::from("node.key"),
            agent_count: 0,
            agent_skills: default_agent_skills(),
            network_listen: None,
            network_bootstrap: None,
            network_topic: "meshcloud/nodes".to_string(),
            network_ttl_seconds: 120,
            agent_max_memory_mb: 256,
            agent_fuel_limit: 500_000_000,
            agent_execution_timeout_seconds: 5,
            participant_id: None,
            heartbeat_interval_seconds: 15,
            offer_ttl_seconds: 90,
            cpu_cores: None,
            ram_gb: None,
            storage_gb: 50,
            gpu: false,
            max_cpu_percent: 50,
            max_memory_mb: 2048,
            availability: Availability::WhenIdle,
            supported_workloads: default_workloads(),
            poll_interval_seconds: 5,
            fuel_limit: 1_000_000_000,
            execution_timeout_seconds: 5,
            runtime_isolation: true,
            simulate_crash_after_heartbeats: None,
            log_level: "info".to_string(),
            log_json: false,
        }
    }
}

impl WorkerConfig {
    /// Load from `MESH_CONFIG` (optional), a default file, then env vars.
    pub fn load() -> anyhow::Result<Self> {
        let path =
            std::env::var("MESH_CONFIG").unwrap_or_else(|_| "configs/mesh-worker.toml".to_string());

        let mut figment = Figment::from(Serialized::defaults(WorkerConfig::default()));
        if std::path::Path::new(&path).exists() {
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
