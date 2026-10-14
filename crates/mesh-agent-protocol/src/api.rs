//! Agent API request/response types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use mesh_protocol::ids::{AgentId, NodeId, ParticipantId};

use crate::manifest::{AgentCapabilities, AgentPolicy, SignedAgentManifest};

/// Liveness state of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Reachable and accepting work.
    Online,
    /// Not currently heartbeating.
    Offline,
    /// Intentionally finishing in-flight work.
    Draining,
}

impl AgentStatus {
    /// Whether new work may be assigned to an agent in this state.
    pub fn accepts_work(self) -> bool {
        matches!(self, AgentStatus::Online)
    }
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AgentStatus::Online => "online",
            AgentStatus::Offline => "offline",
            AgentStatus::Draining => "draining",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for AgentStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "online" => Ok(AgentStatus::Online),
            "offline" => Ok(AgentStatus::Offline),
            "draining" => Ok(AgentStatus::Draining),
            other => Err(format!("unknown agent status: {other}")),
        }
    }
}

/// `POST /api/v1/agents/register` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterAgentRequest {
    /// Worker hosting the agent. Must already be registered.
    pub node_id: NodeId,
    /// Signed capability manifest. Its `node_id` must match `node_id`.
    pub manifest: SignedAgentManifest,
}

/// `POST /api/v1/agents/register` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterAgentResponse {
    /// Registered agent.
    pub agent_id: AgentId,
    /// Server-assigned status.
    pub status: AgentStatus,
    /// How often the agent should heartbeat, in seconds.
    pub heartbeat_interval_seconds: u64,
}

/// `POST /api/v1/agents/{id}/heartbeat` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHeartbeatRequest {
    /// Freshly signed manifest.
    pub manifest: SignedAgentManifest,
}

/// `POST /api/v1/agents/{id}/heartbeat` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHeartbeatResponse {
    /// The agent that heartbeated.
    pub agent_id: AgentId,
    /// Server-assigned status.
    pub status: AgentStatus,
    /// Server time at which the heartbeat was processed.
    pub server_time: DateTime<Utc>,
    /// How often the agent should heartbeat, in seconds.
    pub heartbeat_interval_seconds: u64,
}

/// An agent as returned by the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentView {
    /// Agent identifier.
    pub agent_id: AgentId,
    /// Hosting worker.
    pub node_id: NodeId,
    /// Owning participant.
    pub participant_id: ParticipantId,
    /// Current status.
    pub status: AgentStatus,
    /// Advertised capabilities.
    pub capabilities: AgentCapabilities,
    /// Delegation policy.
    pub policy: AgentPolicy,
    /// Hash of the current capability manifest.
    pub manifest_hash: String,
    /// Last reported backend, if any.
    pub backend: Option<String>,
    /// RFC 3339 timestamp of the last heartbeat, if any.
    pub last_heartbeat: Option<DateTime<Utc>>,
}
