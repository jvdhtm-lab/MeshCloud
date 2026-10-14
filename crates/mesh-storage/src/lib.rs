//! MeshCloud distributed storage primitives.
//!
//! Objects are encrypted client-side, split into Reed-Solomon data and parity
//! shards, and placed one shard per node. Availability is checked with retrieval
//! challenges, and missing shards are reconstructed from the surviving shards
//! and re-placed. Rewards are tied to verified availability, not allocated
//! space.

#![forbid(unsafe_code)]

pub mod challenge;
pub mod crypto;
pub mod erasure;

use mesh_protocol::ids::NodeId;

/// Errors raised by the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// An encryption or decryption error.
    #[error("crypto error: {0}")]
    Crypto(String),
    /// An erasure-coding error.
    #[error("erasure error: {0}")]
    Erasure(String),
    /// A placement error.
    #[error("placement error: {0}")]
    Placement(String),
    /// An I/O error.
    #[error("storage I/O error: {0}")]
    Io(String),
}

/// Deterministically assign one shard to each of `shard_count` distinct nodes.
pub fn plan(nodes: &[NodeId], shard_count: usize) -> Result<Vec<NodeId>, StorageError> {
    let mut unique: Vec<NodeId> = nodes.to_vec();
    unique.sort();
    unique.dedup();
    if unique.len() < shard_count {
        return Err(StorageError::Placement(format!(
            "need {shard_count} nodes, have {}",
            unique.len()
        )));
    }
    Ok(unique.into_iter().take(shard_count).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_is_deterministic_and_distinct() {
        let nodes = vec![
            NodeId::new("node-b"),
            NodeId::new("node-a"),
            NodeId::new("node-c"),
            NodeId::new("node-a"),
        ];
        let placed = plan(&nodes, 3).unwrap();
        assert_eq!(
            placed,
            vec![
                NodeId::new("node-a"),
                NodeId::new("node-b"),
                NodeId::new("node-c")
            ]
        );
        assert!(plan(&nodes, 4).is_err());
    }
}
