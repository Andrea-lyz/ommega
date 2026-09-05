// Copyright 2020, The Android Open Source Project
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Pure-Rust implementations of the legacy keystore2 crypto helpers, replacing
//! the BoringSSL-backed ones. AES-GCM, HKDF and PBKDF2 retain their wire formats.
//! P-521 private keys are written as Ommega raw scalars; reads also accept the
//! SEC1 DER representation used by the former BoringSSL implementation.

use crate::error::Error;
use crate::zvec::ZVec;
use core::convert::TryFrom;
use elliptic_curve::sec1::ToEncodedPoint as _;
use kmr_common::crypto::Rng;
use std::vec;
use std::vec::Vec;

/// Length of the expected initialization vector.
pub const GCM_IV_LENGTH: usize = 12;
/// Length of the expected AEAD TAG.
pub const TAG_LENGTH: usize = 16;
/// Length of an AES 256 key in bytes.
pub const AES_256_KEY_LENGTH: usize = 32;
/// Length of an AES 128 key in bytes.
pub const AES_128_KEY_LENGTH: usize = 16;
/// Length of the expected salt for key from password generation.
pub const SALT_LENGTH: usize = 16;
/// Length of an HMAC-SHA256 tag in bytes.
pub const HMAC_SHA256_LEN: usize = 32;
/// Length of the GCM tag in bytes.
pub const GCM_TAG_LENGTH: usize = 128 / 8;
/// Length of ECDH P-521 output in bytes.
pub const ECDH_P521_OUTPUT_LEN: usize = 66;

/// Older versions of keystore incorrectly truncated ECDH P-521 outputs to the following length.
pub const LEGACY_TRUNCATED_ECDH_OUTPUT_LEN: usize = 32;

/// AES-GCM encryption result: `(ciphertext, iv, tag)`.
pub type AesGcmEncryption = (Vec<u8>, Vec<u8>, Vec<u8>);

/// Older versions of keystore produced IVs with four extra ignored zero bytes.
pub const LEGACY_IV_LENGTH: usize = 16;

/// Generate an AES256 key, essentially 32 random bytes.
pub fn generate_aes256_key() -> Result<ZVec, Error> {
    let mut key = ZVec::new(AES_256_KEY_LENGTH)?;
    crate::rng::OmmegaRng.fill_bytes(&mut key);
    Ok(key)
}

/// Generate a salt.
pub fn generate_salt() -> Result<Vec<u8>, Error> {
    generate_random_data(SALT_LENGTH)
}

/// Generate random data of the given size.
pub fn generate_random_data(size: usize) -> Result<Vec<u8>, Error> {
    let mut data = vec![0; size];
    crate::rng::OmmegaRng.fill_bytes(&mut data);
    Ok(data)
}

/// Perform HMAC-SHA256.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> Result<Vec<u8>, Error> {
    kmr_common::crypto::hmac_sha256(&crate::hmac::OmmegaHmac, key, msg)
        .map_err(|_| Error::HmacSha256Failed)
}

/// AES-GCM decrypt (ciphertext is NOT authenticated until the tag is verified).
pub fn aes_gcm_decrypt(data: &[u8], iv: &[u8], tag: &[u8], key: &[u8]) -> Result<ZVec, Error> {
    let iv = match iv.len() {
        GCM_IV_LENGTH => iv,
        LEGACY_IV_LENGTH => &iv[..GCM_IV_LENGTH],
        _ => return Err(Error::InvalidIvLength),
    };
    if tag.len() != TAG_LENGTH {
        return Err(Error::InvalidAeadTagLength);
    }
    match key.len() {
        AES_128_KEY_LENGTH | AES_256_KEY_LENGTH => {}
        _ => return Err(Error::InvalidKeyLength),
    }

    let iv_arr: [u8; 12] = iv.try_into().map_err(|_| Error::InvalidIvLength)?;
    let plain = crate::aes::gcm_decrypt_raw(key, &iv_arr, &[], data, tag)
        .map_err(|_| Error::DecryptionFailed)?;
    let mut result = ZVec::new(plain.len())?;
    result.copy_from_slice(&plain);
    Ok(result)
}

