//! Agent-to-agent message types and their signed envelope.

use serde::{Deserialize, Serialize};

use mesh_protocol::canonical::{CanonicalEncode, CanonicalEncoder};
use mesh_protocol::ids::AgentId;

/// The agent protocol message set. All messages reference a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentMessage {
    /// Initial handshake.
    Hello {
        /// Protocol version.
        protocol_version: u32,
    },
    /// Liveness and current load.
    Heartbeat {
        /// Jobs the agent currently owns.
        owned_jobs: u32,
        /// Maximum concurrent tasks.
        max_concurrent: u32,
        /// Backend identifier.
        backend: String,
    },
    /// Advertisement of a capability manifest by hash.
    CapabilityAdvertisement {
        /// Hash of the advertised manifest.
        manifest_hash: String,
    },
    /// Proposal to run a job.
    TaskProposal {
        /// Job being proposed.
        job_id: String,
        /// Hash of the task spec.
        spec_hash: String,
    },
    /// Response to a proposal.
    TaskBid {
        /// Job being bid on.
        job_id: String,
        /// Whether the agent accepts.
        accept: bool,
        /// Price in credits.
        price_credits: i64,
        /// Estimated seconds to completion.
        eta_seconds: u64,
    },
    /// Award of a job to an agent.
    TaskAward {
        /// Job awarded.
        job_id: String,
    },
    /// Acceptance of an award.
    TaskAccept {
        /// Job accepted.
        job_id: String,
    },
    /// Progress report.
    TaskProgress {
        /// Job in progress.
        job_id: String,
        /// Percent complete.
        percent: u8,
    },
    /// Successful result.
    TaskResult {
        /// Job completed.
        job_id: String,
        /// Hash of the result.
        result_hash: String,
    },
    /// Failure report.
    TaskFailure {
        /// Job that failed.
        job_id: String,
        /// Human-readable reason.
        error: String,
    },
    /// Cancellation.
    TaskCancel {
        /// Job cancelled.
        job_id: String,
    },
    /// Delegation of a child job.
    DelegateTask {
        /// Child job being delegated.
        job_id: String,
        /// Parent job.
        parent_job_id: String,
        /// Hash of the child spec.
        spec_hash: String,
    },
    /// Cancellation of a delegation.
    CancelDelegation {
        /// Child job whose delegation is cancelled.
        job_id: String,
    },
}

impl AgentMessage {
    /// A stable discriminator for the message kind.
    pub fn code(&self) -> u8 {
        match self {
            AgentMessage::Hello { .. } => 0,
            AgentMessage::Heartbeat { .. } => 1,
            AgentMessage::CapabilityAdvertisement { .. } => 2,
            AgentMessage::TaskProposal { .. } => 3,
            AgentMessage::TaskBid { .. } => 4,
            AgentMessage::TaskAward { .. } => 5,
            AgentMessage::TaskAccept { .. } => 6,
            AgentMessage::TaskProgress { .. } => 7,
            AgentMessage::TaskResult { .. } => 8,
            AgentMessage::TaskFailure { .. } => 9,
            AgentMessage::TaskCancel { .. } => 10,
            AgentMessage::DelegateTask { .. } => 11,
            AgentMessage::CancelDelegation { .. } => 12,
        }
    }

    /// A stable string name for the message kind.
    pub fn kind(&self) -> &'static str {
        match self {
            AgentMessage::Hello { .. } => "hello",
            AgentMessage::Heartbeat { .. } => "heartbeat",
            AgentMessage::CapabilityAdvertisement { .. } => "capability_advertisement",
            AgentMessage::TaskProposal { .. } => "task_proposal",
            AgentMessage::TaskBid { .. } => "task_bid",
            AgentMessage::TaskAward { .. } => "task_award",
            AgentMessage::TaskAccept { .. } => "task_accept",
            AgentMessage::TaskProgress { .. } => "task_progress",
            AgentMessage::TaskResult { .. } => "task_result",
            AgentMessage::TaskFailure { .. } => "task_failure",
            AgentMessage::TaskCancel { .. } => "task_cancel",
            AgentMessage::DelegateTask { .. } => "delegate_task",
            AgentMessage::CancelDelegation { .. } => "cancel_delegation",
        }
    }
}

