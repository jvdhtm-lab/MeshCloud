//! Client-side encryption for stored objects (AES-256-GCM).

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};

use crate::StorageError;

/// An encrypted object: a random nonce and the ciphertext.
#[derive(Debug, Clone)]
pub struct Encrypted {
    /// 96-bit GCM nonce.
    pub nonce: [u8; 12],
    /// Ciphertext (includes the GCM tag).
    pub ciphertext: Vec<u8>,
}

/// Generate a fresh 256-bit key.
pub fn random_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(&mut key);
    key
}

/// Parse a 32-byte key from hex.
pub fn key_from_hex(value: &str) -> Result<[u8; 32], StorageError> {
    let bytes = hex::decode(value).map_err(|e| StorageError::Crypto(e.to_string()))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| StorageError::Crypto("key must be 32 bytes".into()))
}

/// Encrypt `plaintext` with `key`, returning a random nonce and ciphertext.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Encrypted, StorageError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| StorageError::Crypto(e.to_string()))?;
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(nonce.as_slice());
    Ok(Encrypted {
        nonce: nonce_bytes,
        ciphertext,
    })
}

/// Decrypt ciphertext produced by [`encrypt`].
pub fn decrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    ciphertext: &[u8],
) -> Result<Vec<u8>, StorageError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| StorageError::Crypto(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_tamper_detection() {
        let key = random_key();
        let encrypted = encrypt(&key, b"hello storage").unwrap();
        let decrypted = decrypt(&key, &encrypted.nonce, &encrypted.ciphertext).unwrap();
        assert_eq!(decrypted, b"hello storage");

        let mut tampered = encrypted.ciphertext.clone();
        tampered[0] ^= 0xff;
        assert!(decrypt(&key, &encrypted.nonce, &tampered).is_err());
    }
}
