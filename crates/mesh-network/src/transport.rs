//! The peer-messaging transport abstraction.
//!
//! Workers and agents advertise and discover peers through a [`MeshTransport`].
//! The default [`MemoryTransport`] is an in-process broadcast used for tests and
//! single-process deployments; the `libp2p` feature provides a real
//! [`crate::libp2p_transport::Libp2pTransport`]. Because messaging sits behind
//! this trait, the scheduler and control plane do not depend on the transport.

use tokio::sync::broadcast;

use crate::advertisement::SignedNodeAdvertisement;
use crate::NetworkError;

/// A publish/subscribe transport for signed node advertisements.
pub trait MeshTransport: Send + Sync {
    /// Publish an advertisement to the mesh.
    fn advertise(&self, advertisement: SignedNodeAdvertisement) -> Result<(), NetworkError>;

    /// Subscribe to advertisements from the mesh.
    fn advertisements(&self) -> broadcast::Receiver<SignedNodeAdvertisement>;
}

/// An in-process broadcast transport.
pub struct MemoryTransport {
    sender: broadcast::Sender<SignedNodeAdvertisement>,
}

impl MemoryTransport {
    /// Create a transport with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self { sender }
    }
}

impl MeshTransport for MemoryTransport {
    fn advertise(&self, advertisement: SignedNodeAdvertisement) -> Result<(), NetworkError> {
        // Ignore "no receivers"; the advertisement is simply not retained.
        let _ = self.sender.send(advertisement);
        Ok(())
    }

    fn advertisements(&self) -> broadcast::Receiver<SignedNodeAdvertisement> {
        self.sender.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::advertisement::NodeAdvertisement;
    use chrono::{TimeZone, Utc};
    use mesh_protocol::ids::NodeId;

    fn advertisement(node: &str, nonce: u64) -> SignedNodeAdvertisement {
        SignedNodeAdvertisement {
            advertisement: NodeAdvertisement {
                node_id: NodeId::new(node),
                peer_id: "peer".to_string(),
                addresses: vec![],
                protocols: vec!["mesh/1".to_string()],
                issued_at: Utc.timestamp_opt(1_900_000_000, 0).unwrap(),
                expires_at: Utc.timestamp_opt(1_900_000_100, 0).unwrap(),
                nonce,
            },
            public_key: "ab".repeat(32),
            signature: "cd".repeat(64),
        }
    }

    #[tokio::test]
    async fn publish_is_received_by_subscribers() {
        let transport = MemoryTransport::new(8);
        let mut rx = transport.advertisements();
        transport.advertise(advertisement("node-1", 1)).unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received.advertisement.node_id.as_str(), "node-1");
    }
}
