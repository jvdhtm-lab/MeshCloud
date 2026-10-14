//! Shared protocol for MeshCloud AI agents.
//!
//! Agent identity, capability manifests, and agent-to-agent messages live here.
//! Everything is signed over a canonical byte encoding so two implementations
//! cannot disagree about what was authorized.

#![forbid(unsafe_code)]

pub mod api;
pub mod manifest;
pub mod message;

/// The agent wire protocol version.
pub const AGENT_PROTOCOL_VERSION: u32 = 1;

pub use api::*;
pub use manifest::*;
pub use message::*;
