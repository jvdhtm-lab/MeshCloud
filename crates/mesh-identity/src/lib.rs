//! Node identity and signing.
//!
//! A node's identity is an Ed25519 keypair. The private key is persisted to
//! disk with restrictive permissions and zeroized in memory when dropped.

#![forbid(unsafe_code)]

use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use mesh_protocol::canonical::{signing_payload, CanonicalEncode};
use mesh_protocol::ids::NodeId;

mod agent;

pub use agent::AgentIdentity;

/// Errors raised by identity handling.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// The key material could not be parsed.
    #[error("invalid key material: {0}")]
    InvalidKey(String),
    /// A signature failed to verify.
    #[error("signature verification failed")]
    BadSignature,
    /// Reading or writing the identity file failed.
    #[error("identity file error: {0}")]
    Io(#[from] std::io::Error),
    /// The identity file was not valid JSON.
    #[error("identity file is malformed: {0}")]
    Decode(#[from] serde_json::Error),
}

/// On-disk representation of a node identity.
#[derive(Debug, Serialize, Deserialize)]
struct IdentityFile {
    scheme: String,
    version: u32,
    node_id: NodeId,
    public_key: String,
    /// Hex-encoded 32-byte Ed25519 seed.
    private_key: String,
}

/// A node's Ed25519 signing identity.
pub struct NodeIdentity {
    node_id: NodeId,
    signing_key: SigningKey,
}

impl NodeIdentity {
    /// Generate a brand new identity for `node_id`.
    pub fn generate(node_id: NodeId) -> Self {
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        Self {
            node_id,
            signing_key,
        }
    }

    /// Reconstruct an identity from a raw 32-byte seed.
    pub fn from_seed(node_id: NodeId, seed: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(seed);
        Self {
            node_id,
            signing_key,
        }
    }

    /// This node's identifier.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// The public verifying key.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// The public key, hex encoded.
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.signing_key.verifying_key().to_bytes())
    }

    /// The private seed, wrapped so it is zeroized when dropped.
    pub fn seed(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.signing_key.to_bytes())
    }

    /// Sign a message, returning the 64-byte signature.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
    }

    /// Sign a canonical payload, returning a hex-encoded signature.
    ///
    /// The signed bytes are the domain-separated signing payload, so two
    /// implementations with the same types always sign the same bytes.
    pub fn sign_canonical<T: CanonicalEncode + ?Sized>(&self, message: &T) -> String {
        hex::encode(self.sign(&signing_payload(message)))
    }

    /// Verify a hex-encoded signature over a canonical payload.
    pub fn verify_canonical<T: CanonicalEncode + ?Sized>(
        public_key_hex: &str,
        message: &T,
        signature_hex: &str,
    ) -> Result<(), IdentityError> {
        let sig_bytes =
            hex::decode(signature_hex).map_err(|e| IdentityError::InvalidKey(e.to_string()))?;
        let signature: [u8; 64] = sig_bytes
            .as_slice()
            .try_into()
            .map_err(|_| IdentityError::InvalidKey("signature must be 64 bytes".into()))?;
        Self::verify(public_key_hex, &signing_payload(message), &signature)
    }

    /// Verify `signature` over `message` against a hex-encoded public key.
    pub fn verify(
        public_key_hex: &str,
        message: &[u8],
        signature: &[u8; 64],
    ) -> Result<(), IdentityError> {
        let bytes =
            hex::decode(public_key_hex).map_err(|e| IdentityError::InvalidKey(e.to_string()))?;
        let key_bytes: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| IdentityError::InvalidKey("public key must be 32 bytes".into()))?;
        let verifying_key = VerifyingKey::from_bytes(&key_bytes)
            .map_err(|e| IdentityError::InvalidKey(e.to_string()))?;
        let signature = Signature::from_bytes(signature);
        verifying_key
            .verify(message, &signature)
            .map_err(|_| IdentityError::BadSignature)
    }

    /// Persist the identity to `path`, restricting permissions where possible.
    pub fn save(&self, path: &Path) -> Result<(), IdentityError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = IdentityFile {
            scheme: "ed25519".into(),
            version: 1,
            node_id: self.node_id.clone(),
            public_key: self.public_key_hex(),
            private_key: hex::encode(self.seed().as_slice()),
        };
        let encoded = serde_json::to_vec_pretty(&file)?;
        std::fs::write(path, encoded)?;
        restrict_permissions(path)?;
        Ok(())
    }

    /// Load an identity from `path`.
    pub fn load(path: &Path) -> Result<Self, IdentityError> {
        let raw = std::fs::read(path)?;
        let file: IdentityFile = serde_json::from_slice(&raw)?;
        if file.scheme != "ed25519" {
            return Err(IdentityError::InvalidKey(format!(
                "unsupported scheme: {}",
                file.scheme
            )));
        }
        let seed_bytes =
            hex::decode(&file.private_key).map_err(|e| IdentityError::InvalidKey(e.to_string()))?;
        let seed: [u8; 32] = seed_bytes
            .as_slice()
            .try_into()
            .map_err(|_| IdentityError::InvalidKey("private key must be 32 bytes".into()))?;
        let identity = Self::from_seed(file.node_id, &seed);
        // Defensive check: the stored public key must match the seed.
        if identity.public_key_hex() != file.public_key {
            return Err(IdentityError::InvalidKey(
                "public key does not match private key".into(),
            ));
        }
        Ok(identity)
    }

    /// Load the identity at `path`, generating and saving one if it is absent.
    pub fn load_or_generate(path: &Path) -> Result<Self, IdentityError> {
        if path.exists() {
            Self::load(path)
        } else {
            let identity = Self::generate(NodeId::generate());
            identity.save(path)?;
            Ok(identity)
        }
    }
}

