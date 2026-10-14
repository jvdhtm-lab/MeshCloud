//! Control-plane binary entry point.

use std::time::Duration;

use anyhow::Context;
use mesh_control_plane::{build_router, config::Config, db, telemetry, AppState};
#[cfg(feature = "libp2p")]
use mesh_network::MeshTransport;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load().context("failed to load configuration")?;
    telemetry::init(&config.log);

    tracing::info!(
        bind_addr = %config.bind_addr,
        database = %redact(&config.database_url),
        "starting mesh-control-plane"
    );

    let pool = connect_with_retry(&config).await?;
    db::run_migrations(&pool)
        .await
        .context("failed to run database migrations")?;
    tracing::info!("database migrations applied");

    let role = config
        .replica_role
        .parse::<mesh_network::ReplicaRole>()
        .unwrap_or(mesh_network::ReplicaRole::Primary);

    if role.runs_writers() {
        spawn_offline_sweeper(pool.clone(), config.node_offline_after_seconds);
        spawn_job_sweeper(pool.clone(), config.job_sweep_interval_seconds);
        spawn_settlement_worker(pool.clone(), config.clone());
        tracing::info!(role = %role, "background writers enabled");
    } else {
        tracing::info!(role = %role, "replica: background writers disabled (single-writer model)");
    }

    #[allow(unused_mut)]
    let mut state = AppState::new(pool, config.clone());

    #[cfg(feature = "libp2p")]
    if let Some(listen) = config.network_listen.clone() {
        match start_mesh(&listen, &config.network_topic).await {
            Ok(network) => {
                spawn_mesh_ingest(&network, state.registry.clone());
                spawn_registry_pruner(state.registry.clone());
                state = state.with_network(std::sync::Arc::new(network));
                tracing::info!(listen = %listen, "mesh networking enabled");
            }
            Err(err) => tracing::warn!(error = %err, "mesh networking disabled"),
        }
    }
    #[cfg(not(feature = "libp2p"))]
    {
        spawn_registry_pruner(state.registry.clone());
        if config.network_listen.is_some() {
            tracing::warn!("network_listen set but the `libp2p` feature is not enabled");
        }
    }

    let app = build_router(state.clone());

    if role.runs_writers() {
        spawn_storage_sweeper(state, config.storage_challenge_interval_seconds);
    }

    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .with_context(|| format!("failed to bind {}", config.bind_addr))?;
    tracing::info!(addr = %config.bind_addr, "control plane listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    tracing::info!("control plane stopped");
    Ok(())
}

async fn connect_with_retry(config: &Config) -> anyhow::Result<sqlx::PgPool> {
    let mut delay = Duration::from_millis(500);
    for attempt in 1..=10 {
        match db::connect(config).await {
            Ok(pool) => return Ok(pool),
            Err(err) => {
                tracing::warn!(attempt, error = %err, "database not ready, retrying");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(10));
            }
        }
    }
    db::connect(config)
        .await
        .context("could not connect to database after retries")
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %err, "failed to listen for shutdown signal");
    }
    tracing::info!("shutdown signal received");
}

/// Periodically mark nodes offline once they stop heartbeating.
fn spawn_offline_sweeper(pool: sqlx::PgPool, timeout_seconds: u64) {
    let period = Duration::from_secs((timeout_seconds / 3).max(5));
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(period);
        loop {
            ticker.tick().await;
            match db::mark_stale_nodes_offline(&pool, timeout_seconds).await {
                Ok(0) => {}
                Ok(count) => tracing::info!(count, "marked stale nodes offline"),
                Err(err) => tracing::warn!(error = %err, "offline sweep failed"),
            }
        }
    });
}

