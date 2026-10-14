//! `mesh` — the MeshCloud command-line interface.

#![forbid(unsafe_code)]

use anyhow::Context;
use base64::Engine as _;
use clap::{Parser, Subcommand};
use mesh_agent_protocol::AgentView;
use mesh_protocol::{
    AbuseReportRequest, AbuseReportResponse, AgentId, ContributionView, CreateReservationRequest,
    CreditBalanceView, DelegationView, DownloadObjectResponse, HealthResponse, JobDetailResponse,
    JobTreeResponse, JobView, NodeView, ParticipantView, PriceEstimate, RepairObjectResponse,
    ReservationView, SetWalletRequest, StorageObjectView, SubmitJobRequest, SubmitJobResponse,
    UploadObjectRequest, WasmJobInput,
};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "mesh", version, about = "MeshCloud command-line interface")]
struct Cli {
    /// Base URL of the control plane.
    #[arg(
        long,
        global = true,
        env = "MESH_CONTROL_PLANE_URL",
        default_value = "http://localhost:8080"
    )]
    control_plane_url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Check that the control plane is reachable.
    Health,
    /// Inspect and list worker nodes.
    Node {
        #[command(subcommand)]
        command: NodeCommand,
    },
    /// Inspect and list AI agents.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Manage participant accounts.
    Participant {
        #[command(subcommand)]
        command: ParticipantCommand,
    },
    /// Browse the marketplace: available compute, agents, and prices.
    Marketplace {
        #[command(subcommand)]
        command: MarketplaceCommand,
    },
    /// Store and retrieve objects on the distributed storage layer.
    Storage {
        #[command(subcommand)]
        command: StorageCommand,
    },
    /// Submit and inspect jobs.
    Job {
        #[command(subcommand)]
        command: JobCommand,
    },
    /// Inspect contribution receipts.
    Contribution {
        #[command(subcommand)]
        command: ContributionCommand,
    },
    /// Inspect wallet balances and settlements.
    Wallet {
        #[command(subcommand)]
        command: WalletCommand,
    },
    /// Report and inspect abuse.
    Abuse {
        #[command(subcommand)]
        command: AbuseCommand,
    },
    /// Print Prometheus metrics.
    Metrics,
}

#[derive(Subcommand, Debug)]
enum AbuseCommand {
    /// Report abuse against a node or agent.
    Report {
        /// Subject type: `node` or `agent`.
        #[arg(long, default_value = "node")]
        subject_type: String,
        /// Subject identifier.
        #[arg(long)]
        subject: String,
        /// Category, e.g. `malicious_output`.
        #[arg(long)]
        category: String,
        /// Free-form detail.
        #[arg(long)]
        detail: Option<String>,
    },
    /// List abuse reports.
    Ls,
}

#[derive(Subcommand, Debug)]
enum WalletCommand {
    /// Show a participant's credit balance.
    Balance {
        /// Participant identifier.
        #[arg(long)]
        participant: String,
    },
}

