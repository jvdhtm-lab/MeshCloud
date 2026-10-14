//! Local shard store and master-key handling for the storage prototype.
//!
//! Shards are written under `<storage_dir>/<object_id>/<index>.bin`. In a fully
//! distributed deployment the bytes would live on the assigned nodes; here the
//! control plane holds them so placement, repair, and challenges can be
//! exercised end to end.

use std::path::{Path, PathBuf};

use mesh_storage::crypto;

/// Load the master key from config or the storage directory, creating one if
/// needed.
pub fn load_or_create_key(dir: &Path, configured: Option<&str>) -> anyhow::Result<[u8; 32]> {
    std::fs::create_dir_all(dir)?;
    if let Some(value) = configured {
        return Ok(crypto::key_from_hex(value)?);
    }
    let key_path = dir.join("master.key");
    if key_path.exists() {
        let encoded = std::fs::read_to_string(&key_path)?;
        Ok(crypto::key_from_hex(encoded.trim())?)
    } else {
        let key = crypto::random_key();
        std::fs::write(&key_path, hex::encode(key))?;
        Ok(key)
    }
}

/// Path to a shard file.
pub fn shard_path(dir: &Path, object_id: &str, index: i32) -> PathBuf {
    dir.join(object_id).join(format!("{index}.bin"))
}

/// Write a shard file.
pub fn write_shard(dir: &Path, object_id: &str, index: i32, bytes: &[u8]) -> std::io::Result<()> {
    let path = shard_path(dir, object_id, index);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)
}

/// Read a shard file.
pub fn read_shard(dir: &Path, object_id: &str, index: i32) -> std::io::Result<Vec<u8>> {
    std::fs::read(shard_path(dir, object_id, index))
}

/// Whether a shard file exists.
pub fn shard_exists(dir: &Path, object_id: &str, index: i32) -> bool {
    shard_path(dir, object_id, index).exists()
}
