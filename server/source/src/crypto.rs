//! Symmetric encryption for stored private keys, mirroring Django's Fernet
//! scheme (`DeviceServerIdentity.encrypt_private_pem`).
//!
//! Django derives the Fernet key as `base64.urlsafe_b64encode(sha256(SECRET_KEY))`.
//! We do the same: the relay's `RELAY_SECRET_KEY` is hashed with SHA-256 and
//! used as the 32-byte Fernet key.
//!
//! If no secret key is configured, encryption is a no-op (plaintext fallback),
//! preserving backward compatibility with pre-existing plaintext rows.

use base64::Engine;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

static FERNET: OnceLock<fernet::Fernet> = OnceLock::new();

/// Initialize the global Fernet cipher from a secret key string.
pub fn init_fernet(secret_key: &str) {
    if secret_key.is_empty() {
        return;
    }
    let digest = Sha256::digest(secret_key.as_bytes());
    let key_b64 = base64::engine::general_purpose::URL_SAFE.encode(digest);
    if let Some(f) = fernet::Fernet::new(&key_b64) {
        let _ = FERNET.set(f);
    }
}

/// Encrypt a private key PEM. Falls back to plaintext if no key is configured.
pub fn encrypt_private_pem(raw: &str) -> String {
    match FERNET.get() {
        Some(f) => f.encrypt(raw.as_bytes()),
        None => raw.to_string(),
    }
}

/// Decrypt a private key PEM. Falls back to returning the input as-is when
/// decryption fails (e.g. legacy plaintext rows) or no key is configured.
pub fn decrypt_private_pem(cipher: &str) -> String {
    match FERNET.get() {
        Some(f) => f
            .decrypt(cipher)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_else(|_| cipher.to_string()),
        None => cipher.to_string(),
    }
}
