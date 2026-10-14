//! On-chain settlement via `alloy` and the `MeshReward` contract.
//!
//! Only the Merkle root of a batch is submitted on-chain (signed by the
//! authorized issuer). Individual receipts are later claimed with a Merkle
//! proof. The contract prevents duplicate claims.

use std::str::FromStr;

use alloy::network::EthereumWallet;
use alloy::primitives::{Address, B256, U256};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use sqlx::PgPool;

use crate::{
    claim_batch, confirm_batch, fail_batch, requeue_stale_batches, Settlement, SettlementBatch,
    SettlementError,
};

sol! {
    #[sol(rpc)]
    interface IMeshReward {
        function submitBatch(bytes32 root, uint256 totalAmount, uint256 receiptCount) external;
        function claim(bytes32 root, bytes32 receiptHash, address recipient, uint256 amount, bytes32[] calldata proof) external;
        function claimed(bytes32 receiptHash) external view returns (bool);
        function balances(address account) external view returns (uint256);
        function issuer() external view returns (address);
    }
}

/// Settlement against an EVM chain running the `MeshReward` contract.
#[derive(Debug, Clone)]
pub struct EvmSettlement {
    /// EVM JSON-RPC endpoint.
    pub rpc_url: String,
    /// Deployed `MeshReward` address.
    pub contract_address: String,
    /// Hex-encoded issuer private key used to sign `submitBatch`.
    pub issuer_key: String,
    /// Chain id recorded on batches.
    pub chain_id: i64,
}

impl Settlement for EvmSettlement {
    async fn reconcile(
        &self,
        pool: &PgPool,
        max_receipts: usize,
    ) -> Result<usize, SettlementError> {
        requeue_stale_batches(pool, 120).await?;
        let Some(batch) = claim_batch(pool, max_receipts, self.chain_id).await? else {
            return Ok(0);
        };

        match self.submit_root(&batch).await {
            Ok(tx_hash) => {
                confirm_batch(pool, &batch.batch_id, Some(&tx_hash)).await?;
                Ok(batch.entries.len())
            }
            Err(err) => {
                fail_batch(pool, &batch.batch_id).await?;
                Err(err)
            }
        }
    }
}

impl EvmSettlement {
    /// Submit a batch's Merkle root to the contract.
    pub async fn submit_root(&self, batch: &SettlementBatch) -> Result<String, SettlementError> {
        let signer = PrivateKeySigner::from_str(&self.issuer_key)
            .map_err(|e| SettlementError::Config(format!("invalid issuer key: {e}")))?;
        let wallet = EthereumWallet::from(signer);
        let url = self
            .rpc_url
            .parse()
            .map_err(|e| SettlementError::Config(format!("invalid rpc url: {e}")))?;
        let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);
        let address = Address::from_str(&self.contract_address)
            .map_err(|e| SettlementError::Config(format!("invalid contract address: {e}")))?;

        let contract = IMeshReward::new(address, &provider);
        let root = B256::from(batch.merkle_root);
        let pending = contract
            .submitBatch(
                root,
                U256::from(batch.total_reward.max(0) as u64),
                U256::from(batch.entries.len() as u64),
            )
            .send()
            .await
            .map_err(|e| SettlementError::Backend(e.to_string()))?;
        let receipt = pending
            .get_receipt()
            .await
            .map_err(|e| SettlementError::Backend(e.to_string()))?;

        Ok(format!("{:#x}", receipt.transaction_hash))
    }
}
