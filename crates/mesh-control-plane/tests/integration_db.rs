//! End-to-end tests against a real PostgreSQL, run via testcontainers.
//!
//! These are gated behind the `integration-tests` feature and marked `#[ignore]`
//! so `cargo test` never requires Docker. Run them with:
//!
//!   cargo test -p mesh-control-plane --features integration-tests -- --ignored
#![cfg(feature = "integration-tests")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration as ChronoDuration, Utc};
use mesh_control_plane::{build_router, db, AppState, Config};
use mesh_identity::NodeIdentity;
use mesh_protocol::offers::{Availability, ResourceLimits, ResourceOffer, SignedResourceOffer};
use mesh_protocol::status::JobState;
use mesh_protocol::{
    AgentId, AttemptId, ClaimedJobResponse, CompleteJobRequest, HeartbeatRequest,
    HeartbeatResponse, NodeCapabilities, NodeId, ParticipantId, RegisterNodeRequest,
    RegisterNodeResponse,
};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tower::ServiceExt;

async fn start_postgres() -> (testcontainers::ContainerAsync<Postgres>, String) {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    (container, url)
}

fn caps() -> NodeCapabilities {
    NodeCapabilities {
        cpu_cores: 4,
        ram_gb: 16,
        storage_gb: 100,
        gpu: false,
    }
}

fn signed_offer(identity: &NodeIdentity, node_id: &NodeId, nonce: u64) -> SignedResourceOffer {
    let offer = ResourceOffer {
        node_id: node_id.clone(),
        capabilities: caps(),
        limits: ResourceLimits {
            max_cpu_percent: 50,
            max_memory_mb: 4096,
        },
        availability: Availability::WhenIdle,
        expires_at: Utc::now() + ChronoDuration::seconds(120),
        nonce,
    };
    let signature = identity.sign_canonical(&offer);
    SignedResourceOffer {
        offer,
        public_key: identity.public_key_hex(),
        signature,
    }
}

