//! Encryption boundary for SOTER private keys at rest.

use std::path::Path;
use std::sync::OnceLock;

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use zeroize::Zeroize;

pub const SEED_CONFIG: &str = "/data/misc/keystore/ommega/config.toml";
const PREFIX: &str = "sealed:v1:";
const HKDF_INFO: &[u8] = b"ommega soter seal v1";

pub struct SealKey([u8; 32]);

impl std::fmt::Debug for SealKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SealKey([REDACTED])")
    }
}

impl Drop for SealKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl SealKey {
    pub fn from_seed(seed: &[u8; 32]) -> SealKey {
        let hkdf = Hkdf::<Sha256>::new(None, seed);
        let mut output = [0u8; 32];
        hkdf.expand(HKDF_INFO, &mut output)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        SealKey(output)
    }

    pub fn load_from(path: &Path) -> Result<SealKey, String> {
        let raw = std::fs::read_to_string(path).map_err(|_| {
            format!(
                "cannot read the SOTER seal configuration file {}",
                path.display()
            )
        })?;
        let document: toml::Table = raw.parse().map_err(|_| {
            format!(
                "the SOTER seal configuration file {} is not valid TOML",
                path.display()
            )
        })?;
        let seed = document
            .get("crypto")
            .and_then(|crypto| crypto.get("root_kek_seed"))
            .and_then(|seed| seed.as_str())
            .ok_or_else(|| {
                format!(
                    "the SOTER seal configuration file {} is missing [crypto].root_kek_seed",
                    path.display()
                )
            })?;
        if seed.chars().count() != 64 {
            return Err(format!(
                "[crypto].root_kek_seed in {} is not 32 bytes of hexadecimal",
                path.display()
            ));
        }
        let mut decoded = hex::decode(seed).map_err(|_| {
            format!(
                "[crypto].root_kek_seed in {} is not 32 bytes of hexadecimal",
                path.display()
            )
        })?;
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&decoded);
        decoded.zeroize();
        let key = SealKey::from_seed(&seed);
        seed.zeroize();
        Ok(key)
    }

    pub fn seal(&self, plaintext: &[u8], aad: &[u8]) -> Result<String, String> {
        let cipher = Aes256Gcm::new_from_slice(&self.0).map_err(|_| "invalid SOTER seal key")?;
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(
                aes_gcm::Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| "failed to seal SOTER private key".to_string())?;
        let mut packed = Vec::with_capacity(nonce.len() + ciphertext.len());
        packed.extend_from_slice(&nonce);
        packed.extend_from_slice(&ciphertext);
        Ok(format!("{}{}", PREFIX, STANDARD.encode(packed)))
    }

    pub fn open(&self, sealed: &str, aad: &[u8]) -> Result<Vec<u8>, String> {
        let encoded = sealed
            .trim()
            .strip_prefix(PREFIX)
            .ok_or_else(|| "sealed value is missing the sealed:v1: prefix".to_string())?;
        let packed = STANDARD
            .decode(encoded)
            .map_err(|_| "sealed value is not valid base64".to_string())?;
        if packed.len() < 12 + 16 {
            return Err("sealed value is too short".to_string());
        }
        let (nonce, ciphertext) = packed.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(&self.0).map_err(|_| "invalid SOTER seal key")?;
        cipher
            .decrypt(
                aes_gcm::Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|_| "sealed value authentication failed".to_string())
    }
}

pub fn active() -> Result<&'static SealKey, String> {
    static ACTIVE: OnceLock<SealKey> = OnceLock::new();
    if let Some(key) = ACTIVE.get() {
        return Ok(key);
    }
    #[cfg(not(test))]
    let loaded = SealKey::load_from(Path::new(SEED_CONFIG))?;
    #[cfg(test)]
    let loaded = SealKey::from_seed(&[0x5a; 32]);
    Ok(ACTIVE.get_or_init(|| loaded))
}

pub fn is_sealed(text: &str) -> bool {
    text.trim_start().starts_with(PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key(byte: u8) -> SealKey {
        SealKey::from_seed(&[byte; 32])
    }

    #[test]
    fn seal_round_trip_and_inputs_are_authenticated() {
        let key = test_key(1);
        let aad = b"test aad";
        let plaintext = b"private material";
        let sealed = key.seal(plaintext, aad).unwrap();
        assert!(key.open(&sealed, aad).unwrap() == plaintext);
        assert!(test_key(2).open(&sealed, aad).is_err());
        assert!(key.open(&sealed, b"other aad").is_err());
        let encoded = sealed.strip_prefix(PREFIX).unwrap();
        let mut packed = STANDARD.decode(encoded).unwrap();
        let last = packed.len() - 1;
        packed[last] ^= 1;
        let flipped = format!("{}{}", PREFIX, STANDARD.encode(&packed));
        assert!(key.open(&flipped, aad).is_err());
        let truncated = format!("{}{}", PREFIX, STANDARD.encode(&packed[..20]));
        assert!(key.open(&truncated, aad).is_err());
        assert!(key.open(plaintext_as_text(), aad).is_err());
    }

    fn plaintext_as_text() -> &'static str {
        "not sealed"
    }

    #[test]
    fn sealing_is_randomized_and_detectable() {
        let key = test_key(1);
        let first = key.seal(b"same", b"aad").unwrap();
        let second = key.seal(b"same", b"aad").unwrap();
        assert!(first != second);
        assert!(is_sealed(&first));
        assert!(is_sealed(&format!("  {first}")));
        assert!(!is_sealed("plain"));
    }

    #[test]
    fn seal_key_load_from_validates_the_config_shape() {
        let directory =
            std::env::temp_dir().join(format!("soter-seal-config-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let valid = directory.join("valid.toml");
        std::fs::write(
            &valid,
            "[crypto]\nroot_kek_seed = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n",
        )
        .unwrap();
        SealKey::load_from(&valid).unwrap_or_else(|error| panic!("valid config rejected: {error}"));
        assert!(SealKey::load_from(&directory.join("missing.toml")).is_err());
        let invalid = directory.join("invalid.toml");
        std::fs::write(&invalid, "[crypto\n").unwrap();
        assert!(SealKey::load_from(&invalid).is_err());
        let missing = directory.join("missing-field.toml");
        std::fs::write(&missing, "[crypto]\n").unwrap();
        assert!(SealKey::load_from(&missing).is_err());
        let short = directory.join("short.toml");
        std::fs::write(&short, "[crypto]\nroot_kek_seed = \"aa\"\n").unwrap();
        assert!(SealKey::load_from(&short).is_err());
        let non_hex = directory.join("non-hex.toml");
        std::fs::write(
            &non_hex,
            "[crypto]\nroot_kek_seed = \"zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz\"\n",
        )
        .unwrap();
        assert!(SealKey::load_from(&non_hex).is_err());
        let _ = std::fs::remove_dir_all(directory);
    }
}
