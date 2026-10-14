//! Agent runtime: the model/policy backend behind an agent.
//!
//! Phase 6 ships only the deterministic stub backend. LLM backends are added
//! behind the same [`AgentBackend`] trait in later phases. The stub produces
//! reproducible output so agent behavior is testable end to end.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use mesh_agent_protocol::{AgentCapabilities, AgentPolicy};

/// A pluggable agent backend: a planner/policy engine.
pub trait AgentBackend: Send + Sync {
    /// Stable backend identifier recorded in manifests.
    fn name(&self) -> &'static str;

    /// Tools the backend can use.
    fn tools(&self) -> Vec<String>;

    /// Models the backend can run.
    fn models(&self) -> Vec<String>;

    /// Deterministically split a task spec into subtask specs.
    fn plan(&self, spec: &str) -> Vec<String>;
}

/// A deterministic, non-LLM backend used for development and tests.
#[derive(Debug, Default, Clone, Copy)]
pub struct DeterministicStub;

impl AgentBackend for DeterministicStub {
    fn name(&self) -> &'static str {
        "noop-deterministic"
    }

    fn tools(&self) -> Vec<String> {
        vec!["echo".to_string(), "wasm_exec".to_string()]
    }

    fn models(&self) -> Vec<String> {
        vec!["noop-deterministic".to_string()]
    }

    fn plan(&self, spec: &str) -> Vec<String> {
        let parts: Vec<String> = spec
            .split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() {
            if spec.trim().is_empty() {
                Vec::new()
            } else {
                vec![spec.trim().to_string()]
            }
        } else {
            parts
        }
    }
}

/// Build default capabilities for a backend, given a skill list.
pub fn default_capabilities(backend: &dyn AgentBackend, skills: Vec<String>) -> AgentCapabilities {
    AgentCapabilities {
        skills,
        tools: backend.tools(),
        models: backend.models(),
        max_concurrent_tasks: 4,
        max_depth_supported: 3,
    }
}

/// Build the default delegation policy.
pub fn default_policy() -> AgentPolicy {
    AgentPolicy {
        accepts_delegation: true,
        min_budget_credits: 1,
        max_task_seconds: 300,
    }
}

struct Bucket {
    messages: f64,
    bytes: f64,
    last: Instant,
}

/// A per-agent token bucket enforcing message-rate and byte-rate limits.
///
/// Every agent message (including delegation attempts) costs one message token
/// and its encoded size in byte tokens. Both are refilled continuously.
pub struct AgentMeter {
    message_rate: f64,
    message_burst: f64,
    byte_rate: f64,
    byte_burst: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl AgentMeter {
    /// Create a meter with the given per-second rates and burst capacities.
    pub fn new(message_rate: f64, message_burst: f64, byte_rate: f64, byte_burst: f64) -> Self {
        Self {
            message_rate: message_rate.max(0.0),
            message_burst: message_burst.max(1.0),
            byte_rate: byte_rate.max(0.0),
            byte_burst: byte_burst.max(1.0),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Try to consume one message and `bytes` for `agent`. Returns `false` when
    /// either limit is exceeded.
    pub fn allow(&self, agent: &str, bytes: u64) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = buckets.entry(agent.to_string()).or_insert(Bucket {
            messages: self.message_burst,
            bytes: self.byte_burst,
            last: now,
        });

        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.last = now;
        bucket.messages = (bucket.messages + elapsed * self.message_rate).min(self.message_burst);
        bucket.bytes = (bucket.bytes + elapsed * self.byte_rate).min(self.byte_burst);

        let cost = bytes as f64;
        if bucket.messages < 1.0 || bucket.bytes < cost {
            return false;
        }
        bucket.messages -= 1.0;
        bucket.bytes -= cost;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_plan_is_deterministic() {
        let stub = DeterministicStub;
        let plan = stub.plan("a, b ,c");
        assert_eq!(plan, vec!["a", "b", "c"]);
        assert_eq!(stub.plan("a, b ,c"), plan);
        assert!(stub.plan("").is_empty());
        assert_eq!(stub.plan("single"), vec!["single"]);
    }

    #[test]
    fn default_capabilities_use_backend_tools() {
        let stub = DeterministicStub;
        let caps = default_capabilities(&stub, vec!["planning".to_string()]);
        assert_eq!(caps.models, vec!["noop-deterministic"]);
        assert!(caps.tools.contains(&"wasm_exec".to_string()));
    }

    #[test]
    fn meter_enforces_message_rate() {
        let meter = AgentMeter::new(0.0, 2.0, 1_000_000.0, 1_000_000.0);
        assert!(meter.allow("agent-a", 10));
        assert!(meter.allow("agent-a", 10));
        // Burst exhausted and no refill at rate 0.
        assert!(!meter.allow("agent-a", 10));
        // A different agent has its own bucket.
        assert!(meter.allow("agent-b", 10));
    }

    #[test]
    fn meter_enforces_byte_budget() {
        let meter = AgentMeter::new(100.0, 100.0, 1000.0, 1000.0);
        assert!(meter.allow("agent-a", 900));
        assert!(!meter.allow("agent-a", 900));
    }
}
