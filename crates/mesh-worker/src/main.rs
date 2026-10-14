//! MeshCloud worker agent.
//!
//! On start the worker loads a persistent Ed25519 identity, detects its
//! resources, registers a signed resource offer with the control plane, and
//! then runs two loops: heartbeating (with fresh signed offers) and polling for
//! jobs to execute. Phase 2 ships a deterministic `echo` workload; sandboxed
//! WASM execution arrives in Phase 3.

#![forbid(unsafe_code)]

mod agent_host;
mod capabilities;
mod config;
mod control_plane;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use chrono::{Duration as ChronoDuration, Utc};
use mesh_agent_protocol::AgentHeartbeatRequest;
use mesh_identity::NodeIdentity;
use mesh_network::MeshTransport;
use mesh_protocol::offers::{ResourceOffer, SignedResourceOffer};
use mesh_protocol::{
    AgentId, ClaimedJobResponse, CompleteJobRequest, DelegateJobRequest, FailJobRequest,
    HeartbeatRequest, NodeCapabilities, ParticipantId, RegisterNodeRequest, ResourceLimits,
    WasmJobInput,
};
use mesh_runtime::RuntimeLimits;
use sha2::{Digest, Sha256};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::agent_host::AgentHost;
use crate::config::WorkerConfig;
use crate::control_plane::ControlPlaneClient;