/// AES-GCM encrypt.
pub fn aes_gcm_encrypt(plaintext: &[u8], key: &[u8]) -> Result<AesGcmEncryption, Error> {
    let mut iv = vec![0; GCM_IV_LENGTH];
    crate::rng::OmmegaRng.fill_bytes(&mut iv);
    match key.len() {
        AES_128_KEY_LENGTH | AES_256_KEY_LENGTH => {}
        _ => return Err(Error::InvalidKeyLength),
    }
    let iv_arr: [u8; 12] = iv
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidIvLength)?;
    let out = crate::aes::gcm_encrypt_raw(key, &iv_arr, &[], plaintext)
        .map_err(|_| Error::EncryptionFailed)?;
    let split = plaintext.len();
    let (ciphertext, tag) = out.split_at(split);
    Ok((ciphertext.to_vec(), iv, tag.to_vec()))
}

fn pbkdf2(key: &mut [u8], password: &[u8], salt: &[u8]) -> Result<(), Error> {
    let iterations = 8192;
    if key.len() == AES_128_KEY_LENGTH {
        pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, salt, iterations, key)
    } else {
        pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password, salt, iterations, key)
    }
    Ok(())
}

/// A high-entropy synthetic password from which an AES key may be derived.
pub enum Password<'a> {
    /// Borrow an existing byte array.
    Ref(&'a [u8]),
    /// Use an owned ZVec to store the key.
    Owned(ZVec),
}

impl<'a> From<&'a [u8]> for Password<'a> {
    fn from(pw: &'a [u8]) -> Self {
        Self::Ref(pw)
    }
}

impl<'a> Password<'a> {
    fn get_key(&'a self) -> &'a [u8] {
        match self {
            Self::Ref(b) => b,
            Self::Owned(z) => z,
        }
    }

    /// Derive a key from the given password and salt, using PBKDF2 with 8192 iterations.
    pub fn derive_key_pbkdf2(&self, salt: &[u8], out_len: usize) -> Result<ZVec, Error> {
        if salt.len() != SALT_LENGTH {
            return Err(Error::InvalidSaltLength);
        }
        match out_len {
            AES_128_KEY_LENGTH | AES_256_KEY_LENGTH => {}
            _ => return Err(Error::InvalidKeyLength),
        }
        let pw = self.get_key();
        let mut result = ZVec::new(out_len)?;
        pbkdf2(&mut result, pw, salt).map_err(|_| Error::EncryptionFailed)?;
        Ok(result)
    }

    /// Derive a key from the given high-entropy synthetic password and salt, using HKDF.
    pub fn derive_key_hkdf(&self, salt: &[u8], out_len: usize) -> Result<ZVec, Error> {
        let prk = hkdf_extract(self.get_key(), salt)?;
        hkdf_expand(out_len, &prk, &[])
    }

    /// Reproduce the accidental KDF used by older ommega builds.
    #[doc(hidden)]
    pub fn derive_key_ommega_legacy(&self, salt: &[u8], out_len: usize) -> Result<ZVec, Error> {
        let prk = ommega_legacy_kdf_extract(self.get_key(), salt)?;
        ommega_legacy_kdf_expand(out_len, &prk, &[])
    }

    /// Try to make another Password object with the same data.
    pub fn try_clone(&self) -> Result<Password<'static>, Error> {
        Ok(Password::Owned(ZVec::try_from(self.get_key())?))
    }
}

/// HKDF-Extract (SHA-256).
pub fn hkdf_extract(secret: &[u8], salt: &[u8]) -> Result<ZVec, Error> {
    let mut prk = ZVec::new(HMAC_SHA256_LEN)?;
    let (out, _) = hkdf::Hkdf::<sha2::Sha256>::extract(Some(salt), secret);
    prk.copy_from_slice(out.as_slice());
    Ok(prk)
}

/// HKDF-Expand (SHA-256).
pub fn hkdf_expand(out_len: usize, prk: &[u8], info: &[u8]) -> Result<ZVec, Error> {
    let hk = hkdf::Hkdf::<sha2::Sha256>::from_prk(prk).map_err(|_| Error::HKDFExpandFailed)?;
    let mut buf = ZVec::new(out_len)?;
    hk.expand(info, &mut buf)
        .map_err(|_| Error::HKDFExpandFailed)?;
    Ok(buf)
}

