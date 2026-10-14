//! Strongly typed identifiers.
//!
//! Each identifier is a distinct newtype so the compiler rejects accidentally
//! passing a `JobId` where a `NodeId` is expected.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Wrap an existing identifier string.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Generate a fresh random identifier with this type's prefix.
            pub fn generate() -> Self {
                Self(format!("{}-{}", $prefix, Uuid::new_v4()))
            }

            /// Borrow the identifier as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(
    /// Identifies a participant (resource owner / account holder).
    ParticipantId,
    "ptc"
);
id_type!(
    /// Identifies a worker node in the network.
    NodeId,
    "node"
);
id_type!(
    /// Identifies an AI agent hosted by a worker.
    AgentId,
    "agent"
);
id_type!(
    /// Identifies a submitted job.
    JobId,
    "job"
);
id_type!(
    /// Identifies a single execution attempt of a job.
    AttemptId,
    "att"
);
id_type!(
    /// Identifies a contribution receipt.
    ReceiptId,
    "rcpt"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_prefixed_and_unique() {
        let a = NodeId::generate();
        let b = NodeId::generate();
        assert!(a.as_str().starts_with("node-"));
        assert_ne!(a, b);
    }

    #[test]
    fn ids_roundtrip_through_json() {
        let id = JobId::new("job-123");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"job-123\"");
        let back: JobId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }
}
