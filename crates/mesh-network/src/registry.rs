//! A registry of the latest verified advertisement per node.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

use mesh_protocol::ids::NodeId;

use crate::advertisement::{NodeAdvertisement, SignedNodeAdvertisement};

/// Tracks the most recent advertisement seen for each node.
#[derive(Default)]
pub struct DiscoveryRegistry {
    inner: Mutex<HashMap<NodeId, SignedNodeAdvertisement>>,
}

impl DiscoveryRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace the advertisement for its node.
    ///
    /// Callers should verify the signature first (see
    /// [`crate::verify_advertisement`]).
    pub fn upsert(&self, advertisement: SignedNodeAdvertisement) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.insert(advertisement.advertisement.node_id.clone(), advertisement);
    }

    /// Fetch the current advertisement for a node.
    pub fn get(&self, node_id: &NodeId) -> Option<NodeAdvertisement> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.get(node_id).map(|s| s.advertisement.clone())
    }

    /// Snapshot all current advertisements.
    pub fn peers(&self) -> Vec<NodeAdvertisement> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.values().map(|s| s.advertisement.clone()).collect()
    }

    /// Remove expired advertisements, returning how many were pruned.
    pub fn prune_expired(&self, now: DateTime<Utc>) -> usize {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let before = inner.len();
        inner.retain(|_, signed| !signed.advertisement.is_expired_at(now));
        before - inner.len()
    }

    /// Number of known peers.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn signed(node: &str, expires: i64) -> SignedNodeAdvertisement {
        SignedNodeAdvertisement {
            advertisement: NodeAdvertisement {
                node_id: NodeId::new(node),
                peer_id: "peer".to_string(),
                addresses: vec![],
                protocols: vec!["mesh/1".to_string()],
                issued_at: Utc.timestamp_opt(1_900_000_000, 0).unwrap(),
                expires_at: Utc.timestamp_opt(expires, 0).unwrap(),
                nonce: 1,
            },
            public_key: "ab".repeat(32),
            signature: "cd".repeat(64),
        }
    }

    #[test]
    fn tracks_latest_and_prunes_expired() {
        let registry = DiscoveryRegistry::new();
        registry.upsert(signed("node-1", 1_900_000_100));
        registry.upsert(signed("node-2", 1_800_000_000));
        assert_eq!(registry.len(), 2);

        let now = Utc.timestamp_opt(1_900_000_050, 0).unwrap();
        assert_eq!(registry.prune_expired(now), 1);
        assert_eq!(registry.len(), 1);
        assert!(registry.get(&NodeId::new("node-2")).is_none());
    }
}
