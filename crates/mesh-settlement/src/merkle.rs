//! Merkle tree over reward receipts.
//!
//! Leaves are `keccak256(receiptHash || recipient || amount)` encoded exactly as
//! Solidity `abi.encodePacked`. Internal nodes hash the pair with the two
//! children sorted, and an odd node is promoted unchanged. The on-chain
//! verifier uses the same scheme.

use sha3::{Digest, Keccak256};

/// Hash a receipt id string to the 32-byte value used in leaves and on-chain.
pub fn receipt_hash(receipt_id: &str) -> [u8; 32] {
    keccak(receipt_id.as_bytes())
}

/// Compute a leaf for a receipt.
pub fn leaf_hash(receipt_id: &str, recipient: &[u8; 20], amount: i64) -> [u8; 32] {
    let mut buffer = Vec::with_capacity(32 + 20 + 32);
    buffer.extend_from_slice(&receipt_hash(receipt_id));
    buffer.extend_from_slice(recipient);
    buffer.extend_from_slice(&amount_be32(amount));
    keccak(&buffer)
}

/// Parse a `0x`-prefixed hex address into 20 bytes.
pub fn address_bytes(address: &str) -> Option<[u8; 20]> {
    let trimmed = address.strip_prefix("0x").unwrap_or(address);
    let bytes = hex::decode(trimmed).ok()?;
    let array: [u8; 20] = bytes.as_slice().try_into().ok()?;
    Some(array)
}

/// Compute the Merkle root of a list of leaves.
pub fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    let mut level = leaves.to_vec();
    while level.len() > 1 {
        level = next_level(&level);
    }
    level[0]
}

/// Compute the Merkle proof for the leaf at `index`.
pub fn merkle_proof(leaves: &[[u8; 32]], index: usize) -> Vec<[u8; 32]> {
    let mut proof = Vec::new();
    let mut idx = index;
    let mut level = leaves.to_vec();
    while level.len() > 1 {
        if idx % 2 == 1 {
            proof.push(level[idx - 1]);
        } else if idx + 1 < level.len() {
            proof.push(level[idx + 1]);
        }
        idx /= 2;
        level = next_level(&level);
    }
    proof
}

/// Verify a Merkle proof against a root.
pub fn verify_proof(root: &[u8; 32], leaf: [u8; 32], proof: &[[u8; 32]]) -> bool {
    let mut computed = leaf;
    for sibling in proof {
        computed = hash_pair(&computed, sibling);
    }
    &computed == root
}

fn next_level(level: &[[u8; 32]]) -> Vec<[u8; 32]> {
    let mut next = Vec::with_capacity(level.len().div_ceil(2));
    let mut i = 0;
    while i < level.len() {
        if i + 1 < level.len() {
            next.push(hash_pair(&level[i], &level[i + 1]));
        } else {
            next.push(level[i]);
        }
        i += 2;
    }
    next
}

fn hash_pair(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let (first, second) = if a <= b { (a, b) } else { (b, a) };
    let mut buffer = [0u8; 64];
    buffer[..32].copy_from_slice(first);
    buffer[32..].copy_from_slice(second);
    keccak(&buffer)
}

fn amount_be32(amount: i64) -> [u8; 32] {
    let mut out = [0u8; 32];
    let value = amount.max(0) as u64;
    out[24..32].copy_from_slice(&value.to_be_bytes());
    out
}

fn keccak(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_leaf_root_is_the_leaf() {
        let leaf = leaf_hash("rcpt-1", &[0u8; 20], 1);
        assert_eq!(merkle_root(&[leaf]), leaf);
        assert!(verify_proof(&merkle_root(&[leaf]), leaf, &[]));
    }

    #[test]
    fn proofs_verify_for_all_leaves() {
        let leaves: Vec<[u8; 32]> = (0..7)
            .map(|i| leaf_hash(&format!("rcpt-{i}"), &[i as u8; 20], i as i64 + 1))
            .collect();
        let root = merkle_root(&leaves);
        for (i, leaf) in leaves.iter().enumerate() {
            let proof = merkle_proof(&leaves, i);
            assert!(verify_proof(&root, *leaf, &proof), "leaf {i} failed");
        }
    }

    #[test]
    fn tampered_leaf_fails_verification() {
        let leaves: Vec<[u8; 32]> = (0..4)
            .map(|i| leaf_hash(&format!("rcpt-{i}"), &[0u8; 20], 1))
            .collect();
        let root = merkle_root(&leaves);
        let proof = merkle_proof(&leaves, 0);
        let tampered = leaf_hash("rcpt-0", &[0u8; 20], 999);
        assert!(!verify_proof(&root, tampered, &proof));
    }

    #[test]
    fn address_parsing() {
        assert!(address_bytes("0x0000000000000000000000000000000000000001").is_some());
        assert!(address_bytes("nope").is_none());
    }
}
