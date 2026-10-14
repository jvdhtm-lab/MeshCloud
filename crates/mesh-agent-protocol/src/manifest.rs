//! Agent capability manifests and their canonical encoding.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use mesh_protocol::canonical::{signing_payload, CanonicalEncode, CanonicalEncoder};
use mesh_protocol::ids::{AgentId, NodeId};

/// What an agent can do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCapabilities {
    /// Skills the agent advertises.
    pub skills: Vec<String>,
    /// Tools the agent may call.
    pub tools: Vec<String>,
    /// Model/backend identifiers the agent can run.
    pub models: Vec<String>,
    /// Maximum concurrent tasks.
    pub max_concurrent_tasks: u32,
    /// Maximum delegation depth the agent supports.
    pub max_depth_supported: u32,
}

impl CanonicalEncode for AgentCapabilities {
    fn encode_canonical(&self, encoder: &mut CanonicalEncoder) {
        encode_strings(encoder, &self.skills);
        encode_strings(encoder, &self.tools);
        encode_strings(encoder, &self.models);
        encoder.push_u32(self.max_concurrent_tasks);
        encoder.push_u32(self.max_depth_supported);
    }
}

/// Delegation policy advertised by an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPolicy {
    /// Whether the agent accepts delegated tasks.
    pub accepts_delegation: bool,
    /// Minimum budget the agent will accept.
    pub min_budget_credits: i64,
    /// Maximum wall-clock time the agent will spend on a task.
    pub max_task_seconds: u64,
}

impl CanonicalEncode for AgentPolicy {
    fn encode_canonical(&self, encoder: &mut CanonicalEncoder) {
        encoder.push_bool(self.accepts_delegation);
        encoder.push_i64(self.min_budget_credits);
        encoder.push_u64(self.max_task_seconds);
    }
}

/// A signed, expiring advertisement of an agent's capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentManifest {
    /// Agent publishing the manifest.
    pub agent_id: AgentId,
    /// Worker hosting the agent.
    pub node_id: NodeId,
    /// Agent protocol version.
    pub protocol_version: u32,
    /// Advertised capabilities.
    pub capabilities: AgentCapabilities,
    /// Delegation policy.
    pub policy: AgentPolicy,
    /// Expiry (RFC 3339).
    pub expires_at: DateTime<Utc>,
    /// Replay-resistant nonce.
    pub nonce: u64,
}

impl CanonicalEncode for AgentManifest {
    fn encode_canonical(&self, encoder: &mut CanonicalEncoder) {
        encoder.push_str(self.agent_id.as_str());
        encoder.push_str(self.node_id.as_str());
        encoder.push_u32(self.protocol_version);
        self.capabilities.encode_canonical(encoder);
        self.policy.encode_canonical(encoder);
        encoder.push_i64(self.expires_at.timestamp());
        encoder.push_u64(self.nonce);
    }
}

impl AgentManifest {
    /// Whether the manifest has expired as of `now`.
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }
}

/// An agent manifest together with the signature that authorizes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedAgentManifest {
    /// The manifest.
    pub manifest: AgentManifest,
    /// Hex-encoded Ed25519 public key of the agent.
    pub public_key: String,
    /// Hex-encoded Ed25519 signature over the canonical signing payload.
    pub signature: String,
}

/// Hash of a manifest's canonical signing payload, used as its identity.
pub fn manifest_hash(manifest: &AgentManifest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(signing_payload(manifest));
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

    fn manifest(nonce: u64) -> AgentManifest {
        AgentManifest {
            agent_id: AgentId::new("agent-1"),
            node_id: NodeId::new("node-1"),
            protocol_version: 1,
            capabilities: AgentCapabilities {
                skills: vec!["planning".into()],
                tools: vec!["wasm_exec".into()],
                models: vec!["noop-deterministic".into()],
                max_concurrent_tasks: 4,
                max_depth_supported: 3,
            },
            policy: AgentPolicy {
                accepts_delegation: true,
                min_budget_credits: 1,
                max_task_seconds: 300,
            },
            expires_at: Utc.timestamp_opt(1_900_000_000, 0).unwrap(),
            nonce,
        }
    }

    #[test]
    fn canonical_encoding_is_deterministic() {
        assert_eq!(signing_payload(&manifest(1)), signing_payload(&manifest(1)));
        assert_ne!(manifest_hash(&manifest(1)), manifest_hash(&manifest(2)));
    }
}
