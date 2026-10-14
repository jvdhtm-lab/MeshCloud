//! Settlement: turning accepted contribution receipts into settled rewards.
//!
//! Settlement is replaceable behind the [`Settlement`] trait. The
//! [`LocalLedger`] backend records settlement in Postgres; the optional `evm`
//! backend submits Merkle roots to an on-chain contract via alloy. Both share
//! the same batching and idempotency guarantees: a receipt belongs to at most
//! one batch, so rewards can never be settled twice.

#![forbid(unsafe_code)]

pub mod merkle;

#[cfg(feature = "evm")]
pub mod evm;

use sqlx::{PgPool, Row};
use uuid::Uuid;

use mesh_protocol::BatchView;

/// Errors raised by the settlement layer.
#[derive(Debug, thiserror::Error)]
pub enum SettlementError {
    /// A database error occurred.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// The settlement backend failed.
    #[error("settlement backend error: {0}")]
    Backend(String),
    /// The settlement backend is misconfigured.
    #[error("settlement configuration error: {0}")]
    Config(String),
}

/// One receipt inside a settlement batch.
#[derive(Debug, Clone)]
pub struct BatchEntry {
    /// Receipt identifier.
    pub receipt_id: String,
    /// Participant credited.
    pub participant_id: String,
    /// Participant wallet address, if registered.
    pub wallet_address: Option<String>,
    /// Reward amount.
    pub amount: i64,
}

/// A batch of receipts ready to settle.
#[derive(Debug, Clone)]
pub struct SettlementBatch {
    /// Batch identifier.
    pub batch_id: String,
    /// Chain the batch targets (0 for the local ledger).
    pub chain_id: i64,
    /// Merkle root over the receipt leaves.
    pub merkle_root: [u8; 32],
    /// Sum of reward amounts.
    pub total_reward: i64,
    /// Receipt entries.
    pub entries: Vec<BatchEntry>,
}

/// The result of settling a batch.
#[derive(Debug, Clone)]
pub struct SettlementOutcome {
    /// Batch identifier.
    pub batch_id: String,
    /// Backend transaction hash, if any.
    pub tx_hash: Option<String>,
}

/// A replaceable settlement backend.
#[allow(async_fn_in_trait)]
pub trait Settlement: Send + Sync {
    /// Settle all currently settleable receipts, up to `max_receipts`. Returns
    /// the number of receipts settled. Safe to call repeatedly.
    async fn reconcile(&self, pool: &PgPool, max_receipts: usize)
        -> Result<usize, SettlementError>;
}

/// The local-ledger backend: settlement is recorded in Postgres only.
pub struct LocalLedger {
    /// Chain id recorded on batches (0 for the local ledger).
    pub chain_id: i64,
}

impl LocalLedger {
    /// Create a local-ledger backend.
    pub fn new(chain_id: i64) -> Self {
        Self { chain_id }
    }
}

impl Settlement for LocalLedger {
    async fn reconcile(
        &self,
        pool: &PgPool,
        max_receipts: usize,
    ) -> Result<usize, SettlementError> {
        requeue_stale_batches(pool, 300).await?;
        let Some(batch) = claim_batch(pool, max_receipts, self.chain_id).await? else {
            return Ok(0);
        };
        let tx_hash = format!("local-{}", batch.batch_id);
        confirm_batch(pool, &batch.batch_id, Some(&tx_hash)).await?;
        Ok(batch.entries.len())
    }
}