async fn post_json(app: axum::Router, uri: &str, body: Vec<u8>) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires a running Docker daemon"]
async fn migrations_apply_register_heartbeat_and_list() {
    let (_container, url) = start_postgres().await;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    db::run_migrations(&pool).await.unwrap();

    let config = Config {
        database_url: url.clone(),
        ..Config::default()
    };
    let app = build_router(AppState::new(pool, config));

    let node_id = NodeId::new("node-it-1");
    let identity = NodeIdentity::generate(node_id.clone());

    let register = RegisterNodeRequest {
        node_id: node_id.clone(),
        participant_id: ParticipantId::new("ptc-it-1"),
        public_key: identity.public_key_hex(),
        offer: signed_offer(&identity, &node_id, 1),
    };

    let response = post_json(
        app.clone(),
        "/api/v1/nodes/register",
        serde_json::to_vec(&register).unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let registered: RegisterNodeResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(registered.node_id.as_str(), "node-it-1");

    // A heartbeat with a fresh signed offer keeps the node online.
    let heartbeat = HeartbeatRequest {
        offer: signed_offer(&identity, &node_id, 2),
    };
    let response = post_json(
        app.clone(),
        "/api/v1/nodes/node-it-1/heartbeat",
        serde_json::to_vec(&heartbeat).unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let beat: HeartbeatResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(beat.status, mesh_protocol::NodeStatus::Online);

    // The registry lists the node.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/nodes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let nodes: Vec<mesh_protocol::NodeView> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].node_id.as_str(), "node-it-1");
    assert_eq!(nodes[0].capabilities.cpu_cores, 4);
}

#[tokio::test]
#[ignore = "requires a running Docker daemon"]
async fn tampered_offer_is_rejected() {
    let (_container, url) = start_postgres().await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    db::run_migrations(&pool).await.unwrap();

    let config = Config {
        database_url: url.clone(),
        ..Config::default()
    };
    let app = build_router(AppState::new(pool, config));

    let node_id = NodeId::new("node-tamper");
    let identity = NodeIdentity::generate(node_id.clone());
    let mut offer = signed_offer(&identity, &node_id, 1);
    // Corrupt the advertised capabilities after signing.
    offer.offer.capabilities.cpu_cores = 999;

    let register = RegisterNodeRequest {
        node_id: node_id.clone(),
        participant_id: ParticipantId::new("ptc-tamper"),
        public_key: identity.public_key_hex(),
        offer,
    };

    let response = post_json(
        app,
        "/api/v1/nodes/register",
        serde_json::to_vec(&register).unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires a running Docker daemon"]
async fn offline_sweep_marks_only_stale_nodes() {
    let (_container, url) = start_postgres().await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    db::run_migrations(&pool).await.unwrap();

    let caps = serde_json::to_value(caps()).unwrap();

    sqlx::query(
        "INSERT INTO participants (participant_id, public_key) VALUES ('ptc-sweep', 'key')",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO nodes (node_id, participant_id, public_key, capabilities, status, last_heartbeat) \
         VALUES ('node-fresh', 'ptc-sweep', 'key', $1, 'online', now()), \
                ('node-stale', 'ptc-sweep', 'key', $1, 'online', now() - interval '1 hour')",
    )
    .bind(&caps)
    .execute(&pool)
    .await
    .unwrap();

    let updated = db::mark_stale_nodes_offline(&pool, 60).await.unwrap();
    assert_eq!(updated, 1);

    let fresh: String = sqlx::query_scalar("SELECT status FROM nodes WHERE node_id = 'node-fresh'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let stale: String = sqlx::query_scalar("SELECT status FROM nodes WHERE node_id = 'node-stale'")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(fresh, "online");
    assert_eq!(stale, "offline");
}

async fn seed_online_node(pool: &sqlx::PgPool, node_id: &str) {
    let caps = serde_json::to_value(caps()).unwrap();
    sqlx::query(
        "INSERT INTO participants (participant_id, public_key) VALUES ('ptc-jobs', 'key') \
         ON CONFLICT (participant_id) DO NOTHING",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nodes (node_id, participant_id, public_key, capabilities, status, last_heartbeat) \
         VALUES ($1, 'ptc-jobs', 'key', $2, 'online', now()) \
         ON CONFLICT (node_id) DO UPDATE SET status = 'online', capabilities = EXCLUDED.capabilities",
    )
    .bind(node_id)
    .bind(&caps)
    .execute(pool)
    .await
    .unwrap();
}

fn worker_context(node_id: &str) -> mesh_scheduler::WorkerContext {
    mesh_scheduler::WorkerContext {
        worker_id: NodeId::new(node_id),
        cpu_cores: 4,
        ram_gb: 16,
        supported_workloads: vec!["echo".to_string()],
    }
}

fn echo_job(max_attempts: u32) -> mesh_scheduler::NewJob {
    mesh_scheduler::NewJob {
        workload_type: "echo".to_string(),
        input: serde_json::json!({"n": 1}),
        required_cpu_cores: 1,
        required_ram_gb: 1,
        max_attempts,
        deadline_seconds: 600,
        ..Default::default()
    }
}

async fn migrated_pool(url: &str) -> sqlx::PgPool {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .unwrap();
    db::run_migrations(&pool).await.unwrap();
    pool
}

#[tokio::test]
#[ignore = "requires a running Docker daemon"]
async fn job_claim_and_complete_is_exclusive_and_idempotent() {
    let (_container, url) = start_postgres().await;
    let pool = migrated_pool(&url).await;
    seed_online_node(&pool, "node-j1").await;
    let worker = worker_context("node-j1");

    let submitted = mesh_scheduler::submit_job(&pool, "tester", echo_job(2))
        .await
        .unwrap();

    let claim = mesh_scheduler::claim_job(
        &pool,
        &worker,
        60,
        &mesh_scheduler::ClaimFilter::worker_jobs(),
    )
    .await
    .unwrap()
    .expect("a job should be claimable");

    // A second claim finds nothing: the only job is already scheduled.
    assert!(mesh_scheduler::claim_job(
        &pool,
        &worker,
        60,
        &mesh_scheduler::ClaimFilter::worker_jobs()
    )
    .await
    .unwrap()
    .is_none());

    let done = mesh_scheduler::complete_job(
        &pool,
        &submitted.job_id,
        &claim.attempt_id,
        "result-hash",
        Some("done"),
    )
    .await
    .unwrap();
    assert_eq!(done.state, JobState::Succeeded);

    // Completing again is a no-op and does not add an attempt.
    let again = mesh_scheduler::complete_job(
        &pool,
        &submitted.job_id,
        &claim.attempt_id,
        "result-hash",
        None,
    )
    .await
    .unwrap();
    assert_eq!(again.state, JobState::Succeeded);

    let detail = mesh_scheduler::job_detail(&pool, &submitted.job_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.attempts.len(), 1);
    assert_eq!(detail.job.attempts, 1);
    assert_eq!(detail.job.state, JobState::Succeeded);
}

#[tokio::test]
#[ignore = "requires a running Docker daemon"]
async fn failed_attempt_retries_then_fails() {
    let (_container, url) = start_postgres().await;
    let pool = migrated_pool(&url).await;
    seed_online_node(&pool, "node-j2").await;
    let worker = worker_context("node-j2");

    let submitted = mesh_scheduler::submit_job(&pool, "tester", echo_job(2))
        .await
        .unwrap();

    let first = mesh_scheduler::claim_job(
        &pool,
        &worker,
        60,
        &mesh_scheduler::ClaimFilter::worker_jobs(),
    )
    .await
    .unwrap()
    .unwrap();
    let failed = mesh_scheduler::fail_job(&pool, &submitted.job_id, &first.attempt_id, "boom")
        .await
        .unwrap();
    assert!(failed.will_retry);
    assert_eq!(failed.state, JobState::Pending);

    let second = mesh_scheduler::claim_job(
        &pool,
        &worker,
        60,
        &mesh_scheduler::ClaimFilter::worker_jobs(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(second.attempt_number, 2);
    let failed = mesh_scheduler::fail_job(&pool, &submitted.job_id, &second.attempt_id, "boom")
        .await
        .unwrap();
    assert!(!failed.will_retry);
    assert_eq!(failed.state, JobState::Failed);
}

#[tokio::test]
#[ignore = "requires a running Docker daemon"]
async fn expired_lease_requeues_job() {
    let (_container, url) = start_postgres().await;
    let pool = migrated_pool(&url).await;
    seed_online_node(&pool, "node-j3").await;
    let worker = worker_context("node-j3");

    let submitted = mesh_scheduler::submit_job(&pool, "tester", echo_job(3))
        .await
        .unwrap();
    let claim = mesh_scheduler::claim_job(
        &pool,
        &worker,
        60,
        &mesh_scheduler::ClaimFilter::worker_jobs(),
    )
    .await
    .unwrap()
    .unwrap();

    // Simulate the worker vanishing: its lease is now in the past.
    sqlx::query("UPDATE job_attempts SET lease_expires_at = now() - interval '1 minute' WHERE attempt_id = $1")
        .bind(claim.attempt_id.as_str())
        .execute(&pool)
        .await
        .unwrap();

    let requeued = mesh_scheduler::requeue_expired_leases(&pool).await.unwrap();
    assert_eq!(requeued, 1);

    let detail = mesh_scheduler::job_detail(&pool, &submitted.job_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.job.state, JobState::Pending);
}

async fn claim_via_http(app: &axum::Router, worker: &str) -> Option<ClaimedJobResponse> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/workers/{worker}/jobs/next?workloads=echo"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    if response.status() == StatusCode::NO_CONTENT {
        return None;
    }
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    Some(serde_json::from_slice(&bytes).unwrap())
}

async fn complete_via_http(
    app: &axum::Router,
    job_id: &str,
    request: &CompleteJobRequest,
) -> StatusCode {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/jobs/{job_id}/complete"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    response.status()
}

async fn receipt_count(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM contribution_receipts")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires a running Docker daemon"]
async fn verification_gates_reward_and_is_idempotent() {
    let (_container, url) = start_postgres().await;
    let pool = migrated_pool(&url).await;
    seed_online_node(&pool, "node-v1").await;
    let config = Config {
        database_url: url.clone(),
        ..Config::default()
    };
    let app = build_router(AppState::new(pool.clone(), config));

    let participant = ParticipantId::new("ptc-jobs");
    let verify_config = mesh_verifier::VerifyConfig::default();

    // --- invalid result earns nothing and fails the job ---
    let bad = mesh_scheduler::submit_job(&pool, "tester", echo_job(1))
        .await
        .unwrap();
    let claim = claim_via_http(&app, "node-v1").await.unwrap();
    assert_eq!(claim.job_id, bad.job_id);
    let status = complete_via_http(
        &app,
        bad.job_id.as_str(),
        &CompleteJobRequest {
            attempt_id: claim.attempt_id.clone(),
            result_hash: "not-the-right-hash".to_string(),
            log: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let detail = mesh_scheduler::job_detail(&pool, &bad.job_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.job.state, JobState::Failed);
    assert_eq!(receipt_count(&pool).await, 0);
    assert!(mesh_accounting::participant_balance(&pool, &participant)
        .await
        .unwrap()
        .is_none());

    // --- a reproducible result earns exactly one receipt and credit ---
    let good = mesh_scheduler::submit_job(&pool, "tester", echo_job(2))
        .await
        .unwrap();
    let claim = claim_via_http(&app, "node-v1").await.unwrap();
    assert_eq!(claim.job_id, good.job_id);
    let correct =
        mesh_verifier::expected_hash("echo", &claim.input, &verify_config).expect("echo hash");
    let status = complete_via_http(
        &app,
        good.job_id.as_str(),
        &CompleteJobRequest {
            attempt_id: claim.attempt_id.clone(),
            result_hash: correct.clone(),
            log: Some("verified".to_string()),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(receipt_count(&pool).await, 1);
    let balance = mesh_accounting::participant_balance(&pool, &participant)
        .await
        .unwrap()
        .expect("balance exists");
    assert_eq!(balance.earned, 1);
    assert_eq!(balance.available, 1);

    // --- replaying the completion does not double-credit ---
    let status = complete_via_http(
        &app,
        good.job_id.as_str(),
        &CompleteJobRequest {
            attempt_id: claim.attempt_id.clone(),
            result_hash: correct,
            log: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(receipt_count(&pool).await, 1);
    let balance = mesh_accounting::participant_balance(&pool, &participant)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(balance.earned, 1);
}

#[tokio::test]
#[ignore = "requires a running Docker daemon"]
async fn local_settlement_is_idempotent() {
    let (_container, url) = start_postgres().await;
    let pool = migrated_pool(&url).await;
    seed_online_node(&pool, "node-s1").await;

    // Give the participant a wallet address so a leaf can be built.
    sqlx::query("UPDATE participants SET wallet_address = $1 WHERE participant_id = 'ptc-jobs'")
        .bind("0x00000000000000000000000000000000000000aa")
        .execute(&pool)
        .await
        .unwrap();

    let job = mesh_scheduler::submit_job(&pool, "tester", echo_job(1))
        .await
        .unwrap();

    let participant = ParticipantId::new("ptc-jobs");
    let policy = mesh_accounting::RewardPolicy {
        version: 1,
        rate_per_unit: 1,
        delegator_rate_per_unit: 1,
    };
    let contribution = mesh_accounting::ContributionInput {
        job_id: Some(job.job_id.clone()),
        root_job_id: None,
        attempt_id: AttemptId::new("att-settle-1"),
        participant_id: participant.clone(),
        node_id: NodeId::new("node-s1"),
        agent_id: None,
        role: "worker".to_string(),
        workload_type: "echo".to_string(),
        accepted_units: 3,
        verification_method: "echo-deterministic".to_string(),
        evidence_hash: "abc".to_string(),
        policy_version: 1,
    };
    let recorded = mesh_accounting::record_contribution(&pool, &policy, &contribution)
        .await
        .unwrap();
    assert!(recorded.is_some());

    // Settle once.
    let ledger = mesh_settlement::LocalLedger::new(0);
    let settled = mesh_settlement::Settlement::reconcile(&ledger, &pool, 100)
        .await
        .unwrap();
    assert_eq!(settled, 1);

    let status: String = sqlx::query_scalar(
        "SELECT status FROM contribution_receipts WHERE attempt_id = 'att-settle-1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "settled");
    let batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reward_batches")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(batches, 1);

    // Settling again does nothing: no double settlement.
    let settled_again = mesh_settlement::Settlement::reconcile(&ledger, &pool, 100)
        .await
        .unwrap();
    assert_eq!(settled_again, 0);
    let batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reward_batches")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(batches, 1);
}

fn signed_agent_manifest(
    agent: &mesh_identity::AgentIdentity,
    node: &NodeId,
    nonce: u64,
) -> mesh_agent_protocol::SignedAgentManifest {
    use mesh_agent_protocol::{
        AgentCapabilities, AgentManifest, AgentPolicy, SignedAgentManifest, AGENT_PROTOCOL_VERSION,
    };
    let manifest = AgentManifest {
        agent_id: agent.agent_id().clone(),
        node_id: node.clone(),
        protocol_version: AGENT_PROTOCOL_VERSION,
        capabilities: AgentCapabilities {
            skills: vec!["deterministic".to_string()],
            tools: vec!["echo".to_string()],
            models: vec!["noop-deterministic".to_string()],
            max_concurrent_tasks: 2,
            max_depth_supported: 2,
        },
        policy: AgentPolicy {
            accepts_delegation: true,
            min_budget_credits: 1,
            max_task_seconds: 300,
        },
        expires_at: Utc::now() + ChronoDuration::seconds(120),
        nonce,
    };
    let signature = agent.sign_canonical(&manifest);
    SignedAgentManifest {
        manifest,
        public_key: agent.public_key_hex(),
        signature,
    }
}

async fn get_agents(app: &axum::Router) -> Vec<mesh_agent_protocol::AgentView> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
#[ignore = "requires a running Docker daemon"]
async fn agents_register_independently_and_follow_their_node() {
    let (_container, url) = start_postgres().await;
    let pool = migrated_pool(&url).await;
    seed_online_node(&pool, "node-a").await;
    seed_online_node(&pool, "node-b").await;
    let config = Config {
        database_url: url.clone(),
        ..Config::default()
    };
    let app = build_router(AppState::new(pool.clone(), config));

    let node_a = NodeId::new("node-a");
    let node_b = NodeId::new("node-b");
    let agent_a = mesh_identity::AgentIdentity::generate(AgentId::new("agent-a"));
    let agent_b = mesh_identity::AgentIdentity::generate(AgentId::new("agent-b"));

    for (node, agent) in [(&node_a, &agent_a), (&node_b, &agent_b)] {
        let request = mesh_agent_protocol::RegisterAgentRequest {
            node_id: node.clone(),
            manifest: signed_agent_manifest(agent, node, 1),
        };
        let response = post_json(
            app.clone(),
            "/api/v1/agents/register",
            serde_json::to_vec(&request).unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    let agents = get_agents(&app).await;
    assert_eq!(agents.len(), 2);
    assert!(agents
        .iter()
        .all(|a| a.status == mesh_agent_protocol::AgentStatus::Online));

    // A tampered manifest (changed after signing) is rejected.
    let mut tampered = signed_agent_manifest(&agent_b, &node_b, 2);
    tampered.manifest.capabilities.max_concurrent_tasks = 99;
    let request = mesh_agent_protocol::RegisterAgentRequest {
        node_id: node_b.clone(),
        manifest: tampered,
    };
    let response = post_json(
        app.clone(),
        "/api/v1/agents/register",
        serde_json::to_vec(&request).unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Take node-a offline; only its agent should go offline.
    sqlx::query("UPDATE nodes SET last_heartbeat = now() - interval '1 hour' WHERE node_id = $1")
        .bind(node_a.as_str())
        .execute(&pool)
        .await
        .unwrap();
    let updated = db::mark_stale_nodes_offline(&pool, 60).await.unwrap();
    assert_eq!(updated, 1);

    let agents = get_agents(&app).await;
    let a = agents
        .iter()
        .find(|x| x.agent_id.as_str() == "agent-a")
        .unwrap();
    let b = agents
        .iter()
        .find(|x| x.agent_id.as_str() == "agent-b")
        .unwrap();
    assert_eq!(a.status, mesh_agent_protocol::AgentStatus::Offline);
    assert_eq!(b.status, mesh_agent_protocol::AgentStatus::Online);
}
