//! Signed resource offers: a node's expiring advertisement of what it shares.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::api::NodeCapabilities;
use crate::canonical::{CanonicalEncode, CanonicalEncoder};
use crate::ids::NodeId;

/// When a node is willing to share its resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// Available whenever the node is online.
    Always,
    /// Available only when the machine is otherwise idle.
    WhenIdle,
    /// Available only within a scheduled window.
    Scheduled,
}

impl Availability {
    /// Return the stable discriminant used in canonical encoding.
    pub fn code(self) -> u8 {
        match self {
            Availability::Always => 0,
            Availability::WhenIdle => 1,
            Availability::Scheduled => 2,
        }
    }
}

impl CanonicalEncode for Availability {
    fn encode_canonical(&self, encoder: &mut CanonicalEncoder) {
        encoder.push_u8(self.code());
    }
}

/// Hard caps a node places on how much of a resource a workload may use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum share of CPU a single workload may use, as a percentage.
    pub max_cpu_percent: u32,
    /// Maximum memory a single workload may use, in megabytes.
    pub max_memory_mb: u32,
}

impl CanonicalEncode for ResourceLimits {
    fn encode_canonical(&self, encoder: &mut CanonicalEncoder) {
        encoder.push_u32(self.max_cpu_percent);
        encoder.push_u32(self.max_memory_mb);
    }
}

impl CanonicalEncode for NodeCapabilities {
    fn encode_canonical(&self, encoder: &mut CanonicalEncoder) {
        encoder.push_u32(self.cpu_cores);
        encoder.push_u32(self.ram_gb);
        encoder.push_u32(self.storage_gb);
        encoder.push_bool(self.gpu);
    }
}

/// A signed, expiring advertisement of a node's shared resources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceOffer {
    /// Node publishing the offer.
    pub node_id: NodeId,
    /// Resources offered.
    pub capabilities: NodeCapabilities,
    /// Caps the node enforces on workloads.
    pub limits: ResourceLimits,
    /// When the resources are available.
    pub availability: Availability,
    /// Offer expiry (RFC 3339).
    pub expires_at: DateTime<Utc>,
    /// Monotonic nonce to make each offer unique and replay-resistant.
    pub nonce: u64,
}

impl CanonicalEncode for ResourceOffer {
    fn encode_canonical(&self, encoder: &mut CanonicalEncoder) {
        encoder.push_str(self.node_id.as_str());
        self.capabilities.encode_canonical(encoder);
        self.limits.encode_canonical(encoder);
        self.availability.encode_canonical(encoder);
        encoder.push_i64(self.expires_at.timestamp());
        encoder.push_u64(self.nonce);
    }
}

impl ResourceOffer {
    /// Whether the offer has expired as of `now`.
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }
}

/// A resource offer together with the signature that authorizes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedResourceOffer {
    /// The offered resources.
    pub offer: ResourceOffer,
    /// Hex-encoded Ed25519 public key of the offering node.
    pub public_key: String,
    /// Hex-encoded Ed25519 signature over the canonical signing payload.
    pub signature: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::signing_payload;
    use chrono::TimeZone;

    fn sample_offer() -> ResourceOffer {
        ResourceOffer {
            node_id: NodeId::new("node-a"),
            capabilities: NodeCapabilities {
                cpu_cores: 4,
                ram_gb: 16,
                storage_gb: 100,
                gpu: true,
            },
            limits: ResourceLimits {
                max_cpu_percent: 50,
                max_memory_mb: 4096,
            },
            availability: Availability::WhenIdle,
            expires_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            nonce: 7,
        }
    }

    #[test]
    fn canonical_encoding_is_deterministic() {
        let a = sample_offer();
        let b = sample_offer();
        assert_eq!(signing_payload(&a), signing_payload(&b));
    }

    #[test]
    fn changing_a_field_changes_the_payload() {
        let a = sample_offer();
        let mut b = sample_offer();
        b.nonce += 1;
        assert_ne!(signing_payload(&a), signing_payload(&b));
    }

    #[test]
    fn signed_offer_roundtrips_through_json() {
        let signed = SignedResourceOffer {
            offer: sample_offer(),
            public_key: "ab".repeat(32),
            signature: "cd".repeat(64),
        };
        let json = serde_json::to_string(&signed).unwrap();
        let back: SignedResourceOffer = serde_json::from_str(&json).unwrap();
        assert_eq!(signed, back);
    }
}