/// Atomically claim up to `max_receipts` issued receipts into a new batch.
pub async fn claim_batch(
    pool: &PgPool,
    max_receipts: usize,
    chain_id: i64,
) -> Result<Option<SettlementBatch>, SettlementError> {
    let mut tx = pool.begin().await?;

    let rows = sqlx::query(
        "SELECT r.receipt_id, r.participant_id, r.reward_amount, p.wallet_address \
         FROM contribution_receipts r \
         JOIN participants p ON p.participant_id = r.participant_id \
         WHERE r.status = 'issued' \
         ORDER BY r.created_at \
         LIMIT $1 \
         FOR UPDATE OF r SKIP LOCKED",
    )
    .bind(max_receipts as i64)
    .fetch_all(&mut *tx)
    .await?;

    if rows.is_empty() {
        tx.commit().await?;
        return Ok(None);
    }

    let mut entries = Vec::with_capacity(rows.len());
    let mut leaves = Vec::with_capacity(rows.len());
    let mut total_reward = 0i64;
    for row in rows {
        let receipt_id: String = row.try_get("receipt_id")?;
        let participant_id: String = row.try_get("participant_id")?;
        let amount: i64 = row.try_get("reward_amount")?;
        let wallet_address: Option<String> = row.try_get("wallet_address")?;
        let recipient = wallet_address
            .as_deref()
            .and_then(merkle::address_bytes)
            .unwrap_or([0u8; 20]);
        leaves.push(merkle::leaf_hash(&receipt_id, &recipient, amount));
        total_reward = total_reward.saturating_add(amount);
        entries.push(BatchEntry {
            receipt_id,
            participant_id,
            wallet_address,
            amount,
        });
    }

    let merkle_root = merkle::merkle_root(&leaves);
    let batch_id = format!("batch-{}", Uuid::new_v4());

    sqlx::query(
        "INSERT INTO reward_batches \
             (batch_id, merkle_root, total_reward, receipt_count, chain_id, status) \
         VALUES ($1, $2, $3, $4, $5, 'submitted')",
    )
    .bind(&batch_id)
    .bind(hex::encode(merkle_root))
    .bind(total_reward)
    .bind(entries.len() as i32)
    .bind(chain_id)
    .execute(&mut *tx)
    .await?;

    for entry in &entries {
        sqlx::query(
            "INSERT INTO batch_receipts (batch_id, receipt_id) VALUES ($1, $2) \
             ON CONFLICT (receipt_id) DO NOTHING",
        )
        .bind(&batch_id)
        .bind(&entry.receipt_id)
        .execute(&mut *tx)
        .await?;
    }

    let receipt_ids: Vec<String> = entries.iter().map(|e| e.receipt_id.clone()).collect();
    sqlx::query("UPDATE contribution_receipts SET status = 'batched' WHERE receipt_id = ANY($1)")
        .bind(&receipt_ids)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT INTO settlements (settlement_id, batch_id, chain_id, status) \
         VALUES ($1, $2, $3, 'submitted')",
    )
    .bind(format!("stl-{}", Uuid::new_v4()))
    .bind(&batch_id)
    .bind(chain_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Some(SettlementBatch {
        batch_id,
        chain_id,
        merkle_root,
        total_reward,
        entries,
    }))
}

/// Mark a claimed batch settled.
pub async fn confirm_batch(
    pool: &PgPool,
    batch_id: &str,
    tx_hash: Option<&str>,
) -> Result<(), SettlementError> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "UPDATE reward_batches SET status = 'settled', tx_hash = $2, settled_at = now() \
         WHERE batch_id = $1",
    )
    .bind(batch_id)
    .bind(tx_hash)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE contribution_receipts SET status = 'settled' \
         WHERE receipt_id IN (SELECT receipt_id FROM batch_receipts WHERE batch_id = $1)",
    )
    .bind(batch_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("UPDATE settlements SET status = 'settled', tx_hash = $2 WHERE batch_id = $1")
        .bind(batch_id)
        .bind(tx_hash)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Mark a claimed batch failed and return its receipts to the queue.
pub async fn fail_batch(pool: &PgPool, batch_id: &str) -> Result<(), SettlementError> {
    let mut tx = pool.begin().await?;

    sqlx::query("UPDATE reward_batches SET status = 'failed' WHERE batch_id = $1")
        .bind(batch_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "UPDATE contribution_receipts SET status = 'issued' \
         WHERE status = 'batched' \
           AND receipt_id IN (SELECT receipt_id FROM batch_receipts WHERE batch_id = $1)",
    )
    .bind(batch_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("UPDATE settlements SET status = 'failed' WHERE batch_id = $1")
        .bind(batch_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Fail batches that were claimed but never confirmed, returning their receipts
/// to the queue. Returns the number of batches requeued.
pub async fn requeue_stale_batches(
    pool: &PgPool,
    older_than_seconds: i64,
) -> Result<u64, SettlementError> {
    let stale: Vec<String> = sqlx::query_scalar(
        "SELECT batch_id FROM reward_batches \
         WHERE status = 'submitted' \
           AND created_at < now() - ($1::bigint * interval '1 second')",
    )
    .bind(older_than_seconds)
    .fetch_all(pool)
    .await?;

    let mut requeued = 0u64;
    for batch_id in stale {
        fail_batch(pool, &batch_id).await?;
        requeued += 1;
    }
    Ok(requeued)
}

/// List recent reward batches, newest first.
pub async fn list_batches(pool: &PgPool, limit: i64) -> Result<Vec<BatchView>, SettlementError> {
    let rows = sqlx::query(
        "SELECT batch_id, merkle_root, total_reward, receipt_count, chain_id, status, \
             tx_hash, created_at, settled_at \
         FROM reward_batches ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let mut batches = Vec::with_capacity(rows.len());
    for row in rows {
        batches.push(BatchView {
            batch_id: row.try_get("batch_id")?,
            merkle_root: row.try_get("merkle_root")?,
            total_reward: row.try_get("total_reward")?,
            receipt_count: row.try_get::<i32, _>("receipt_count")? as u32,
            chain_id: row.try_get("chain_id")?,
            status: row.try_get("status")?,
            tx_hash: row.try_get("tx_hash")?,
            created_at: row.try_get("created_at")?,
            settled_at: row.try_get("settled_at")?,
        });
    }
    Ok(batches)
}
