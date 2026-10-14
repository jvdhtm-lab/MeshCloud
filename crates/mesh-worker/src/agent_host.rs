//! Hosting AI agents inside a worker.

use std::path::Path;

use chrono::{Duration as ChronoDuration, Utc};

use mesh_agent_protocol::{AgentManifest, SignedAgentManifest, AGENT_PROTOCOL_VERSION};
use mesh_agent_runtime::{default_capabilities, default_policy, AgentBackend, DeterministicStub};
use mesh_identity::AgentIdentity;
use mesh_protocol::ids::{AgentId, NodeId};

/// A single hosted agent: its identity and its model/policy backend.
pub struct AgentHost {
    identity: AgentIdentity,
    backend: Box<dyn AgentBackend>,
    skills: Vec<String>,
}

impl AgentHost {
    /// Build a host for an agent identity, using the deterministic stub backend.
    pub fn new(identity: AgentIdentity, skills: Vec<String>) -> Self {
        Self {
            identity,
            backend: Box::new(DeterministicStub),
            skills,
        }
    }

    /// This agent's identifier.
    pub fn agent_id(&self) -> &AgentId {
        self.identity.agent_id()
    }

    /// Workload types this agent's tools permit.
    pub fn allowed_workloads(&self) -> Vec<String> {
        let mut workloads = Vec::new();
        for tool in self.backend.tools() {
            match tool.as_str() {
                "wasm_exec" => workloads.push("wasm".to_string()),
                "echo" => workloads.push("echo".to_string()),
                _ => {}
            }
        }
        workloads
    }

    /// Whether this agent may execute the given workload (capability allowlist).
    pub fn allows_workload(&self, workload: &str) -> bool {
        self.allowed_workloads().iter().any(|w| w == workload)
    }

    /// Build a freshly signed manifest for this agent.
    pub fn signed_manifest(&self, node_id: &NodeId, ttl_seconds: u64) -> SignedAgentManifest {
        let manifest = AgentManifest {
            agent_id: self.identity.agent_id().clone(),
            node_id: node_id.clone(),
            protocol_version: AGENT_PROTOCOL_VERSION,
            capabilities: default_capabilities(self.backend.as_ref(), self.skills.clone()),
            policy: default_policy(),
            expires_at: Utc::now() + ChronoDuration::seconds(ttl_seconds as i64),
            nonce: Utc::now().timestamp_millis() as u64,
        };
        let signature = self.identity.sign_canonical(&manifest);
        SignedAgentManifest {
            manifest,
            public_key: self.identity.public_key_hex(),
            signature,
        }
    }
}

/// Load or create `count` agent identities beside `node_identity_path`.
pub fn load_agent_hosts(
    node_identity_path: &Path,
    count: u32,
    skills: Vec<String>,
) -> anyhow::Result<Vec<AgentHost>> {
    let dir = node_identity_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(".").to_path_buf());

    let mut hosts = Vec::with_capacity(count as usize);
    for index in 0..count {
        let path = dir.join(format!("agent-{index}.key"));
        let identity = AgentIdentity::load_or_generate(AgentId::generate(), &path)?;
        hosts.push(AgentHost::new(identity, skills.clone()));
    }
    Ok(hosts)
}