fn main() -> anyhow::Result<()> {
    // Defense in depth: when re-executed with the sandbox flag, run one module
    // from stdin and exit, without starting the async runtime.
    if std::env::args().nth(1).as_deref() == Some(mesh_runtime::SANDBOX_ARG) {
        mesh_runtime::sandbox_main().map_err(|err| anyhow!("sandbox error: {err}"))?;
        return Ok(());
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?
        .block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    let config = WorkerConfig::load().context("failed to load worker configuration")?;
    init_telemetry(&config);

    let identity = NodeIdentity::load_or_generate(&config.identity_path)
        .context("failed to load or generate node identity")?;
    let participant_id = ParticipantId::new(
        config
            .participant_id
            .clone()
            .unwrap_or_else(|| derive_participant_id(&identity)),
    );

    let capabilities = capabilities::detect_capabilities(&config);
    let limits = capabilities::limits(&config);

    info!(
        node_id = %identity.node_id(),
        participant_id = %participant_id,
        public_key = %identity.public_key_hex(),
        cpu_cores = capabilities.cpu_cores,
        ram_gb = capabilities.ram_gb,
        storage_gb = capabilities.storage_gb,
        gpu = capabilities.gpu,
        workloads = ?config.supported_workloads,
        control_plane = %config.control_plane_url,
        "mesh-worker starting"
    );

    let client = ControlPlaneClient::new(config.control_plane_url.clone())?;

    let register_request = RegisterNodeRequest {
        node_id: identity.node_id().clone(),
        participant_id,
        public_key: identity.public_key_hex(),
        offer: build_offer(&identity, &capabilities, &limits, &config),
    };

    let registered = register_with_retry(&client, &register_request).await?;
    info!(
        status = %registered.status,
        heartbeat_interval_seconds = registered.heartbeat_interval_seconds,
        "registered with control plane"
    );

    // Host AI agents (deterministic stub backend) if configured.
    let agent_hosts = agent_host::load_agent_hosts(
        &config.identity_path,
        config.agent_count,
        config.agent_skills.clone(),
    )
    .context("failed to load agent identities")?;
    for host in &agent_hosts {
        register_agent_with_retry(&client, &registered.node_id, host, &config).await?;
    }
    if !agent_hosts.is_empty() {
        info!(count = agent_hosts.len(), "hosted agents registered");
    }

    // Optionally join the libp2p mesh and advertise this node.
    #[cfg(feature = "libp2p")]
    let network: Option<Arc<dyn MeshTransport>> = match config.network_listen.clone() {
        Some(listen) => match start_mesh(
            &identity,
            &listen,
            config.network_bootstrap.as_deref(),
            &config.network_topic,
        )
        .await
        {
            Ok(net) => {
                info!(peer_id = %net.peer_id(), "mesh networking enabled");
                Some(Arc::new(net))
            }
            Err(err) => {
                warn!(error = %err, "mesh networking disabled");
                None
            }
        },
        None => None,
    };
    #[cfg(not(feature = "libp2p"))]
    let network: Option<Arc<dyn MeshTransport>> = {
        if config.network_listen.is_some() {
            warn!("network_listen set but the `libp2p` feature is not enabled");
        }
        None
    };

    run_loop(
        &client,
        &identity,
        &capabilities,
        &limits,
        &config,
        &registered,
        &agent_hosts,
        network.as_deref(),
    )
    .await
}

#[cfg(feature = "libp2p")]
async fn start_mesh(
    identity: &NodeIdentity,
    listen: &str,
    bootstrap: Option<&str>,
    topic: &str,
) -> anyhow::Result<mesh_network::Libp2pTransport> {
    let address: mesh_network::libp2p_transport::Multiaddr =
        listen.parse().context("invalid network listen address")?;
    let bootstrap =
        match bootstrap {
            Some(value) => vec![mesh_network::libp2p_transport::parse_bootstrap(value)
                .map_err(|e| anyhow!("{e}"))?],
            None => vec![],
        };
    mesh_network::libp2p_transport::start_with_identity(identity, vec![address], bootstrap, topic)
        .await
        .map_err(|e| anyhow!("{e}"))
}

async fn register_with_retry(
    client: &ControlPlaneClient,
    request: &RegisterNodeRequest,
) -> anyhow::Result<mesh_protocol::RegisterNodeResponse> {
    for attempt in 1..=30 {
        match client.register(request).await {
            Ok(response) => return Ok(response),
            Err(err) => {
                warn!(attempt, error = %err, "registration failed, retrying");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
    Err(anyhow!(
        "could not register with control plane after retries"
    ))
}

async fn register_agent_with_retry(
    client: &ControlPlaneClient,
    node_id: &mesh_protocol::NodeId,
    host: &AgentHost,
    config: &WorkerConfig,
) -> anyhow::Result<()> {
    let request = mesh_agent_protocol::RegisterAgentRequest {
        node_id: node_id.clone(),
        manifest: host.signed_manifest(node_id, config.offer_ttl_seconds),
    };
    for attempt in 1..=30 {
        match client.register_agent(&request).await {
            Ok(response) => {
                info!(agent_id = %response.agent_id, status = %response.status, "agent registered");
                return Ok(());
            }
            Err(err) => {
                warn!(agent_id = %host.agent_id(), attempt, error = %err, "agent registration failed, retrying");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
    Err(anyhow!(
        "could not register agent {} after retries",
        host.agent_id()
    ))
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    client: &ControlPlaneClient,
    identity: &NodeIdentity,
    capabilities: &NodeCapabilities,
    limits: &ResourceLimits,
    config: &WorkerConfig,
    registered: &mesh_protocol::RegisterNodeResponse,
    agent_hosts: &[AgentHost],
    network: Option<&dyn MeshTransport>,
) -> anyhow::Result<()> {
    let heartbeat_period = Duration::from_secs(registered.heartbeat_interval_seconds.max(1));
    let poll_period = Duration::from_secs(config.poll_interval_seconds.max(1));
    let mut heartbeat = tokio::time::interval(heartbeat_period);
    let mut poll = tokio::time::interval(poll_period);
    let mut beats: u64 = 0;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown signal received, stopping worker");
                break;
            }
            _ = heartbeat.tick() => {
                beats += 1;
                if let Some(max) = config.simulate_crash_after_heartbeats {
                    if beats >= max {
                        error!(beats, "simulating worker crash (test hook)");
                        std::process::exit(70);
                    }
                }
                let request = HeartbeatRequest {
                    offer: build_offer(identity, capabilities, limits, config),
                };
                match client.heartbeat(identity.node_id(), &request).await {
                    Ok(response) => info!(beats, status = %response.status, "heartbeat sent"),
                    Err(err) => warn!(beats, error = %err, "heartbeat failed"),
                }

                for host in agent_hosts {
                    let manifest = host.signed_manifest(identity.node_id(), config.offer_ttl_seconds);
                    let request = AgentHeartbeatRequest { manifest };
                    match client.heartbeat_agent(host.agent_id(), &request).await {
                        Ok(response) => {
                            info!(agent_id = %host.agent_id(), status = %response.status, "agent heartbeat sent")
                        }
                        Err(err) => {
                            warn!(agent_id = %host.agent_id(), error = %err, "agent heartbeat failed")
                        }
                    }
                }

                if let Some(network) = network {
                    let addresses: Vec<String> =
                        config.network_listen.clone().into_iter().collect();
                    match mesh_network::build_advertisement(
                        identity,
                        addresses,
                        vec!["mesh/1".to_string()],
                        config.network_ttl_seconds,
                        Utc::now().timestamp_millis() as u64,
                    ) {
                        Ok(advertisement) => {
                            let signed = mesh_network::sign_advertisement(identity, advertisement);
                            if let Err(err) = network.advertise(signed) {
                                warn!(error = %err, "mesh advertise failed");
                            }
                        }
                        Err(err) => warn!(error = %err, "failed to build advertisement"),
                    }
                }
            }
            _ = poll.tick() => {
                if let Err(err) = poll_worker_jobs(client, identity, config).await {
                    warn!(error = %err, "job poll failed");
                }
                if let Err(err) = poll_agent_jobs(client, config, agent_hosts).await {
                    warn!(error = %err, "agent job poll failed");
                }
            }
        }
    }

    Ok(())
}

async fn poll_worker_jobs(
    client: &ControlPlaneClient,
    identity: &NodeIdentity,
    config: &WorkerConfig,
) -> anyhow::Result<()> {
    if let Some(job) = client
        .claim_job(identity.node_id(), &config.supported_workloads)
        .await?
    {
        handle_claimed(client, job, config, None).await?;
    }
    Ok(())
}

async fn poll_agent_jobs(
    client: &ControlPlaneClient,
    config: &WorkerConfig,
    agent_hosts: &[AgentHost],
) -> anyhow::Result<()> {
    for host in agent_hosts {
        if let Some(job) = client
            .claim_agent_job(host.agent_id(), &config.supported_workloads)
            .await?
        {
            handle_claimed(client, job, config, Some(host)).await?;
        }
    }
    Ok(())
}

fn worker_limits(config: &WorkerConfig) -> RuntimeLimits {
    RuntimeLimits::from_megabytes(
        config.max_memory_mb as u64,
        config.fuel_limit,
        std::time::Duration::from_secs(config.execution_timeout_seconds.max(1)),
    )
}

fn agent_limits(config: &WorkerConfig) -> RuntimeLimits {
    RuntimeLimits::from_megabytes(
        config.agent_max_memory_mb,
        config.agent_fuel_limit,
        std::time::Duration::from_secs(config.agent_execution_timeout_seconds.max(1)),
    )
}

async fn handle_claimed(
    client: &ControlPlaneClient,
    job: ClaimedJobResponse,
    config: &WorkerConfig,
    agent: Option<&AgentHost>,
) -> anyhow::Result<()> {
    info!(
        job_id = %job.job_id,
        kind = %job.kind,
        attempt = job.attempt_number,
        workload_type = %job.workload_type,
        depth = job.depth,
        "claimed job"
    );

    if job.workload_type == "delegate" {
        match delegate_workload(client, &job, agent.map(AgentHost::agent_id)).await {
            Ok(count) => {
                info!(job_id = %job.job_id, children = count, "delegated subtasks; waiting on children")
            }
            Err(reason) => {
                warn!(job_id = %job.job_id, reason = %reason, "delegation failed");
                let request = FailJobRequest {
                    attempt_id: job.attempt_id.clone(),
                    error: reason,
                };
                let _ = client.fail_job(&job.job_id, &request).await;
            }
        }
        return Ok(());
    }

    // Capability allowlist: an agent may only run workloads its tools permit.
    if let Some(host) = agent {
        if !host.allows_workload(&job.workload_type) {
            warn!(
                job_id = %job.job_id,
                workload = %job.workload_type,
                agent = %host.agent_id(),
                "workload not permitted by agent tool allowlist"
            );
            let request = FailJobRequest {
                attempt_id: job.attempt_id.clone(),
                error: format!(
                    "workload '{}' not permitted by agent tool allowlist",
                    job.workload_type
                ),
            };
            let _ = client.fail_job(&job.job_id, &request).await;
            return Ok(());
        }
    }

    let limits = if agent.is_some() {
        agent_limits(config)
    } else {
        worker_limits(config)
    };
    let workload_type = job.workload_type.clone();
    let input = job.input.clone();
    let worker_config = config.clone();
    let executed = tokio::task::spawn_blocking(move || {
        execute(&workload_type, &input, &worker_config, &limits)
    })
    .await
    .unwrap_or_else(|err| Err(format!("execution task failed: {err}")));

    match executed {
        Ok((result_hash, log)) => {
            let request = CompleteJobRequest {
                attempt_id: job.attempt_id.clone(),
                result_hash,
                log: Some(log),
            };
            match client.complete_job(&job.job_id, &request).await {
                Ok(response) => {
                    info!(job_id = %job.job_id, state = %response.state, "job completed")
                }
                Err(err) => {
                    warn!(job_id = %job.job_id, error = %err, "failed to report completion")
                }
            }
        }
        Err(reason) => {
            warn!(job_id = %job.job_id, reason = %reason, "job execution failed");
            let request = FailJobRequest {
                attempt_id: job.attempt_id.clone(),
                error: reason,
            };
            if let Err(err) = client.fail_job(&job.job_id, &request).await {
                warn!(job_id = %job.job_id, error = %err, "failed to report failure");
            }
        }
    }

    Ok(())
}

async fn delegate_workload(
    client: &ControlPlaneClient,
    job: &ClaimedJobResponse,
    delegator: Option<&AgentId>,
) -> Result<u32, String> {
    #[derive(serde::Deserialize)]
    struct DelegateSpec {
        subtask_workload_type: String,
        subtask_input: serde_json::Value,
        count: u32,
    }

    let spec: DelegateSpec = serde_json::from_value(job.input.clone())
        .map_err(|e| format!("invalid delegate input: {e}"))?;
    if spec.count == 0 {
        return Ok(0);
    }

    let mut delegatees: Vec<String> = client
        .fetch_agents()
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|agent| agent.status.accepts_work())
        .map(|agent| agent.agent_id.to_string())
        .collect();
    delegatees.sort();
    if let Some(me) = delegator {
        delegatees.retain(|agent| agent != me.as_str());
    }
    if delegatees.is_empty() {
        return Err("no other online agents available to delegate to".to_string());
    }

    let remaining = (job.deadline - Utc::now()).num_seconds().max(1) as u64;
    let deadline_seconds = remaining.saturating_sub(1).clamp(1, 600);

    let mut created = 0u32;
    for index in 0..spec.count {
        let delegatee = AgentId::new(delegatees[index as usize % delegatees.len()].clone());
        let request = DelegateJobRequest {
            delegatee_agent_id: delegatee,
            workload_type: spec.subtask_workload_type.clone(),
            input: spec.subtask_input.clone(),
            budget_credits: 1,
            deadline_seconds,
        };
        client
            .delegate_job(&job.job_id, &request)
            .await
            .map_err(|e| e.to_string())?;
        created += 1;
    }

    client
        .wait_job(&job.job_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(created)
}

/// Execute a workload deterministically. Never blocks the async runtime: the
/// caller runs this on a blocking thread.
fn execute(
    workload_type: &str,
    input: &serde_json::Value,
    config: &WorkerConfig,
    limits: &RuntimeLimits,
) -> Result<(String, String), String> {
    match workload_type {
        "echo" => {
            let result_hash = hash_bytes(&serde_json::to_vec(input).unwrap_or_default());
            let log = format!("echo input={input}");
            Ok((result_hash, log))
        }
        "wasm" => execute_wasm(input, config, limits),
        other => Err(format!("unsupported workload type: {other}")),
    }
}

fn execute_wasm(
    input: &serde_json::Value,
    config: &WorkerConfig,
    limits: &RuntimeLimits,
) -> Result<(String, String), String> {
    let job_input: WasmJobInput = serde_json::from_value(input.clone())
        .map_err(|e| format!("invalid wasm job input: {e}"))?;
    let module = job_input
        .module_bytes()
        .map_err(|e| format!("invalid module encoding: {e}"))?;
    let data = serde_json::to_vec(&job_input.data).map_err(|e| e.to_string())?;

    let output = if config.runtime_isolation {
        match std::env::current_exe() {
            Ok(exe) => mesh_runtime::run_isolated(&exe, &module, &data, limits),
            Err(err) => Err(mesh_runtime::RuntimeError::Io(err.to_string())),
        }
    } else {
        mesh_runtime::run(&module, &data, limits)
    }
    .map_err(|e| e.to_string())?;

    let result_hash = hash_bytes(&output.output);
    let log = format!(
        "wasm output ({} bytes, {} fuel): {}",
        output.output.len(),
        output.fuel_consumed,
        preview(&output.output)
    );
    Ok((result_hash, log))
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn preview(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).chars().take(200).collect()
}

fn build_offer(
    identity: &NodeIdentity,
    capabilities: &NodeCapabilities,
    limits: &ResourceLimits,
    config: &WorkerConfig,
) -> SignedResourceOffer {
    let offer = ResourceOffer {
        node_id: identity.node_id().clone(),
        capabilities: capabilities.clone(),
        limits: limits.clone(),
        availability: config.availability,
        expires_at: Utc::now() + ChronoDuration::seconds(config.offer_ttl_seconds as i64),
        nonce: Utc::now().timestamp_millis() as u64,
    };
    let signature = identity.sign_canonical(&offer);
    SignedResourceOffer {
        offer,
        public_key: identity.public_key_hex(),
        signature,
    }
}

fn derive_participant_id(identity: &NodeIdentity) -> String {
    let public_key = identity.public_key_hex();
    let short = &public_key[..16.min(public_key.len())];
    format!("ptc-{short}")
}

fn init_telemetry(config: &WorkerConfig) {
    let filter = EnvFilter::try_new(&config.log_level).unwrap_or_else(|_| EnvFilter::new("info"));
    let result = if config.log_json {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .try_init()
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).try_init()
    };
    if let Err(err) = result {
        error!(error = %err, "failed to initialize tracing subscriber");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_protocol::NodeId;

    #[test]
    fn participant_id_is_derived_from_public_key() {
        let identity = NodeIdentity::generate(NodeId::new("node-test"));
        let pid = derive_participant_id(&identity);
        assert!(pid.starts_with("ptc-"));
        assert_eq!(pid.len(), 4 + 16);
    }

    #[test]
    fn built_offer_verifies_and_is_not_expired() {
        let identity = NodeIdentity::generate(NodeId::new("node-test"));
        let config = WorkerConfig::default();
        let caps = NodeCapabilities {
            cpu_cores: 2,
            ram_gb: 4,
            storage_gb: 10,
            gpu: false,
        };
        let limits = ResourceLimits {
            max_cpu_percent: 50,
            max_memory_mb: 1024,
        };
        let signed = build_offer(&identity, &caps, &limits, &config);
        assert!(!signed.offer.is_expired_at(Utc::now()));
        assert!(NodeIdentity::verify_canonical(
            &signed.public_key,
            &signed.offer,
            &signed.signature
        )
        .is_ok());
    }

    #[test]
    fn echo_hash_is_deterministic() {
        let input = serde_json::json!({"n": 5, "op": "square"});
        let config = WorkerConfig::default();
        let (hash_a, _) = execute("echo", &input, &config, &worker_limits(&config)).unwrap();
        let (hash_b, _) = execute("echo", &input, &config, &worker_limits(&config)).unwrap();
        assert_eq!(hash_a, hash_b);
        assert_eq!(hash_a.len(), 64);
    }

    #[test]
    fn unknown_workload_is_rejected() {
        let config = WorkerConfig::default();
        assert!(execute("cobol", &serde_json::json!({}), &config, &worker_limits(&config)).is_err());
    }

    #[test]
    fn wasm_workload_runs_in_process() {
        const HELLO_WAT: &str = r#"
            (module
              (memory (export "memory") 1)
              (data (i32.const 16) "hello from meshcloud\00")
              (func (export "alloc") (param i32) (result i32) (i32.const 1024))
              (func (export "run") (param i32 i32) (result i32) (i32.const 16)))
        "#;
        let config = WorkerConfig {
            runtime_isolation: false,
            ..WorkerConfig::default()
        };
        let input = serde_json::to_value(WasmJobInput::from_module_bytes(
            HELLO_WAT.as_bytes(),
            serde_json::json!({}),
        ))
        .unwrap();
        let (hash, log) = execute("wasm", &input, &config, &worker_limits(&config)).unwrap();
        assert_eq!(hash.len(), 64);
        assert!(log.contains("hello from meshcloud"));
    }
}

