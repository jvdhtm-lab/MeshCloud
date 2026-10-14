//! Reed-Solomon erasure coding over GF(256).

use reed_solomon_erasure::galois_8::ReedSolomon;
use sha2::{Digest, Sha256};

use crate::StorageError;

/// Encode `data` into `k` data shards plus `m` parity shards (all equal length).
pub fn encode(data: &[u8], k: usize, m: usize) -> Result<Vec<Vec<u8>>, StorageError> {
    if k == 0 || m == 0 || k + m > 255 {
        return Err(StorageError::Erasure("invalid shard counts".into()));
    }
    let shard_len = data.len().div_ceil(k).max(1);
    let mut shards: Vec<Vec<u8>> = Vec::with_capacity(k + m);
    for index in 0..k {
        let start = index * shard_len;
        let mut shard = vec![0u8; shard_len];
        if start < data.len() {
            let end = (start + shard_len).min(data.len());
            shard[..end - start].copy_from_slice(&data[start..end]);
        }
        shards.push(shard);
    }
    for _ in 0..m {
        shards.push(vec![0u8; shard_len]);
    }

    let rs = ReedSolomon::new(k, m).map_err(|e| StorageError::Erasure(e.to_string()))?;
    rs.encode(&mut shards)
        .map_err(|e| StorageError::Erasure(e.to_string()))?;
    Ok(shards)
}

/// Reconstruct the original `data` from any `k` of the `n` shards.
///
/// `shards` must have length `k + m`, with `None` for missing shards.
pub fn reconstruct(
    shards: &[Option<Vec<u8>>],
    k: usize,
    m: usize,
    original_len: usize,
) -> Result<Vec<u8>, StorageError> {
    let mut working = shards.to_vec();
    let rs = ReedSolomon::new(k, m).map_err(|e| StorageError::Erasure(e.to_string()))?;
    rs.reconstruct(&mut working)
        .map_err(|e| StorageError::Erasure(e.to_string()))?;

    let mut output = Vec::with_capacity(original_len);
    for shard in working.iter().take(k) {
        output.extend_from_slice(
            shard
                .as_ref()
                .ok_or_else(|| StorageError::Erasure("shard still missing".into()))?,
        );
    }
    output.truncate(original_len);
    Ok(output)
}

/// Checksum of a shard (for integrity and repair detection).
pub fn shard_checksum(shard: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(shard);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_from_any_k_shards() {
        let data = b"the quick brown fox jumps over the lazy dog".repeat(20);
        let (k, m) = (4, 2);
        let shards = encode(&data, k, m).unwrap();
        assert_eq!(shards.len(), k + m);

        // Drop two shards (the maximum tolerated) and reconstruct.
        let mut present: Vec<Option<Vec<u8>>> = shards.iter().cloned().map(Some).collect();
        present[1] = None;
        present[4] = None;
        let recovered = reconstruct(&present, k, m, data.len()).unwrap();
        assert_eq!(recovered, data);
    }

    #[test]
    fn too_many_missing_shards_fails() {
        let data = b"abcdefgh".to_vec();
        let (k, m) = (2, 1);
        let shards = encode(&data, k, m).unwrap();
        let mut present: Vec<Option<Vec<u8>>> = shards.iter().cloned().map(Some).collect();
        present[0] = None;
        present[1] = None;
        // Only parity remains: not enough.
        assert!(reconstruct(&present, k, m, data.len()).is_err());
    }
}
