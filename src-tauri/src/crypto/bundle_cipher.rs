//! AES-256-GCM encryption for secret-bundle values.
//!
//! Format: `ciphertext || tag` (aes-gcm crate convention); the 12-byte `nonce`
//! is returned/stored separately. A fresh random nonce is generated per value
//! via the OS CSPRNG. The 32-byte data key lives in the OS keyring (see
//! `keyring_store::get_or_create_bundle_key`) — never on disk in plaintext.

use aes_gcm::{
    aead::{rand_core::RngCore, Aead, OsRng},
    Aes256Gcm, KeyInit, Nonce,
};

const NONCE_LEN: usize = 12;

/// Generate a fresh random 32-byte data key (used once, then stored in keyring).
pub fn generate_data_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}

/// Encrypt `plaintext` with `key`. Returns `(ciphertext, nonce)`.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("bad key length: {e}"))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("encrypt failed: {e}"))?;
    Ok((ciphertext, nonce_bytes.to_vec()))
}

/// Decrypt. Any failure (wrong key, tampering, bad nonce) collapses to one
/// opaque error — no oracle distinction.
pub fn decrypt(key: &[u8; 32], ciphertext: &[u8], nonce: &[u8]) -> Result<Vec<u8>, String> {
    // Guard before `Nonce::from_slice`: that function validates length by
    // panicking, so callers supplying attacker-controlled input would hit a
    // DoS panic. We check here and return an opaque Err instead.
    if nonce.len() != NONCE_LEN {
        return Err("decrypt failed".to_string());
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "decrypt failed".to_string())?;
    let nonce = Nonce::from_slice(nonce);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "decrypt failed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; 32] {
        [0x42u8; 32]
    }

    #[test]
    fn roundtrip() {
        let k = key();
        let (ct, nonce) = encrypt(&k, b"ssh-secret-value").unwrap();
        assert_eq!(decrypt(&k, &ct, &nonce).unwrap(), b"ssh-secret-value");
    }

    #[test]
    fn fresh_nonce_each_call() {
        let k = key();
        let (c1, n1) = encrypt(&k, b"same").unwrap();
        let (c2, n2) = encrypt(&k, b"same").unwrap();
        assert_ne!(n1, n2);
        assert_ne!(c1, c2);
    }

    #[test]
    fn wrong_key_fails() {
        let (ct, nonce) = encrypt(&key(), b"secret").unwrap();
        assert!(decrypt(&[0x01u8; 32], &ct, &nonce).is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let k = key();
        let (mut ct, nonce) = encrypt(&k, b"secret").unwrap();
        ct[0] ^= 0x01;
        assert!(decrypt(&k, &ct, &nonce).is_err());
    }

    #[test]
    fn tampered_nonce_fails() {
        let k = key();
        let (ct, mut nonce) = encrypt(&k, b"secret").unwrap();
        nonce[0] ^= 0x01;
        assert!(decrypt(&k, &ct, &nonce).is_err());
    }

    #[test]
    fn empty_plaintext_roundtrips() {
        let k = key();
        let (ct, nonce) = encrypt(&k, b"").unwrap();
        assert_eq!(ct.len(), 16); // 0 bytes + 16-byte tag
        assert_eq!(decrypt(&k, &ct, &nonce).unwrap(), b"");
    }

    #[test]
    fn generate_data_key_is_random() {
        assert_ne!(generate_data_key(), generate_data_key());
    }
}
