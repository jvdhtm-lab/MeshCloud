//! Control-plane replica roles and the consensus boundary.
//!
//! # Consensus boundary
//!
//! MeshCloud deliberately splits state into two classes:
//!
//! * **Strongly consistent (requires a single authority).** Job lifecycle,
//!   leases, attempt claims, contribution receipts, credit balances, reward
//!   batches, and settlement. These live in one primary PostgreSQL instance and
//!   are protected by transactions (`FOR UPDATE SKIP LOCKED`, unique
//!   constraints). Only the primary runs the background writers: the offline
//!   sweeper, the job sweeper, the settlement reconciler, and reservation
//!   expiry. This is what prevents partitions from creating conflicting
//!   accepted jobs or reward claims.
//! * **Eventually consistent (safe to gossip).** Peer advertisements, node
//!   liveness hints, and capability discovery. These are replicated through the
//!   mesh transport and reconciled opportunistically; a stale entry only
//!   affects placement, never correctness.
//!
//! A replica therefore serves reads and participates in discovery but does not
//! run writers. Promotion to primary is an operator action (or a future
//! consensus protocol); the model keeps a single writer at a time.

use serde::{Deserialize, Serialize};

/// The role a control-plane instance plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplicaRole {
    /// The single writer: runs schedulers, sweepers, and settlement.
    Primary,
    /// Read-only: serves the API and discovery, runs no background writers.
    Replica,
}

impl ReplicaRole {
    /// Whether this role may run background writers.
    pub fn runs_writers(self) -> bool {
        matches!(self, ReplicaRole::Primary)
    }

    /// Whether this role is the primary.
    pub fn is_primary(self) -> bool {
        matches!(self, ReplicaRole::Primary)
    }
}

impl std::fmt::Display for ReplicaRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ReplicaRole::Primary => "primary",
            ReplicaRole::Replica => "replica",
        })
    }
}

impl std::str::FromStr for ReplicaRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "primary" => Ok(ReplicaRole::Primary),
            "replica" => Ok(ReplicaRole::Replica),
            other => Err(format!("unknown replica role: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_primary_runs_writers() {
        assert!(ReplicaRole::Primary.runs_writers());
        assert!(!ReplicaRole::Replica.runs_writers());
        assert_eq!(
            "replica".parse::<ReplicaRole>().unwrap(),
            ReplicaRole::Replica
        );
    }
}
