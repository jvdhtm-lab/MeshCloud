//! Work verification.
//!
//! MeshCloud rewards accepted contributions, not raw claims. For deterministic
//! workloads, verification is re-execution: the verifier recomputes the result
//! from the job input independently and compares hashes. A worker cannot earn a
//! reward for a result that does not reproduce.

#![forbid(unsafe_code)]

use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};

use mesh_protocol::WasmJobInput;
use mesh_runtime::RuntimeLimits;

/// Verification limits, matched to the worker's sandbox limits.
#[derive(Debug, Clone, Copy)]
pub struct VerifyConfig {
    /// Maximum memory for re-execution, in megabytes.
    pub max_memory_mb: u64,
    /// Fuel budget for re-execution.
    pub fuel: u64,
    /// Wall-clock timeout for re-execution.
    pub timeout: Duration,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        Self {
            max_memory_mb: 256,
            fuel: 1_000_000_000,
            timeout: Duration::from_secs(10),
        }
    }
}

impl VerifyConfig {
    fn runtime_limits(&self) -> RuntimeLimits {
        RuntimeLimits::from_megabytes(self.max_memory_mb, self.fuel, self.timeout)
    }
}

/// The outcome of verifying a claimed result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    /// Whether the claimed result reproduced.
    pub passed: bool,
    /// Verification method, e.g. `wasm-reexecution`.
    pub method: String,
    /// Accepted usage units on success.
    pub accepted_units: i64,
    /// Hash of the independently recomputed result.
    pub evidence_hash: String,
    /// Human-readable detail (mismatch reason, error, or confirmation).
    pub detail: String,
}

/// Recompute the expected result hash for a workload, without trusting any
/// worker. Returns an error string for unsupported workloads.
pub fn expected_hash(
    workload_type: &str,
    input: &Value,
    config: &VerifyConfig,
) -> Result<String, String> {
    match workload_type {
        "echo" => {
            let bytes = serde_json::to_vec(input).map_err(|e| e.to_string())?;
            Ok(hash_bytes(&bytes))
        }
        "wasm" => {
            let job_input: WasmJobInput = serde_json::from_value(input.clone())
                .map_err(|e| format!("invalid wasm input: {e}"))?;
            let module = job_input
                .module_bytes()
                .map_err(|e| format!("invalid module encoding: {e}"))?;
            let data = serde_json::to_vec(&job_input.data).map_err(|e| e.to_string())?;
            let output = mesh_runtime::run(&module, &data, &config.runtime_limits())
                .map_err(|e| e.to_string())?;
            Ok(hash_bytes(&output.output))
        }
        other => Err(format!("unsupported workload type: {other}")),
    }
}

/// Verify a worker's claimed result hash by independent recomputation.
pub fn verify(
    workload_type: &str,
    input: &Value,
    claimed_hash: &str,
    config: &VerifyConfig,
) -> Verification {
    let method = method_for(workload_type).to_string();
    match expected_hash(workload_type, input, config) {
        Ok(expected) => {
            let passed = expected == claimed_hash;
            Verification {
                passed,
                method,
                accepted_units: if passed { 1 } else { 0 },
                evidence_hash: expected.clone(),
                detail: if passed {
                    "result reproduced".to_string()
                } else {
                    format!("claimed {claimed_hash} but recomputed {expected}")
                },
            }
        }
        Err(reason) => Verification {
            passed: false,
            method,
            accepted_units: 0,
            evidence_hash: String::new(),
            detail: reason,
        },
    }
}

/// The verification method name for a workload type.
pub fn method_for(workload_type: &str) -> &'static str {
    match workload_type {
        "echo" => "echo-deterministic",
        "wasm" => "wasm-reexecution",
        _ => "unsupported",
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo_input() -> Value {
        serde_json::json!({"n": 5})
    }

    #[test]
    fn echo_verification_accepts_correct_and_rejects_wrong() {
        let config = VerifyConfig::default();
        let input = echo_input();
        let correct = expected_hash("echo", &input, &config).unwrap();

        let ok = verify("echo", &input, &correct, &config);
        assert!(ok.passed);
        assert_eq!(ok.accepted_units, 1);

        let bad = verify("echo", &input, "deadbeef", &config);
        assert!(!bad.passed);
        assert_eq!(bad.accepted_units, 0);
    }

    #[test]
    fn wasm_verification_reproduces_output() {
        const HELLO_WAT: &str = r#"
            (module
              (memory (export "memory") 1)
              (data (i32.const 16) "hello from meshcloud\00")
              (func (export "alloc") (param i32) (result i32) (i32.const 1024))
              (func (export "run") (param i32 i32) (result i32) (i32.const 16)))
        "#;
        let config = VerifyConfig::default();
        let input = serde_json::to_value(WasmJobInput::from_module_bytes(
            HELLO_WAT.as_bytes(),
            serde_json::json!({}),
        ))
        .unwrap();
        let hash = expected_hash("wasm", &input, &config).unwrap();
        assert!(verify("wasm", &input, &hash, &config).passed);
        assert!(!verify("wasm", &input, "nope", &config).passed);
    }

    #[test]
    fn unsupported_workload_is_rejected() {
        let config = VerifyConfig::default();
        let result = verify("cobol", &serde_json::json!({}), "x", &config);
        assert!(!result.passed);
    }
}