impl CanonicalEncode for AgentMessage {
    fn encode_canonical(&self, encoder: &mut CanonicalEncoder) {
        encoder.push_u8(self.code());
        match self {
            AgentMessage::Hello { protocol_version } => encoder.push_u32(*protocol_version),
            AgentMessage::Heartbeat {
                owned_jobs,
                max_concurrent,
                backend,
            } => {
                encoder.push_u32(*owned_jobs);
                encoder.push_u32(*max_concurrent);
                encoder.push_str(backend);
            }
            AgentMessage::CapabilityAdvertisement { manifest_hash } => {
                encoder.push_str(manifest_hash);
            }
            AgentMessage::TaskProposal { job_id, spec_hash } => {
                encoder.push_str(job_id);
                encoder.push_str(spec_hash);
            }
            AgentMessage::TaskBid {
                job_id,
                accept,
                price_credits,
                eta_seconds,
            } => {
                encoder.push_str(job_id);
                encoder.push_bool(*accept);
                encoder.push_i64(*price_credits);
                encoder.push_u64(*eta_seconds);
            }
            AgentMessage::TaskAward { job_id }
            | AgentMessage::TaskAccept { job_id }
            | AgentMessage::TaskCancel { job_id }
            | AgentMessage::CancelDelegation { job_id } => encoder.push_str(job_id),
            AgentMessage::TaskProgress { job_id, percent } => {
                encoder.push_str(job_id);
                encoder.push_u8(*percent);
            }
            AgentMessage::TaskResult {
                job_id,
                result_hash,
            } => {
                encoder.push_str(job_id);
                encoder.push_str(result_hash);
            }
            AgentMessage::TaskFailure { job_id, error } => {
                encoder.push_str(job_id);
                encoder.push_str(error);
            }
            AgentMessage::DelegateTask {
                job_id,
                parent_job_id,
                spec_hash,
            } => {
                encoder.push_str(job_id);
                encoder.push_str(parent_job_id);
                encoder.push_str(spec_hash);
            }
        }
    }
}

/// The signed envelope that carries every agent message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEnvelope {
    /// Protocol version.
    pub protocol_version: u32,
    /// Sending agent.
    pub from: AgentId,
    /// Intended recipient, if any (broadcast when absent).
    pub to: Option<AgentId>,
    /// Replay-resistant nonce.
    pub nonce: u64,
    /// The message payload.
    pub payload: AgentMessage,
}

impl CanonicalEncode for AgentEnvelope {
    fn encode_canonical(&self, encoder: &mut CanonicalEncoder) {
        encoder.push_u32(self.protocol_version);
        encoder.push_str(self.from.as_str());
        match &self.to {
            Some(to) => {
                encoder.push_bool(true);
                encoder.push_str(to.as_str());
            }
            None => encoder.push_bool(false),
        }
        encoder.push_u64(self.nonce);
        self.payload.encode_canonical(encoder);
    }
}

/// A signed agent envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedAgentEnvelope {
    /// The envelope.
    pub envelope: AgentEnvelope,
    /// Hex-encoded public key of the sender.
    pub public_key: String,
    /// Hex-encoded signature over the canonical signing payload.
    pub signature: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_protocol::canonical::signing_payload;

    #[test]
    fn message_kinds_are_distinct() {
        let hello = AgentMessage::Hello {
            protocol_version: 1,
        };
        let cancel = AgentMessage::TaskCancel {
            job_id: "job-1".into(),
        };
        assert_ne!(hello.code(), cancel.code());
        assert_eq!(hello.kind(), "hello");
        assert_eq!(signing_payload(&hello), signing_payload(&hello.clone()));
    }
}