#[derive(Subcommand, Debug)]
enum ContributionCommand {
    /// List recent contribution receipts.
    Ls {
        /// Maximum number of receipts to show.
        #[arg(long, default_value_t = 50)]
        limit: i64,
        /// Filter by agent.
        #[arg(long)]
        agent: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum NodeCommand {
    /// List all registered nodes.
    Ls,
    /// Show details for a single node.
    Inspect {
        /// Node identifier.
        node_id: String,
    },
}

#[derive(Subcommand, Debug)]
enum AgentCommand {
    /// List all registered agents.
    Ls,
    /// Show details for a single agent.
    Inspect {
        /// Agent identifier.
        agent_id: String,
    },
    /// Show aggregated rewards for an agent.
    Rewards {
        /// Agent identifier.
        agent_id: String,
    },
    /// Show an agent's capability manifest.
    Manifest {
        /// Agent identifier.
        agent_id: String,
    },
    /// Show an agent's delegation history.
    Delegations {
        /// Agent identifier.
        agent_id: String,
    },
    /// Submit a root agent task that plans and delegates subtasks.
    Task {
        #[command(subcommand)]
        command: AgentTaskCommand,
    },
}

#[derive(Subcommand, Debug)]
enum AgentTaskCommand {
    /// Submit a root task to an agent (deterministic subtask delegation).
    Submit {
        /// Orchestrator agent identifier.
        #[arg(long)]
        agent: String,
        /// Optional WASM/WAT module for each subtask (defaults to `echo`).
        #[arg(long)]
        file: Option<PathBuf>,
        /// Number of subtasks to delegate.
        #[arg(long, default_value_t = 2)]
        count: u32,
        /// Maximum delegation depth.
        #[arg(long, default_value_t = 2)]
        max_depth: u32,
        /// Maximum children the root may delegate.
        #[arg(long, default_value_t = 8)]
        max_fanout: u32,
        /// Budget for the tree.
        #[arg(long, default_value_t = 10)]
        budget: i64,
        /// Deadline in seconds from now.
        #[arg(long, default_value_t = 3600)]
        deadline_seconds: u64,
    },
}

#[derive(Subcommand, Debug)]
enum ParticipantCommand {
    /// List participant accounts.
    Ls,
    /// Show a participant account.
    Inspect {
        /// Participant identifier.
        participant_id: String,
    },
    /// Associate a wallet address with a participant.
    SetWallet {
        /// Participant identifier.
        participant_id: String,
        /// Wallet address.
        address: String,
    },
    /// Reserve credits for future work.
    Reserve {
        /// Participant identifier.
        participant_id: String,
        /// Credits to reserve.
        #[arg(long)]
        credits: i64,
        /// Workload type.
        #[arg(long, default_value = "wasm")]
        workload_type: String,
        /// Lifetime in seconds.
        #[arg(long, default_value_t = 3600)]
        ttl_seconds: u64,
    },
    /// Show a participant's reward history.
    History {
        /// Participant identifier.
        participant_id: String,
        /// Maximum receipts to show.
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
}

#[derive(Subcommand, Debug)]
enum MarketplaceCommand {
    /// List available compute matching capabilities.
    Nodes {
        #[arg(long, default_value_t = 0)]
        min_cpu: u32,
        #[arg(long, default_value_t = 0)]
        min_ram: u32,
        #[arg(long)]
        gpu: Option<bool>,
    },
    /// List available agents matching capabilities.
    Agents {
        #[arg(long)]
        skill: Option<String>,
        #[arg(long)]
        tool: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
    /// Estimate the price of a workload.
    Price {
        /// Workload type.
        #[arg(long, default_value = "wasm")]
        workload_type: String,
        /// Units.
        #[arg(long, default_value_t = 1)]
        units: i64,
    },
}

#[derive(Subcommand, Debug)]
enum StorageCommand {
    /// Upload a file.
    Put {
        /// File to upload.
        file: PathBuf,
        /// Owning participant.
        #[arg(long, default_value = "local")]
        owner: String,
        /// Data shards.
        #[arg(long)]
        data_shards: Option<u32>,
        /// Parity shards.
        #[arg(long)]
        parity_shards: Option<u32>,
    },
    /// Download an object to a file.
    Get {
        /// Object identifier.
        object_id: String,
        /// Output file.
        out: PathBuf,
    },
    /// Show object metadata and shards.
    Info {
        /// Object identifier.
        object_id: String,
    },
    /// Repair missing shards.
    Repair {
        /// Object identifier.
        object_id: String,
    },
    /// List stored objects.
    Ls,
}

#[derive(Subcommand, Debug)]
enum JobCommand {
    /// Submit a job.
    Submit {
        /// Workload type, e.g. `echo` or `wasm`.
        #[arg(long = "type", default_value = "echo")]
        workload_type: String,
        /// Job input as a JSON value (used for non-file workloads).
        #[arg(long, default_value = "{}")]
        input: String,
        /// A WASM/WAT module file to run (implies `--type wasm`).
        #[arg(long)]
        file: Option<PathBuf>,
        /// Data passed to the module, as JSON (with `--file`).
        #[arg(long, default_value = "{}")]
        data: String,
        /// Minimum CPU cores required.
        #[arg(long, default_value_t = 0)]
        cpu: u32,
        /// Minimum RAM (GB) required.
        #[arg(long, default_value_t = 0)]
        ram: u32,
        /// Maximum attempts.
        #[arg(long, default_value_t = 3)]
        max_attempts: u32,
        /// Deadline in seconds from now.
        #[arg(long, default_value_t = 3600)]
        deadline_seconds: u64,
    },
    /// List recent jobs.
    Ls {
        /// Maximum number of jobs to show.
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
    /// Show a job and its attempts.
    Status {
        /// Job identifier.
        job_id: String,
    },
    /// Show captured logs for a job's attempts.
    Logs {
        /// Job identifier.
        job_id: String,
    },
    /// Show the delegation tree for a job.
    Tree {
        /// Job identifier (any job in the tree).
        job_id: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let base = cli.control_plane_url.trim_end_matches('/').to_string();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    match cli.command {
        Command::Health => health(&client, &base).await?,
        Command::Node { command } => match command {
            NodeCommand::Ls => node_ls(&client, &base).await?,
            NodeCommand::Inspect { node_id } => node_inspect(&client, &base, &node_id).await?,
        },
        Command::Agent { command } => match command {
            AgentCommand::Ls => agent_ls(&client, &base).await?,
            AgentCommand::Inspect { agent_id } => agent_inspect(&client, &base, &agent_id).await?,
            AgentCommand::Rewards { agent_id } => agent_rewards(&client, &base, &agent_id).await?,
            AgentCommand::Manifest { agent_id } => {
                agent_manifest(&client, &base, &agent_id).await?
            }
            AgentCommand::Delegations { agent_id } => {
                agent_delegations(&client, &base, &agent_id).await?
            }
            AgentCommand::Task { command } => match command {
                AgentTaskCommand::Submit {
                    agent,
                    file,
                    count,
                    max_depth,
                    max_fanout,
                    budget,
                    deadline_seconds,
                } => {
                    agent_task_submit(
                        &client,
                        &base,
                        AgentTaskOptions {
                            agent,
                            file,
                            count,
                            max_depth,
                            max_fanout,
                            budget,
                            deadline_seconds,
                        },
                    )
                    .await?;
                }
            },
        },
        Command::Participant { command } => match command {
            ParticipantCommand::Ls => participant_ls(&client, &base).await?,
            ParticipantCommand::Inspect { participant_id } => {
                participant_inspect(&client, &base, &participant_id).await?
            }
            ParticipantCommand::SetWallet {
                participant_id,
                address,
            } => participant_set_wallet(&client, &base, &participant_id, &address).await?,
            ParticipantCommand::Reserve {
                participant_id,
                credits,
                workload_type,
                ttl_seconds,
            } => {
                participant_reserve(
                    &client,
                    &base,
                    &participant_id,
                    credits,
                    &workload_type,
                    ttl_seconds,
                )
                .await?
            }
            ParticipantCommand::History {
                participant_id,
                limit,
            } => participant_history(&client, &base, &participant_id, limit).await?,
        },
        Command::Marketplace { command } => match command {
            MarketplaceCommand::Nodes {
                min_cpu,
                min_ram,
                gpu,
            } => marketplace_nodes(&client, &base, min_cpu, min_ram, gpu).await?,
            MarketplaceCommand::Agents { skill, tool, model } => {
                marketplace_agents(&client, &base, skill, tool, model).await?
            }
            MarketplaceCommand::Price {
                workload_type,
                units,
            } => marketplace_price(&client, &base, &workload_type, units).await?,
        },
        Command::Storage { command } => match command {
            StorageCommand::Put {
                file,
                owner,
                data_shards,
                parity_shards,
            } => storage_put(&client, &base, &file, &owner, data_shards, parity_shards).await?,
            StorageCommand::Get { object_id, out } => {
                storage_get(&client, &base, &object_id, &out).await?
            }
            StorageCommand::Info { object_id } => storage_info(&client, &base, &object_id).await?,
            StorageCommand::Repair { object_id } => {
                storage_repair(&client, &base, &object_id).await?
            }
            StorageCommand::Ls => storage_ls(&client, &base).await?,
        },
        Command::Job { command } => match command {
            JobCommand::Submit {
                workload_type,
                input,
                file,
                data,
                cpu,
                ram,
                max_attempts,
                deadline_seconds,
            } => {
                job_submit(
                    &client,
                    &base,
                    SubmitOptions {
                        workload_type,
                        input,
                        file,
                        data,
                        cpu,
                        ram,
                        max_attempts,
                        deadline_seconds,
                    },
                )
                .await?;
            }
            JobCommand::Ls { limit } => job_ls(&client, &base, limit).await?,
            JobCommand::Status { job_id } => job_status(&client, &base, &job_id).await?,
            JobCommand::Logs { job_id } => job_logs(&client, &base, &job_id).await?,
            JobCommand::Tree { job_id } => job_tree(&client, &base, &job_id).await?,
        },
        Command::Contribution { command } => match command {
            ContributionCommand::Ls { limit, agent } => {
                contribution_ls(&client, &base, limit, agent.as_deref()).await?
            }
        },
        Command::Wallet { command } => match command {
            WalletCommand::Balance { participant } => {
                wallet_balance(&client, &base, &participant).await?
            }
        },
        Command::Abuse { command } => match command {
            AbuseCommand::Report {
                subject_type,
                subject,
                category,
                detail,
            } => {
                abuse_report(
                    &client,
                    &base,
                    &subject_type,
                    &subject,
                    &category,
                    detail.as_deref(),
                )
                .await?
            }
            AbuseCommand::Ls => abuse_ls(&client, &base).await?,
        },
        Command::Metrics => metrics(&client, &base).await?,
    }

    Ok(())
}

async fn health(client: &reqwest::Client, base: &str) -> anyhow::Result<()> {
    let health: HealthResponse = client
        .get(format!("{base}/api/v1/health"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "status: {}  version: {}  protocol: v{}  uptime: {}s",
        health.status, health.version, health.protocol_version, health.uptime_seconds
    );
    Ok(())
}

async fn fetch_nodes(client: &reqwest::Client, base: &str) -> anyhow::Result<Vec<NodeView>> {
    let nodes = client
        .get(format!("{base}/api/v1/nodes"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    Ok(nodes)
}

async fn node_ls(client: &reqwest::Client, base: &str) -> anyhow::Result<()> {
    let nodes = fetch_nodes(client, base).await?;
    if nodes.is_empty() {
        println!("no nodes registered");
        return Ok(());
    }
    println!(
        "{:<28} {:<26} {:<10} CPU/RAM/STORAGE",
        "NODE", "PARTICIPANT", "STATUS"
    );
    for node in nodes {
        let caps = &node.capabilities;
        println!(
            "{:<28} {:<26} {:<10} {}c / {}GB / {}GB{}",
            node.node_id,
            node.participant_id,
            node.status,
            caps.cpu_cores,
            caps.ram_gb,
            caps.storage_gb,
            if caps.gpu { " +gpu" } else { "" }
        );
    }
    Ok(())
}

async fn node_inspect(client: &reqwest::Client, base: &str, node_id: &str) -> anyhow::Result<()> {
    let nodes = fetch_nodes(client, base).await?;
    let node = nodes
        .into_iter()
        .find(|n| n.node_id.as_str() == node_id)
        .with_context(|| format!("node not found: {node_id}"))?;
    println!("{}", serde_json::to_string_pretty(&node)?);
    Ok(())
}

async fn fetch_agents(client: &reqwest::Client, base: &str) -> anyhow::Result<Vec<AgentView>> {
    let agents = client
        .get(format!("{base}/api/v1/agents"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    Ok(agents)
}

async fn agent_ls(client: &reqwest::Client, base: &str) -> anyhow::Result<()> {
    let agents = fetch_agents(client, base).await?;
    if agents.is_empty() {
        println!("no agents registered");
        return Ok(());
    }
    println!("{:<38} {:<26} {:<10} BACKEND", "AGENT", "NODE", "STATUS");
    for agent in agents {
        println!(
            "{:<38} {:<26} {:<10} {}",
            agent.agent_id,
            agent.node_id,
            agent.status,
            agent.backend.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

async fn agent_inspect(client: &reqwest::Client, base: &str, agent_id: &str) -> anyhow::Result<()> {
    let agent: AgentView = client
        .get(format!("{base}/api/v1/agents/{agent_id}"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!("{}", serde_json::to_string_pretty(&agent)?);
    Ok(())
}

struct SubmitOptions {
    workload_type: String,
    input: String,
    file: Option<PathBuf>,
    data: String,
    cpu: u32,
    ram: u32,
    max_attempts: u32,
    deadline_seconds: u64,
}

async fn job_submit(
    client: &reqwest::Client,
    base: &str,
    options: SubmitOptions,
) -> anyhow::Result<()> {
    let (workload_type, input) = if let Some(file) = options.file {
        let module = std::fs::read(&file)
            .with_context(|| format!("could not read module file {}", file.display()))?;
        let data: serde_json::Value =
            serde_json::from_str(&options.data).context("--data must be valid JSON")?;
        let wasm_input = WasmJobInput::from_module_bytes(&module, data);
        (
            "wasm".to_string(),
            serde_json::to_value(wasm_input).context("failed to encode wasm input")?,
        )
    } else {
        let input: serde_json::Value =
            serde_json::from_str(&options.input).context("--input must be valid JSON")?;
        (options.workload_type, input)
    };

    let request = SubmitJobRequest {
        workload_type,
        input,
        kind: "job".to_string(),
        assignee_agent_id: None,
        required_cpu_cores: options.cpu,
        required_ram_gb: options.ram,
        max_attempts: options.max_attempts,
        deadline_seconds: options.deadline_seconds,
        max_depth: 0,
        max_fanout: 0,
        budget_credits: 0,
    };
    let response: SubmitJobResponse = client
        .post(format!("{base}/api/v1/jobs"))
        .json(&request)
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "submitted job {} (state {}, deadline {})",
        response.job_id, response.state, response.deadline
    );
    Ok(())
}

async fn fetch_job(
    client: &reqwest::Client,
    base: &str,
    job_id: &str,
) -> anyhow::Result<JobDetailResponse> {
    let detail = client
        .get(format!("{base}/api/v1/jobs/{job_id}"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    Ok(detail)
}

async fn job_ls(client: &reqwest::Client, base: &str, limit: i64) -> anyhow::Result<()> {
    let jobs: Vec<JobView> = client
        .get(format!("{base}/api/v1/jobs?limit={limit}"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    if jobs.is_empty() {
        println!("no jobs");
        return Ok(());
    }
    println!(
        "{:<40} {:<10} {:<12} {:<9} RESULT",
        "JOB", "TYPE", "STATE", "ATTEMPTS"
    );
    for job in jobs {
        println!(
            "{:<40} {:<10} {:<12} {:<9} {}",
            job.job_id,
            job.workload_type,
            job.state,
            format!("{}/{}", job.attempts, job.max_attempts),
            job.result_hash.unwrap_or_default()
        );
    }
    Ok(())
}

async fn job_status(client: &reqwest::Client, base: &str, job_id: &str) -> anyhow::Result<()> {
    let detail = fetch_job(client, base, job_id).await?;
    let job = &detail.job;
    println!("job      {}", job.job_id);
    println!("type     {}", job.workload_type);
    println!("state    {}", job.state);
    println!(
        "worker   {}",
        job.worker_id.as_ref().map(|n| n.as_str()).unwrap_or("-")
    );
    println!("attempts {}/{}", job.attempts, job.max_attempts);
    println!("deadline {}", job.deadline);
    println!("result   {}", job.result_hash.as_deref().unwrap_or("-"));
    for attempt in &detail.attempts {
        println!(
            "  attempt {} {} worker={} result={}",
            attempt.attempt_number,
            attempt.state,
            attempt.worker_id,
            attempt.result_hash.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

async fn job_logs(client: &reqwest::Client, base: &str, job_id: &str) -> anyhow::Result<()> {
    let detail = fetch_job(client, base, job_id).await?;
    if detail.attempts.is_empty() {
        println!("no attempts recorded for {}", detail.job.job_id);
        return Ok(());
    }
    for attempt in &detail.attempts {
        println!(
            "--- attempt {} ({}) ---",
            attempt.attempt_number, attempt.state
        );
        println!("{}", attempt.log.as_deref().unwrap_or("(no logs)"));
    }
    Ok(())
}

async fn contribution_ls(
    client: &reqwest::Client,
    base: &str,
    limit: i64,
    agent: Option<&str>,
) -> anyhow::Result<()> {
    let mut url = format!("{base}/api/v1/contributions?limit={limit}");
    if let Some(agent) = agent {
        url.push_str(&format!("&agent={agent}"));
    }
    let contributions: Vec<ContributionView> = client
        .get(url)
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    if contributions.is_empty() {
        println!("no contributions");
        return Ok(());
    }
    println!(
        "{:<40} {:<20} {:<10} {:<9} EVIDENCE",
        "RECEIPT", "PARTICIPANT", "ROLE", "REWARD"
    );
    for receipt in contributions {
        println!(
            "{:<40} {:<20} {:<10} {:<9} {}",
            receipt.receipt_id,
            receipt.participant_id,
            receipt.role,
            receipt.reward_amount,
            receipt.evidence_hash
        );
    }
    Ok(())
}

async fn agent_rewards(client: &reqwest::Client, base: &str, agent_id: &str) -> anyhow::Result<()> {
    let rewards: mesh_protocol::AgentRewardView = client
        .get(format!("{base}/api/v1/agents/{agent_id}/rewards"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!("agent    {}", rewards.agent_id);
    println!("receipts {}", rewards.receipt_count);
    println!("reward   {}", rewards.total_reward);
    Ok(())
}

async fn wallet_balance(
    client: &reqwest::Client,
    base: &str,
    participant: &str,
) -> anyhow::Result<()> {
    let balance: CreditBalanceView = client
        .get(format!("{base}/api/v1/participants/{participant}/balance"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!("participant {}", balance.participant_id);
    println!("earned      {}", balance.earned);
    println!("spent       {}", balance.spent);
    println!("reserved    {}", balance.reserved);
    println!("available   {}", balance.available);
    Ok(())
}

async fn job_tree(client: &reqwest::Client, base: &str, job_id: &str) -> anyhow::Result<()> {
    let tree: JobTreeResponse = client
        .get(format!("{base}/api/v1/jobs/{job_id}/tree"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!("root {}", tree.root_job_id);
    for job in tree.jobs {
        let indent = "  ".repeat(job.depth as usize);
        println!(
            "{indent}{} {} {} agent={} result={}",
            job.job_id,
            job.kind,
            job.state,
            job.assignee_agent_id
                .as_ref()
                .map(|a| a.as_str())
                .unwrap_or("-"),
            job.result_hash.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

struct AgentTaskOptions {
    agent: String,
    file: Option<PathBuf>,
    count: u32,
    max_depth: u32,
    max_fanout: u32,
    budget: i64,
    deadline_seconds: u64,
}

async fn agent_task_submit(
    client: &reqwest::Client,
    base: &str,
    options: AgentTaskOptions,
) -> anyhow::Result<()> {
    let (subtask_workload_type, subtask_input) = if let Some(file) = options.file {
        let module = std::fs::read(&file)
            .with_context(|| format!("could not read module file {}", file.display()))?;
        (
            "wasm".to_string(),
            serde_json::to_value(WasmJobInput::from_module_bytes(
                &module,
                serde_json::json!({}),
            ))?,
        )
    } else {
        ("echo".to_string(), serde_json::json!({}))
    };

    let input = serde_json::json!({
        "subtask_workload_type": subtask_workload_type,
        "subtask_input": subtask_input,
        "count": options.count,
    });

    let request = SubmitJobRequest {
        workload_type: "delegate".to_string(),
        input,
        kind: "agent_task".to_string(),
        assignee_agent_id: Some(AgentId::new(options.agent)),
        required_cpu_cores: 0,
        required_ram_gb: 0,
        max_attempts: 2,
        deadline_seconds: options.deadline_seconds,
        max_depth: options.max_depth,
        max_fanout: options.max_fanout,
        budget_credits: options.budget,
    };

    let response: SubmitJobResponse = client
        .post(format!("{base}/api/v1/jobs"))
        .json(&request)
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "submitted agent task {} (state {}, deadline {})",
        response.job_id, response.state, response.deadline
    );
    Ok(())
}

async fn participant_ls(client: &reqwest::Client, base: &str) -> anyhow::Result<()> {
    let participants: Vec<ParticipantView> = client
        .get(format!("{base}/api/v1/participants"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    if participants.is_empty() {
        println!("no participants");
        return Ok(());
    }
    println!("{:<22} {:<14} WALLET", "PARTICIPANT", "STATUS");
    for p in participants {
        println!(
            "{:<22} {:<14} {}",
            p.participant_id,
            p.status,
            p.wallet_address.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

async fn participant_inspect(
    client: &reqwest::Client,
    base: &str,
    participant_id: &str,
) -> anyhow::Result<()> {
    let participant: ParticipantView = client
        .get(format!("{base}/api/v1/participants/{participant_id}"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!("{}", serde_json::to_string_pretty(&participant)?);
    Ok(())
}

async fn participant_set_wallet(
    client: &reqwest::Client,
    base: &str,
    participant_id: &str,
    address: &str,
) -> anyhow::Result<()> {
    let request = SetWalletRequest {
        wallet_address: address.to_string(),
    };
    let participant: ParticipantView = client
        .post(format!(
            "{base}/api/v1/participants/{participant_id}/wallet"
        ))
        .json(&request)
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "participant {} wallet={}",
        participant.participant_id,
        participant.wallet_address.as_deref().unwrap_or("-")
    );
    Ok(())
}

async fn participant_reserve(
    client: &reqwest::Client,
    base: &str,
    participant_id: &str,
    credits: i64,
    workload_type: &str,
    ttl_seconds: u64,
) -> anyhow::Result<()> {
    let request = CreateReservationRequest {
        workload_type: workload_type.to_string(),
        credits,
        ttl_seconds,
    };
    let reservation: ReservationView = client
        .post(format!(
            "{base}/api/v1/participants/{participant_id}/reservations"
        ))
        .json(&request)
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "reserved {} credits ({}) until {}",
        reservation.credits, reservation.reservation_id, reservation.expires_at
    );
    Ok(())
}

async fn participant_history(
    client: &reqwest::Client,
    base: &str,
    participant_id: &str,
    limit: i64,
) -> anyhow::Result<()> {
    let contributions: Vec<ContributionView> = client
        .get(format!(
            "{base}/api/v1/participants/{participant_id}/contributions?limit={limit}"
        ))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    if contributions.is_empty() {
        println!("no contributions");
        return Ok(());
    }
    println!("{:<40} {:<12} {:<9} EVIDENCE", "RECEIPT", "ROLE", "REWARD");
    for receipt in contributions {
        println!(
            "{:<40} {:<12} {:<9} {}",
            receipt.receipt_id, receipt.role, receipt.reward_amount, receipt.evidence_hash
        );
    }
    Ok(())
}

async fn marketplace_nodes(
    client: &reqwest::Client,
    base: &str,
    min_cpu: u32,
    min_ram: u32,
    gpu: Option<bool>,
) -> anyhow::Result<()> {
    let mut url =
        format!("{base}/api/v1/marketplace/nodes?min_cpu_cores={min_cpu}&min_ram_gb={min_ram}");
    if let Some(gpu) = gpu {
        url.push_str(&format!("&gpu={gpu}"));
    }
    let nodes: Vec<NodeView> = client
        .get(url)
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    if nodes.is_empty() {
        println!("no matching nodes");
        return Ok(());
    }
    println!("{:<28} {:<26} CPU/RAM/STORAGE", "NODE", "PARTICIPANT");
    for node in nodes {
        let caps = &node.capabilities;
        println!(
            "{:<28} {:<26} {}c / {}GB / {}GB{}",
            node.node_id,
            node.participant_id,
            caps.cpu_cores,
            caps.ram_gb,
            caps.storage_gb,
            if caps.gpu { " +gpu" } else { "" }
        );
    }
    Ok(())
}

async fn marketplace_agents(
    client: &reqwest::Client,
    base: &str,
    skill: Option<String>,
    tool: Option<String>,
    model: Option<String>,
) -> anyhow::Result<()> {
    let mut url = format!("{base}/api/v1/marketplace/agents?");
    if let Some(skill) = skill {
        url.push_str(&format!("skill={skill}&"));
    }
    if let Some(tool) = tool {
        url.push_str(&format!("tool={tool}&"));
    }
    if let Some(model) = model {
        url.push_str(&format!("model={model}&"));
    }
    let agents: Vec<AgentView> = client
        .get(url)
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    if agents.is_empty() {
        println!("no matching agents");
        return Ok(());
    }
    println!("{:<38} {:<10} SKILLS", "AGENT", "STATUS");
    for agent in agents {
        println!(
            "{:<38} {:<10} {}",
            agent.agent_id,
            agent.status,
            agent.capabilities.skills.join(",")
        );
    }
    Ok(())
}

async fn marketplace_price(
    client: &reqwest::Client,
    base: &str,
    workload_type: &str,
    units: i64,
) -> anyhow::Result<()> {
    let estimate: PriceEstimate = client
        .get(format!(
            "{base}/api/v1/marketplace/price?workload_type={workload_type}&units={units}"
        ))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "{}: {} units x {} = {} credits",
        estimate.workload_type, estimate.units, estimate.unit_rate, estimate.estimated_credits
    );
    Ok(())
}

async fn agent_manifest(
    client: &reqwest::Client,
    base: &str,
    agent_id: &str,
) -> anyhow::Result<()> {
    let manifest: serde_json::Value = client
        .get(format!("{base}/api/v1/agents/{agent_id}/manifest"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}

async fn agent_delegations(
    client: &reqwest::Client,
    base: &str,
    agent_id: &str,
) -> anyhow::Result<()> {
    let delegations: Vec<DelegationView> = client
        .get(format!("{base}/api/v1/agents/{agent_id}/delegations"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    if delegations.is_empty() {
        println!("no delegations");
        return Ok(());
    }
    println!("{:<40} {:<10} {:<14} PARENT", "JOB", "ROLE", "STATE");
    for d in delegations {
        println!(
            "{:<40} {:<10} {:<14} {}",
            d.job_id,
            d.role,
            d.state,
            d.parent_job_id.as_ref().map(|p| p.as_str()).unwrap_or("-")
        );
    }
    Ok(())
}

async fn storage_put(
    client: &reqwest::Client,
    base: &str,
    file: &PathBuf,
    owner: &str,
    data_shards: Option<u32>,
    parity_shards: Option<u32>,
) -> anyhow::Result<()> {
    let data = std::fs::read(file).with_context(|| format!("could not read {}", file.display()))?;
    let request = UploadObjectRequest {
        owner_participant: mesh_protocol::ParticipantId::new(owner),
        data_base64: base64::engine::general_purpose::STANDARD.encode(&data),
        data_shards,
        parity_shards,
    };
    let object: StorageObjectView = client
        .post(format!("{base}/api/v1/storage/objects"))
        .json(&request)
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "uploaded {} ({} bytes, {}+{} shards)",
        object.object_id, object.size_bytes, object.data_shards, object.parity_shards
    );
    for shard in &object.shards {
        println!("  shard {} -> {}", shard.shard_index, shard.node_id);
    }
    Ok(())
}

async fn storage_get(
    client: &reqwest::Client,
    base: &str,
    object_id: &str,
    out: &PathBuf,
) -> anyhow::Result<()> {
    let response: DownloadObjectResponse = client
        .get(format!(
            "{base}/api/v1/storage/objects/{object_id}/download"
        ))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    let bytes =
        base64::engine::general_purpose::STANDARD.decode(response.data_base64.as_bytes())?;
    std::fs::write(out, &bytes).with_context(|| format!("could not write {}", out.display()))?;
    println!(
        "wrote {} bytes to {} (used {} shards)",
        bytes.len(),
        out.display(),
        response.shards_used
    );
    Ok(())
}

async fn storage_info(client: &reqwest::Client, base: &str, object_id: &str) -> anyhow::Result<()> {
    let object: StorageObjectView = client
        .get(format!("{base}/api/v1/storage/objects/{object_id}"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!("{}", serde_json::to_string_pretty(&object)?);
    Ok(())
}

async fn storage_repair(
    client: &reqwest::Client,
    base: &str,
    object_id: &str,
) -> anyhow::Result<()> {
    let response: RepairObjectResponse = client
        .post(format!("{base}/api/v1/storage/objects/{object_id}/repair"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "repaired {} shards; status {}",
        response.repaired, response.status
    );
    Ok(())
}

async fn storage_ls(client: &reqwest::Client, base: &str) -> anyhow::Result<()> {
    let objects: Vec<StorageObjectView> = client
        .get(format!("{base}/api/v1/storage/objects"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    if objects.is_empty() {
        println!("no objects");
        return Ok(());
    }
    println!("{:<40} {:<10} {:<9} STATUS", "OBJECT", "SIZE", "SHARDS");
    for object in objects {
        println!(
            "{:<40} {:<10} {:<9} {}",
            object.object_id,
            object.size_bytes,
            format!("{}+{}", object.data_shards, object.parity_shards),
            object.status
        );
    }
    Ok(())
}

async fn abuse_report(
    client: &reqwest::Client,
    base: &str,
    subject_type: &str,
    subject: &str,
    category: &str,
    detail: Option<&str>,
) -> anyhow::Result<()> {
    let request = AbuseReportRequest {
        reporter_participant: None,
        subject_type: subject_type.to_string(),
        subject_id: subject.to_string(),
        category: category.to_string(),
        detail: detail.map(|d| d.to_string()),
    };
    let response: AbuseReportResponse = client
        .post(format!("{base}/api/v1/abuse/reports"))
        .json(&request)
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "reported {} {} ({} reports, quarantined={})",
        response.subject_type, response.subject_id, response.report_count, response.quarantined
    );
    Ok(())
}

async fn abuse_ls(client: &reqwest::Client, base: &str) -> anyhow::Result<()> {
    let reports: serde_json::Value = client
        .get(format!("{base}/api/v1/abuse/reports"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .json()
        .await?;
    println!("{}", serde_json::to_string_pretty(&reports)?);
    Ok(())
}

async fn metrics(client: &reqwest::Client, base: &str) -> anyhow::Result<()> {
    let body = client
        .get(format!("{base}/api/v1/metrics"))
        .send()
        .await
        .context("could not reach control plane")?
        .error_for_status()?
        .text()
        .await?;
    print!("{body}");
    Ok(())
}
