//! Shared protocol types for MeshCloud.
//!
//! Everything that crosses a process or network boundary lives in this crate so
//! the control plane, workers, and CLI cannot drift apart. Types are designed
//! to make invalid states hard to represent: identifiers are distinct newtypes,
//! and lifecycle states are closed enums with explicit transition rules.

#![forbid(unsafe_code)]

pub mod api;
pub mod canonical;
pub mod ids;
pub mod offers;
pub mod status;
pub mod workload;

/// The wire protocol version. Bump on any breaking change to shared types.
pub const PROTOCOL_VERSION: u32 = 1;

/// Domain-separation tag mixed into every signed payload.
pub const SIGNING_DOMAIN: &str = "meshcloud/v1";

pub use api::*;
pub use canonical::{canonical_bytes, signing_payload, CanonicalEncode, CanonicalEncoder};
pub use ids::*;
pub use offers::*;
pub use status::*;
pub use workload::*;
