//! Agent identity, separate from node and wallet identity.

use std::path::Path;

use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use mesh_protocol::canonical::{signing_payload, CanonicalEncode};
use mesh_protocol::ids::AgentId;

use crate::{restrict_permissions, IdentityError, NodeIdentity};

#[derive(Debug, Serialize, Deserialize)]
struct AgentIdentityFile {
    scheme: String,
    version: u32,
    agent_id: AgentId,
    public_key: String,
    private_key: String,
}

/// An agent's Ed25519 signing identity. Distinct from the node identity and
/// from any participant wallet.
pub struct AgentIdentity {
    agent_id: AgentId,
    signing_key: SigningKey,
}

impl AgentIdentity {
    /// Generate a new identity for `agent_id`.
    pub fn generate(agent_id: AgentId) -> Self {
        Self {
            agent_id,
            signing_key: SigningKey::generate(&mut rand::rngs::OsRng),
        }
    }

    /// Reconstruct an identity from a 32-byte seed.
    pub fn from_seed(agent_id: AgentId, seed: &[u8; 32]) -> Self {
        Self {
            agent_id,
            signing_key: SigningKey::from_bytes(seed),
        }
    }

    /// This agent's identifier.
    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    /// The public key, hex encoded.
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.signing_key.verifying_key().to_bytes())
    }

    /// The private seed, zeroized on drop.
    pub fn seed(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.signing_key.to_bytes())
    }

    /// Sign a raw message.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
    }

    /// Sign a canonical payload, returning a hex-encoded signature.
    pub fn sign_canonical<T: CanonicalEncode + ?Sized>(&self, message: &T) -> String {
        hex::encode(self.sign(&signing_payload(message)))
    }

    /// Verify a hex-encoded signature over a canonical payload.
    pub fn verify_canonical<T: CanonicalEncode + ?Sized>(
        public_key_hex: &str,
        message: &T,
        signature_hex: &str,
    ) -> Result<(), IdentityError> {
        NodeIdentity::verify_canonical(public_key_hex, message, signature_hex)
    }

    /// Persist the identity with restrictive permissions.
    pub fn save(&self, path: &Path) -> Result<(), IdentityError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = AgentIdentityFile {
            scheme: "ed25519".into(),
            version: 1,
            agent_id: self.agent_id.clone(),
            public_key: self.public_key_hex(),
            private_key: hex::encode(self.seed().as_slice()),
        };
        std::fs::write(path, serde_json::to_vec_pretty(&file)?)?;
        restrict_permissions(path)?;
        Ok(())
    }

    /// Load an agent identity from `path`.
    pub fn load(path: &Path) -> Result<Self, IdentityError> {
        let raw = std::fs::read(path)?;
        let file: AgentIdentityFile = serde_json::from_slice(&raw)?;
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
        let identity = Self::from_seed(file.agent_id, &seed);
        if identity.public_key_hex() != file.public_key {
            return Err(IdentityError::InvalidKey(
                "public key does not match private key".into(),
            ));
        }
        Ok(identity)
    }

    /// Load the identity at `path`, or generate one for `agent_id` if absent.
    pub fn load_or_generate(agent_id: AgentId, path: &Path) -> Result<Self, IdentityError> {
        if path.exists() {
            Self::load(path)
        } else {
            let identity = Self::generate(agent_id);
            identity.save(path)?;
            Ok(identity)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_roundtrip_preserves_agent_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.key");
        let original = AgentIdentity::generate(AgentId::new("agent-test"));
        original.save(&path).unwrap();

        let loaded = AgentIdentity::load(&path).unwrap();
        assert_eq!(loaded.agent_id(), original.agent_id());
        assert_eq!(loaded.public_key_hex(), original.public_key_hex());
    }
}
