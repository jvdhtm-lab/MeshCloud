//! Signed node advertisements for the MeshCloud peer mesh.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use mesh_protocol::canonical::{signing_payload, CanonicalEncode, CanonicalEncoder};
use mesh_protocol::ids::NodeId;

/// A node's advertised presence on the mesh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeAdvertisement {
    /// Node publishing the advertisement.
    pub node_id: NodeId,
    /// libp2p peer id (base58) derived from the node's Ed25519 key.
    pub peer_id: String,
    /// Dialable multiaddresses.
    pub addresses: Vec<String>,
    /// Supported protocols (e.g. `mesh/1`).
    pub protocols: Vec<String>,
    /// Issued-at timestamp.
    pub issued_at: DateTime<Utc>,
    /// Expiry timestamp.
    pub expires_at: DateTime<Utc>,
    /// Replay-resistant nonce.
    pub nonce: u64,
}

impl CanonicalEncode for NodeAdvertisement {
    fn encode_canonical(&self, encoder: &mut CanonicalEncoder) {
        encoder.push_str(self.node_id.as_str());
        encoder.push_str(&self.peer_id);
        encode_strings(encoder, &self.addresses);
        encode_strings(encoder, &self.protocols);
        encoder.push_i64(self.issued_at.timestamp());
        encoder.push_i64(self.expires_at.timestamp());
        encoder.push_u64(self.nonce);
    }
}

impl NodeAdvertisement {
    /// Whether the advertisement has expired as of `now`.
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }
}

/// A node advertisement together with its signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedNodeAdvertisement {
    /// The advertisement.
    pub advertisement: NodeAdvertisement,
    /// Hex-encoded Ed25519 public key of the publisher.
    pub public_key: String,
    /// Hex-encoded signature over the canonical signing payload.
    pub signature: String,
}

/// Hash of an advertisement's canonical signing payload.
pub fn advertisement_hash(advertisement: &NodeAdvertisement) -> String {
    let mut hasher = Sha256::new();
    hasher.update(signing_payload(advertisement));
    hex::encode(hasher.finalize())
}

fn encode_strings(encoder: &mut CanonicalEncoder, values: &[String]) {
    encoder.push_u32(values.len() as u32);
    for value in values {
        encoder.push_str(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn advertisement(nonce: u64) -> NodeAdvertisement {
        NodeAdvertisement {
            node_id: NodeId::new("node-1"),
            peer_id: "12D3KooWtest".to_string(),
            addresses: vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
            protocols: vec!["mesh/1".to_string()],
            issued_at: Utc.timestamp_opt(1_900_000_000, 0).unwrap(),
            expires_at: Utc.timestamp_opt(1_900_000_100, 0).unwrap(),
            nonce,
        }
    }

    #[test]
    fn canonical_encoding_is_deterministic() {
        assert_eq!(
            signing_payload(&advertisement(1)),
            signing_payload(&advertisement(1))
        );
        assert_ne!(
            advertisement_hash(&advertisement(1)),
            advertisement_hash(&advertisement(2))
        );
    }
}