#[cfg(unix)]
pub(crate) fn restrict_permissions(path: &Path) -> Result<(), IdentityError> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn restrict_permissions(_path: &Path) -> Result<(), IdentityError> {
    // On Windows, ACLs are the right mechanism; the default per-user profile
    // ACL is a reasonable starting point for Phase 0.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let id = NodeIdentity::generate(NodeId::generate());
        let msg = b"hello meshcloud";
        let sig = id.sign(msg);
        assert!(NodeIdentity::verify(&id.public_key_hex(), msg, &sig).is_ok());
    }

    #[test]
    fn tampered_message_fails_verification() {
        let id = NodeIdentity::generate(NodeId::generate());
        let sig = id.sign(b"original");
        assert!(NodeIdentity::verify(&id.public_key_hex(), b"tampered", &sig).is_err());
    }

    #[test]
    fn file_roundtrip_preserves_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key");
        let original = NodeIdentity::generate(NodeId::new("node-test"));
        original.save(&path).unwrap();

        let loaded = NodeIdentity::load(&path).unwrap();
        assert_eq!(loaded.node_id(), original.node_id());
        assert_eq!(loaded.public_key_hex(), original.public_key_hex());
        assert_eq!(loaded.seed().as_slice(), original.seed().as_slice());
    }

    #[test]
    fn load_or_generate_creates_then_reuses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("node.key");
        let first = NodeIdentity::load_or_generate(&path).unwrap();
        let second = NodeIdentity::load_or_generate(&path).unwrap();
        assert_eq!(first.node_id(), second.node_id());
        assert_eq!(first.public_key_hex(), second.public_key_hex());
    }

    #[test]
    fn canonical_signature_roundtrips_and_detects_tampering() {
        use chrono::{TimeZone, Utc};
        use mesh_protocol::offers::{Availability, ResourceLimits, ResourceOffer};
        use mesh_protocol::NodeCapabilities;

        let id = NodeIdentity::generate(NodeId::new("node-canon"));
        let mut offer = ResourceOffer {
            node_id: id.node_id().clone(),
            capabilities: NodeCapabilities {
                cpu_cores: 8,
                ram_gb: 32,
                storage_gb: 500,
                gpu: true,
            },
            limits: ResourceLimits {
                max_cpu_percent: 75,
                max_memory_mb: 8192,
            },
            availability: Availability::Always,
            expires_at: Utc.timestamp_opt(1_900_000_000, 0).unwrap(),
            nonce: 1,
        };

        let signature = id.sign_canonical(&offer);
        assert!(NodeIdentity::verify_canonical(&id.public_key_hex(), &offer, &signature).is_ok());

        offer.nonce = 2;
        assert!(NodeIdentity::verify_canonical(&id.public_key_hex(), &offer, &signature).is_err());
    }
}
