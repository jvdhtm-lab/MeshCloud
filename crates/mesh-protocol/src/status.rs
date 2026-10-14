//! Lifecycle state enums and their legal transitions.

use serde::{Deserialize, Serialize};

/// Liveness state of a node as tracked by the control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// Reachable and accepting work.
    Online,
    /// Not currently heartbeating.
    Offline,
    /// Intentionally finishing in-flight work before leaving.
    Draining,
    /// Permanently removed from the registry.
    Decommissioned,
}

impl NodeStatus {
    /// Whether new work may be assigned to a node in this state.
    pub fn accepts_work(self) -> bool {
        matches!(self, NodeStatus::Online)
    }
}

impl std::fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            NodeStatus::Online => "online",
            NodeStatus::Offline => "offline",
            NodeStatus::Draining => "draining",
            NodeStatus::Decommissioned => "decommissioned",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for NodeStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "online" => Ok(NodeStatus::Online),
            "offline" => Ok(NodeStatus::Offline),
            "draining" => Ok(NodeStatus::Draining),
            "decommissioned" => Ok(NodeStatus::Decommissioned),
            other => Err(format!("unknown node status: {other}")),
        }
    }
}

/// Lifecycle state of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Submitted, not yet assigned.
    Pending,
    /// Claimed by a worker.
    Scheduled,
    /// Execution in progress.
    Running,
    /// The job delegated child jobs and is aggregating their results.
    WaitingOnChildren,
    /// Completed and accepted.
    Succeeded,
    /// Waiting for a retry after a transient failure.
    RetryWait,
    /// Permanently failed.
    Failed,
    /// Cancelled by the requester.
    Cancelled,
    /// Deadline passed without success.
    Expired,
}

impl JobState {
    /// Whether the state is terminal (no further transitions).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobState::Succeeded | JobState::Failed | JobState::Cancelled | JobState::Expired
        )
    }

    /// Whether a transition from `self` to `next` is legal.
    ///
    /// Illegal transitions are rejected in exactly one place so the state
    /// machine stays consistent across the control plane and workers.
    pub fn can_transition_to(self, next: JobState) -> bool {
        use JobState::*;
        matches!(
            (self, next),
            (Pending, Scheduled)
                | (Pending, Cancelled)
                | (Pending, Expired)
                | (Scheduled, Running)
                | (Scheduled, RetryWait)
                | (Scheduled, Failed)
                | (Scheduled, Cancelled)
                | (Scheduled, Expired)
                | (Running, Succeeded)
                | (Running, Failed)
                | (Running, RetryWait)
                | (Running, WaitingOnChildren)
                | (Running, Expired)
                | (WaitingOnChildren, Succeeded)
                | (WaitingOnChildren, Failed)
                | (WaitingOnChildren, Cancelled)
                | (WaitingOnChildren, Expired)
                | (RetryWait, Scheduled)
                | (RetryWait, Pending)
                | (RetryWait, Failed)
                | (RetryWait, Cancelled)
                | (RetryWait, Expired)
        )
    }

    /// Return the next state, or an error if the transition is illegal.
    pub fn transition(self, next: JobState) -> Result<JobState, IllegalTransition> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(IllegalTransition {
                from: self,
                to: next,
            })
        }
    }
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            JobState::Pending => "pending",
            JobState::Scheduled => "scheduled",
            JobState::Running => "running",
            JobState::WaitingOnChildren => "waiting_on_children",
            JobState::Succeeded => "succeeded",
            JobState::RetryWait => "retry_wait",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
            JobState::Expired => "expired",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for JobState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(JobState::Pending),
            "scheduled" => Ok(JobState::Scheduled),
            "running" => Ok(JobState::Running),
            "waiting_on_children" => Ok(JobState::WaitingOnChildren),
            "succeeded" => Ok(JobState::Succeeded),
            "retry_wait" => Ok(JobState::RetryWait),
            "failed" => Ok(JobState::Failed),
            "cancelled" => Ok(JobState::Cancelled),
            "expired" => Ok(JobState::Expired),
            other => Err(format!("unknown job state: {other}")),
        }
    }
}

/// Error returned when an illegal job-state transition is attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("illegal job state transition: {from} -> {to}")]
pub struct IllegalTransition {
    /// State the job was in.
    pub from: JobState,
    /// State the caller attempted to move to.
    pub to: JobState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_is_legal() {
        let s = JobState::Pending;
        let s = s.transition(JobState::Scheduled).unwrap();
        let s = s.transition(JobState::Running).unwrap();
        let s = s.transition(JobState::Succeeded).unwrap();
        assert!(s.is_terminal());
    }

    #[test]
    fn illegal_transition_is_rejected() {
        assert!(JobState::Pending.transition(JobState::Succeeded).is_err());
        assert!(JobState::Succeeded.transition(JobState::Running).is_err());
    }

    #[test]
    fn retry_wait_can_be_rescheduled() {
        assert!(JobState::Running
            .transition(JobState::RetryWait)
            .unwrap()
            .can_transition_to(JobState::Scheduled));
    }
}
