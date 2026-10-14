//! Retrieval challenges: proof that a node still holds a shard's bytes.

use sha2::{Digest, Sha256};

/// Build a challenge for a shard at a given nonce.
pub fn challenge_for(shard_id: &str, nonce: u64) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(shard_id.as_bytes());
    hasher.update(nonce.to_le_bytes());
    hasher.finalize().into()
}

/// Compute a node's response to a challenge over the shard bytes.
pub fn respond(challenge: &[u8; 32], shard: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(challenge);
    hasher.update(shard);
    hasher.finalize().into()
}

/// Verify a response against a challenge and the expected shard bytes.
///
/// The verifier recomputes the response from the shard it stored at placement
/// time; a node that has lost the bytes cannot answer a fresh challenge.
pub fn verify(challenge: &[u8; 32], shard: &[u8], response: &[u8; 32]) -> bool {
    &respond(challenge, shard) == response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_challenge_requires_the_bytes() {
        let shard = b"stored shard bytes";
        let challenge = challenge_for("shard-1", 42);
        let response = respond(&challenge, shard);
        assert!(verify(&challenge, shard, &response));
        // A different challenge yields a different response.
        let other = challenge_for("shard-1", 43);
        assert!(!verify(&other, shard, &response));
        // Wrong bytes fail.
        assert!(!verify(&challenge, b"corrupted", &response));
    }
}