/// Reproduce the accidental PBKDF2-based extract used by older ommega builds.
#[doc(hidden)]
pub fn ommega_legacy_kdf_extract(secret: &[u8], salt: &[u8]) -> Result<ZVec, Error> {
    // The old adapter used the whole BoringSSL EVP_MAX_MD_SIZE buffer (64
    // bytes), without the truncation performed by real HKDF-Extract. Keep
    // this historical length only in the read-compatibility KDF.
    let mut buf = ZVec::new(64)?;
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(secret, salt, 1, &mut buf);
    Ok(buf)
}

/// Reproduce the accidental PBKDF2-based expand used by older ommega builds.
#[doc(hidden)]
pub fn ommega_legacy_kdf_expand(out_len: usize, prk: &[u8], info: &[u8]) -> Result<ZVec, Error> {
    let mut buf = ZVec::new(out_len)?;
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(prk, info, 1, &mut buf);
    Ok(buf)
}

/// P-521 ECDH private key used by legacy KeyMaster message encryption.
#[derive(Clone)]
pub struct ECKey(p521::SecretKey);

/// An owned EC_POINT object.
pub struct OwnedECPoint(Vec<u8>);

impl OwnedECPoint {
    /// Get the wrapped EC_POINT object (the encoded point bytes).
    pub fn get_point(&self) -> &[u8] {
        &self.0
    }
}

/// Selects how the ECDH P-521 output is used.
#[derive(Clone, Copy)]
pub enum EcdhComputeKeyVersion {
    /// Use only the first 32 bytes of the ECDH P-521 output.
    LegacyTruncated,
    /// Use the full 66 bytes of the ECDH P-521 output.
    Current,
}

/// Compute ECDH shared secret (P-521).
pub fn ecdh_compute_key(
    pub_key: &[u8],
    priv_key: &ECKey,
    version: EcdhComputeKeyVersion,
) -> Result<ZVec, Error> {
    let peer =
        p521::PublicKey::from_sec1_bytes(pub_key).map_err(|_| Error::ECDHComputeKeyFailed)?;
    let shared = p521::ecdh::diffie_hellman(priv_key.0.to_nonzero_scalar(), peer.as_affine());
    let mut out = ZVec::new(ECDH_P521_OUTPUT_LEN)?;
    let bytes = shared.raw_secret_bytes();
    let pad = ECDH_P521_OUTPUT_LEN.saturating_sub(bytes.len());
    out[pad..pad + bytes.len()].copy_from_slice(bytes);
    match version {
        EcdhComputeKeyVersion::LegacyTruncated => out.reduce_len(LEGACY_TRUNCATED_ECDH_OUTPUT_LEN),
        EcdhComputeKeyVersion::Current => (),
    }
    Ok(out)
}

/// Generate a P-521 EC key.
pub fn ec_key_generate_key() -> Result<ECKey, Error> {
    // Use `SecretKey::random` so the scalar is drawn uniformly in [1, n-1].
    // (Sampling 66 random bytes and calling `from_slice` fails whenever the
    // value is >= the P-521 order n, which is common, not a rare edge case.)
    let mut rng = rand_core::OsRng;
    let scalar = p521::SecretKey::random(&mut rng);
    Ok(ECKey(scalar))
}

/// Marshal a P-521 private key to a 66-byte big-endian scalar.
pub fn ec_key_marshal_private_key(key: &ECKey) -> Result<ZVec, Error> {
    let mut buf = ZVec::new(ECDH_P521_OUTPUT_LEN)?;
    let scalar = key.0.to_bytes();
    let bytes = scalar.as_slice();
    let pad = ECDH_P521_OUTPUT_LEN.saturating_sub(bytes.len());
    buf[pad..].copy_from_slice(bytes);
    Ok(buf)
}

