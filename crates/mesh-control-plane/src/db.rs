//! Database connectivity and migrations.

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::config::Config;

/// Connect to PostgreSQL and return a pool.
pub async fn connect(config: &Config) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database_url)
        .await?;
    Ok(pool)
}

/// Create a lazy pool that does not connect until first use.
///
/// Useful for tests and health checks that must build a router without a live
/// database.
pub fn lazy_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy(database_url)
        .expect("valid database URL")
}

/// Run embedded migrations. Embedded at compile time so a clean checkout needs
/// no extra files on disk.
pub async fn run_migrations(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("../../migrations").run(pool).await?;
    Ok(())
}

/// Mark any online node that has not heartbeated within `timeout_seconds` as
/// offline, and mark the agents hosted by offline nodes offline with it. Returns
/// the number of nodes updated.
pub async fn mark_stale_nodes_offline(pool: &PgPool, timeout_seconds: u64) -> anyhow::Result<u64> {
    let result = sqlx::query(
        "UPDATE nodes SET status = 'offline', updated_at = now() \
         WHERE status = 'online' \
           AND (last_heartbeat IS NULL \
                OR last_heartbeat < now() - ($1::bigint * interval '1 second'))",
    )
    .bind(timeout_seconds as i64)
    .execute(pool)
    .await?;

    // Agents are only as reachable as the worker that hosts them.
    sqlx::query(
        "UPDATE agents SET status = 'offline', updated_at = now() \
         WHERE status = 'online' \
           AND node_id IN (SELECT node_id FROM nodes WHERE status = 'offline')",
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}
