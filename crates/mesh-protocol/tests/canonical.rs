//! Property tests for canonical encoding and signing payloads.

use chrono::{TimeZone, Utc};
use mesh_protocol::canonical::{canonical_bytes, signing_payload};
use mesh_protocol::offers::{Availability, ResourceLimits, ResourceOffer};
use mesh_protocol::{NodeCapabilities, NodeId};
use proptest::prelude::*;

fn offer(nonce: u64, cpu_cores: u32) -> ResourceOffer {
    ResourceOffer {
        node_id: NodeId::new("node-prop"),
        capabilities: NodeCapabilities {
            cpu_cores,
            ram_gb: 8,
            storage_gb: 20,
            gpu: false,
        },
        limits: ResourceLimits {
            max_cpu_percent: 50,
            max_memory_mb: 1024,
        },
        availability: Availability::WhenIdle,
        expires_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
        nonce,
    }
}

proptest! {
    #[test]
    fn canonical_encoding_is_deterministic(nonce in any::<u64>(), cpu in any::<u32>()) {
        let a = offer(nonce, cpu);
        let b = offer(nonce, cpu);
        prop_assert_eq!(canonical_bytes(&a), canonical_bytes(&b));
        prop_assert_eq!(signing_payload(&a), signing_payload(&b));
    }

    #[test]
    fn distinct_nonces_produce_distinct_payloads(nonce in 0u64..1_000_000) {
        let a = offer(nonce, 4);
        let b = offer(nonce.wrapping_add(1), 4);
        prop_assert_ne!(signing_payload(&a), signing_payload(&b));
    }

    #[test]
    fn signing_payload_is_longer_than_canonical_payload(nonce in any::<u64>()) {
        let o = offer(nonce, 2);
        // The signing payload prepends the domain tag and protocol version.
        prop_assert!(signing_payload(&o).len() > canonical_bytes(&o).len());
    }
}