/// Parse an Ommega raw P-521 scalar or a legacy SEC1 DER private key.
pub fn ec_key_parse_private_key(buf: &[u8]) -> Result<ECKey, Error> {
    use der::Decode;

    // Keep all scalar encodings accepted by earlier Ommega versions unchanged.
    if let Ok(scalar) = p521::SecretKey::from_slice(buf) {
        return Ok(ECKey(scalar));
    }
    let encoded =
        sec1::EcPrivateKey::from_der(buf).map_err(|_| Error::ECKEYParsePrivateKeyFailed)?;
    // BoringSSL omitted optional curve/public-key fields for this P-521-only
    // storage format. If a curve is present, it must not name another curve.
    let p521_oid = der::asn1::ObjectIdentifier::new_unwrap("1.3.132.0.35");
    if encoded
        .parameters
        .is_some_and(|parameters| parameters.named_curve() != Some(p521_oid))
    {
        return Err(Error::ECKEYParsePrivateKeyFailed);
    }
    // RustCrypto also verifies any encoded public key against the private scalar.
    let scalar =
        p521::SecretKey::from_sec1_der(buf).map_err(|_| Error::ECKEYParsePrivateKeyFailed)?;
    Ok(ECKey(scalar))
}

/// Return the public key (encoded point) for a P-521 key.
pub fn ec_key_get0_public_key(key: &ECKey) -> &[u8] {
    let pk = key.0.public_key();
    let encoded = pk.as_affine().to_encoded_point(false);
    Box::leak(encoded.as_bytes().to_vec().into_boxed_slice())
}

/// Convert an encoded point to octets (no-op identity wrapper).
pub fn ec_point_point_to_oct(point: &[u8]) -> Result<Vec<u8>, Error> {
    Ok(point.to_vec())
}

/// Parse an encoded point.
pub fn ec_point_oct_to_point(buf: &[u8]) -> Result<OwnedECPoint, Error> {
    Ok(OwnedECPoint(buf.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use der::Encode;

    fn scalar(value: u8) -> [u8; ECDH_P521_OUTPUT_LEN] {
        let mut bytes = [0; ECDH_P521_OUTPUT_LEN];
        bytes[ECDH_P521_OUTPUT_LEN - 1] = value;
        bytes
    }

    #[test]
    fn legacy_sec1_and_raw_scalar_have_the_same_public_key() {
        let bytes = scalar(1);
        let raw = ec_key_parse_private_key(&bytes).unwrap();
        for parameters in [
            None,
            Some(sec1::EcParameters::NamedCurve(
                der::asn1::ObjectIdentifier::new_unwrap("1.3.132.0.35"),
            )),
        ] {
            let encoded = sec1::EcPrivateKey {
                private_key: &bytes,
                parameters,
                public_key: None,
            }
            .to_der()
            .unwrap();
            let parsed = ec_key_parse_private_key(&encoded).unwrap();
            assert_eq!(raw.0.public_key(), parsed.0.public_key());
            // No migration of the existing write format.
            assert_eq!(&ec_key_marshal_private_key(&parsed).unwrap()[..], &bytes);
        }
    }

    #[test]
    fn legacy_sec1_rejects_other_curves_and_inconsistent_public_keys() {
        let bytes = scalar(1);
        let other = p521::SecretKey::from_slice(&scalar(2)).unwrap();
        let other_public = other.public_key().to_encoded_point(false);
        for (parameters, public_key) in [
            (
                Some(sec1::EcParameters::NamedCurve(
                    der::asn1::ObjectIdentifier::new_unwrap("1.3.132.0.34"),
                )),
                None,
            ),
            (None, Some(other_public.as_bytes())),
        ] {
            let encoded = sec1::EcPrivateKey {
                private_key: &bytes,
                parameters,
                public_key,
            }
            .to_der()
            .unwrap();
            assert!(ec_key_parse_private_key(&encoded).is_err());
        }
    }

    #[test]
    fn legacy_sec1_rejects_invalid_scalar_and_trailing_data() {
        for bytes in [scalar(0), [0xff; ECDH_P521_OUTPUT_LEN]] {
            let encoded = sec1::EcPrivateKey {
                private_key: &bytes,
                parameters: None,
                public_key: None,
            }
            .to_der()
            .unwrap();
            assert!(ec_key_parse_private_key(&bytes).is_err());
            assert!(ec_key_parse_private_key(&encoded).is_err());
        }
        let bytes = scalar(1);
        let mut encoded = sec1::EcPrivateKey {
            private_key: &bytes,
            parameters: None,
            public_key: None,
        }
        .to_der()
        .unwrap();
        encoded.push(0);
        assert!(ec_key_parse_private_key(&encoded).is_err());
    }
}
