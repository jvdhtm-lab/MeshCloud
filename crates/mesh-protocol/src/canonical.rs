//! Canonical byte encoding for signed payloads.
//!
//! Signing must cover a deterministic byte layout, never "whatever JSON Serde
//! produced." This module defines an explicit little-endian binary encoding
//! with length-prefixed strings and a domain-separation tag plus protocol
//! version prepended to every signing payload.
//!
//! The encoding is deliberately simple and fixed-field so that two
//! implementations can never disagree about the bytes.

use crate::{PROTOCOL_VERSION, SIGNING_DOMAIN};

/// A growable buffer that appends fields in a single, fixed encoding.
#[derive(Debug, Default, Clone)]
pub struct CanonicalEncoder {
    buf: Vec<u8>,
}

impl CanonicalEncoder {
    /// Create an empty encoder.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Append a single byte.
    pub fn push_u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    /// Append a `u32` in little-endian order.
    pub fn push_u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Append a `u64` in little-endian order.
    pub fn push_u64(&mut self, value: u64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Append an `i64` in little-endian order.
    pub fn push_i64(&mut self, value: i64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Append a boolean as a single byte (`0` or `1`).
    pub fn push_bool(&mut self, value: bool) {
        self.buf.push(u8::from(value));
    }

    /// Append a length-prefixed byte string.
    pub fn push_bytes(&mut self, value: &[u8]) {
        self.push_u32(value.len() as u32);
        self.buf.extend_from_slice(value);
    }

    /// Append a length-prefixed UTF-8 string.
    pub fn push_str(&mut self, value: &str) {
        self.push_bytes(value.as_bytes());
    }

    /// Consume the encoder and return the accumulated bytes.
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// A type that can be encoded canonically for signing.
pub trait CanonicalEncode {
    /// Append this value's canonical representation to `encoder`.
    fn encode_canonical(&self, encoder: &mut CanonicalEncoder);
}

/// Encode a value to its canonical byte representation.
pub fn canonical_bytes<T: CanonicalEncode + ?Sized>(value: &T) -> Vec<u8> {
    let mut encoder = CanonicalEncoder::new();
    value.encode_canonical(&mut encoder);
    encoder.finish()
}

/// Build the byte string that is actually signed: domain tag, protocol
/// version, then the canonical payload.
pub fn signing_payload<T: CanonicalEncode + ?Sized>(value: &T) -> Vec<u8> {
    let mut encoder = CanonicalEncoder::new();
    encoder.push_str(SIGNING_DOMAIN);
    encoder.push_u32(PROTOCOL_VERSION);
    value.encode_canonical(&mut encoder);
    encoder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_encoding_is_little_endian() {
        let mut encoder = CanonicalEncoder::new();
        encoder.push_u32(1);
        assert_eq!(encoder.finish(), vec![1, 0, 0, 0]);
    }

    #[test]
    fn strings_are_length_prefixed() {
        let mut encoder = CanonicalEncoder::new();
        encoder.push_str("ab");
        assert_eq!(encoder.finish(), vec![2, 0, 0, 0, b'a', b'b']);
    }
}
