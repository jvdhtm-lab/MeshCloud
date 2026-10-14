//! Property test: agent protocol decoders must never panic on arbitrary bytes.
use mesh_agent_protocol::{AgentMessage, SignedAgentEnvelope, SignedAgentManifest};
use proptest::prelude::*;

proptest! {
    #[test]
    fn agent_decoders_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let _ = serde_json::from_slice::<AgentMessage>(&bytes);
        let _ = serde_json::from_slice::<SignedAgentEnvelope>(&bytes);
        let _ = serde_json::from_slice::<SignedAgentManifest>(&bytes);
    }
}
