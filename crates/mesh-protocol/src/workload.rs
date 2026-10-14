//! Workload-specific input formats.

use serde::{Deserialize, Serialize};

/// Input for the `wasm` workload.
///
/// The module runs in the sandbox with [`data`](Self::data) serialized to
/// canonical JSON bytes, and returns a deterministic output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WasmJobInput {
    /// Base64-encoded module bytes (WASM binary or WAT text).
    pub module_b64: String,
    /// Data passed to the module's `run` function.
    #[serde(default)]
    pub data: serde_json::Value,
}

impl WasmJobInput {
    /// Decode the embedded module bytes.
    pub fn module_bytes(&self) -> Result<Vec<u8>, base64::DecodeError> {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.decode(self.module_b64.as_bytes())
    }

    /// Encode module bytes into a [`WasmJobInput`].
    pub fn from_module_bytes(module: &[u8], data: serde_json::Value) -> Self {
        use base64::Engine as _;
        Self {
            module_b64: base64::engine::general_purpose::STANDARD.encode(module),
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_bytes_roundtrip() {
        let input =
            WasmJobInput::from_module_bytes(b"\0asm\x01\0\0\0", serde_json::json!({"n": 1}));
        assert_eq!(input.module_bytes().unwrap(), b"\0asm\x01\0\0\0");
        assert_eq!(input.data["n"], 1);
    }
}
