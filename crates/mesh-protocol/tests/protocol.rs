//! Property tests for protocol invariants.

use mesh_protocol::ids::{JobId, NodeId, ParticipantId};
use proptest::prelude::*;

proptest! {
    #[test]
    fn node_id_roundtrips_through_json(s in ".*") {
        let id = NodeId::new(s.clone());
        let json = serde_json::to_string(&id).unwrap();
        let back: NodeId = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(id, back);
    }

    #[test]
    fn generated_ids_never_collide(n in 1u32..2_000) {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..n {
            prop_assert!(seen.insert(JobId::generate()));
        }
    }
}

#[test]
fn distinct_id_types_have_distinct_prefixes() {
    assert!(ParticipantId::generate().as_str().starts_with("ptc-"));
    assert!(NodeId::generate().as_str().starts_with("node-"));
    assert!(JobId::generate().as_str().starts_with("job-"));
}