/// Periodically settle accepted contribution receipts.
fn spawn_settlement_worker(pool: sqlx::PgPool, config: Config) {
    let period = Duration::from_secs(config.settlement_interval_seconds.max(1));
    let batch_size = config.settlement_batch_size as usize;
    let chain_id = config.settlement_chain_id;

    tokio::spawn(async move {
        if config.settlement_backend.eq_ignore_ascii_case("evm") {
            #[cfg(feature = "evm")]
            {
                match (
                    config.settlement_rpc_url.clone(),
                    config.settlement_contract_address.clone(),
                    config.settlement_issuer_key.clone(),
                ) {
                    (Some(rpc_url), Some(contract_address), Some(issuer_key)) => {
                        tracing::info!(chain_id, "using EVM settlement backend");
                        let backend = mesh_settlement::evm::EvmSettlement {
                            rpc_url,
                            contract_address,
                            issuer_key,
                            chain_id,
                        };
                        run_settler(backend, pool, batch_size, period).await;
                        return;
                    }
                    _ => tracing::warn!(
                        "EVM settlement selected but rpc_url/contract_address/issuer_key is missing; \
                         falling back to the local ledger"
                    ),
                }
            }
            #[cfg(not(feature = "evm"))]
            tracing::warn!(
                "EVM settlement selected but the `evm` feature is not enabled; using the local ledger"
            );
        }

        run_settler(
            mesh_settlement::LocalLedger::new(chain_id),
            pool,
            batch_size,
            period,
        )
        .await;
    });
}

async fn run_settler<S: mesh_settlement::Settlement>(
    settler: S,
    pool: sqlx::PgPool,
    batch_size: usize,
    period: Duration,
) {
    let mut ticker = tokio::time::interval(period);
    loop {
        ticker.tick().await;
        match settler.reconcile(&pool, batch_size).await {
            Ok(0) => {}
            Ok(count) => tracing::info!(count, "settled contribution receipts"),
            Err(err) => tracing::warn!(error = %err, "settlement reconciliation failed"),
        }
    }
}

#[cfg(feature = "libp2p")]
async fn start_mesh(listen: &str, topic: &str) -> anyhow::Result<mesh_network::Libp2pTransport> {
    let address: mesh_network::libp2p_transport::Multiaddr =
        listen.parse().context("invalid network listen address")?;
    mesh_network::libp2p_transport::start_random(vec![address], vec![], topic)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(feature = "libp2p")]
fn spawn_mesh_ingest(
    network: &mesh_network::Libp2pTransport,
    registry: std::sync::Arc<mesh_network::DiscoveryRegistry>,
) {
    let mut receiver = network.advertisements();
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(signed) => match mesh_network::verify_advertisement(&signed) {
                    Ok(()) => registry.upsert(signed),
                    Err(err) => tracing::warn!(error = %err, "rejected advertisement"),
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "advertisement stream lagged")
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn spawn_storage_sweeper(state: AppState, interval_seconds: u64) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(interval_seconds.max(5)));
        loop {
            ticker.tick().await;
            let ids: Result<Vec<String>, sqlx::Error> =
                sqlx::query_scalar("SELECT object_id FROM storage_objects")
                    .fetch_all(&state.pool)
                    .await;
            match ids {
                Ok(ids) => {
                    for id in ids {
                        if let Err(err) =
                            mesh_control_plane::routes::storage::challenge_object_impl(&state, &id)
                                .await
                        {
                            tracing::warn!(object_id = %id, error = ?err, "storage challenge failed");
                        }
                    }
                }
                Err(err) => tracing::warn!(error = %err, "storage sweep failed"),
            }
        }
    });
}

fn spawn_registry_pruner(registry: std::sync::Arc<mesh_network::DiscoveryRegistry>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(30));
        loop {
            ticker.tick().await;
            let removed = registry.prune_expired(chrono::Utc::now());
            if removed > 0 {
                tracing::info!(removed, "pruned expired advertisements");
            }
        }
    });
}

fn spawn_job_sweeper(pool: sqlx::PgPool, interval_seconds: u64) {
    let period = Duration::from_secs(interval_seconds.max(1));
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(period);
        loop {
            ticker.tick().await;
            match mesh_scheduler::requeue_expired_leases(&pool).await {
                Ok(n) if n > 0 => tracing::info!(count = n, "requeued jobs with expired leases"),
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "job lease sweep failed"),
            }
            match mesh_scheduler::expire_overdue_jobs(&pool).await {
                Ok(n) if n > 0 => tracing::info!(count = n, "expired overdue jobs"),
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "job expiry sweep failed"),
            }
            match mesh_accounting::expire_reservations(&pool).await {
                Ok(n) if n > 0 => tracing::info!(count = n, "expired reservations"),
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "reservation expiry sweep failed"),
            }
        }
    });
}

/// Hide credentials when logging a connection string.
fn redact(database_url: &str) -> String {
    match url::Url::parse(database_url) {
        Ok(mut url) => {
            if url.password().is_some() {
                let _ = url.set_password(Some("***"));
            }
            url.to_string()
        }
        Err(_) => "<unparseable database url>".to_string(),
    }
}
