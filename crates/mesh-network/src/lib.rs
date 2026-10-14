//! MeshCloud peer-to-peer networking.
//!
//! This crate owns peer identity, signed node advertisements, discovery, the
//! transport abstraction, and the control-plane replica model. Job scheduling
//! and accounting stay in the control plane; the mesh only handles discovery
//! and peer messaging, so a partition can never create conflicting accepted
//! work (see [`replica`]).

#![forbid(unsafe_code)]

pub mod advertisement;
#[cfg(feature = "libp2p")]
pub mod libp2p_transport;
pub mod registry;
pub mod replica;
pub mod transport;

use chrono::{Duration, Utc};
use libp2p_identity::{ed25519, Keypair, PeerId, PublicKey};
use mesh_identity::NodeIdentity;

pub use advertisement::{advertisement_hash, NodeAdvertisement, SignedNodeAdvertisement};
#[cfg(feature = "libp2p")]
pub use libp2p_transport::Libp2pTransport;
pub use registry::DiscoveryRegistry;
pub use replica::ReplicaRole;
pub use transport::{MemoryTransport, MeshTransport};

/// Errors raised by the networking layer.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    /// The identity could not be converted to a libp2p keypair.
    #[error("invalid identity: {0}")]
    Identity(String),
    /// A peer id could not be parsed or derived.
    #[error("invalid peer id: {0}")]
    PeerId(String),
    /// An advertisement was malformed or failed verification.
    #[error("invalid advertisement: {0}")]
    Advertisement(String),
    /// The transport failed.
    #[error("transport error: {0}")]
    Transport(String),
}

/// Derive the libp2p [`PeerId`] for a node identity.
pub fn peer_id_from_identity(identity: &NodeIdentity) -> Result<PeerId, NetworkError> {
    Ok(keypair_from_identity(identity)?.public().to_peer_id())
}

/// Derive a libp2p [`Keypair`] from a node identity's Ed25519 seed.
pub fn keypair_from_identity(identity: &NodeIdentity) -> Result<Keypair, NetworkError> {
    let seed = identity.seed();
    Keypair::ed25519_from_bytes(seed.to_vec()).map_err(|e| NetworkError::Identity(e.to_string()))
}

/// Derive the libp2p [`PeerId`] for a hex-encoded Ed25519 public key.
pub fn peer_id_from_public_key(public_key_hex: &str) -> Result<PeerId, NetworkError> {
    let bytes = hex::decode(public_key_hex).map_err(|e| NetworkError::PeerId(e.to_string()))?;
    let key_bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| NetworkError::PeerId("public key must be 32 bytes".into()))?;
    let public = ed25519::PublicKey::try_from_bytes(&key_bytes)
        .map_err(|e| NetworkError::PeerId(e.to_string()))?;
    Ok(PeerId::from_public_key(&PublicKey::from(public)))
}

/// Build an advertisement for a node.
pub fn build_advertisement(
    identity: &NodeIdentity,
    addresses: Vec<String>,
    protocols: Vec<String>,
    ttl_seconds: u64,
    nonce: u64,
) -> Result<NodeAdvertisement, NetworkError> {
    let peer_id = peer_id_from_identity(identity)?.to_base58();
    let issued_at = Utc::now();
    Ok(NodeAdvertisement {
        node_id: identity.node_id().clone(),
        peer_id,
        addresses,
        protocols,
        issued_at,
        expires_at: issued_at + Duration::seconds(ttl_seconds as i64),
        nonce,
    })
}

/// Sign an advertisement with a node identity.
pub fn sign_advertisement(
    identity: &NodeIdentity,
    advertisement: NodeAdvertisement,
) -> SignedNodeAdvertisement {
    let signature = identity.sign_canonical(&advertisement);
    SignedNodeAdvertisement {
        advertisement,
        public_key: identity.public_key_hex(),
        signature,
    }
}

/// Verify an advertisement's signature and that its peer id matches its key.
pub fn verify_advertisement(signed: &SignedNodeAdvertisement) -> Result<(), NetworkError> {
    NodeIdentity::verify_canonical(&signed.public_key, &signed.advertisement, &signed.signature)
        .map_err(|e| NetworkError::Advertisement(e.to_string()))?;

    let peer = peer_id_from_public_key(&signed.public_key)?;
    if peer.to_base58() != signed.advertisement.peer_id {
        return Err(NetworkError::Advertisement(
            "peer_id does not match public key".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_protocol::ids::NodeId;

    #[test]
    fn advertisement_roundtrips_and_verifies() {
        let identity = NodeIdentity::generate(NodeId::new("node-net-1"));
        let advertisement = build_advertisement(
            &identity,
            vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
            vec!["mesh/1".to_string()],
            120,
            1,
        )
        .unwrap();
        let signed = sign_advertisement(&identity, advertisement);
        assert!(verify_advertisement(&signed).is_ok());

        // The peer id must match the identity's key.
        let expected = peer_id_from_identity(&identity).unwrap().to_base58();
        assert_eq!(signed.advertisement.peer_id, expected);
    }

    #[test]
    fn tampered_advertisement_is_rejected() {
        let identity = NodeIdentity::generate(NodeId::new("node-net-2"));
        let mut advertisement = build_advertisement(&identity, vec![], vec![], 120, 1).unwrap();
        let signed = sign_advertisement(&identity, advertisement.clone());
        advertisement.nonce = 999;
        let tampered = SignedNodeAdvertisement {
            advertisement,
            public_key: signed.public_key,
            signature: signed.signature,
        };
        assert!(verify_advertisement(&tampered).is_err());
    }
}
