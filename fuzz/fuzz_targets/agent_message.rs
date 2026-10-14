#![no_main]
//! Fuzz the agent-message decoders: they must never panic on arbitrary bytes.
use libfuzzer_sys::fuzz_target;
use mesh_agent_protocol::{AgentMessage, SignedAgentEnvelope, SignedAgentManifest};

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<AgentMessage>(data);
    let _ = serde_json::from_slice::<SignedAgentEnvelope>(data);
    let _ = serde_json::from_slice::<SignedAgentManifest>(data);
});
