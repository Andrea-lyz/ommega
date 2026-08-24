// Copyright 2022, The Android Open Source Project
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

//! Pure-Rust implementation of RSA (replaces BoringSSL; RustCrypto `rsa`).
use kmr_common::crypto::{
    rsa::{DecryptionMode, SignMode, PKCS1_UNDIGESTED_SIGNATURE_PADDING_OVERHEAD},
    OpaqueOr,
};
use kmr_common::{crypto, explicit, km_err, try_to_vec, vec_try, Error, FallibleAllocExt};
use kmr_wire::{keymint, keymint::Digest, KeySizeInBits, RsaExponent};
use rand_core::OsRng;
use rsa::pkcs1::{DecodeRsaPrivateKey as _, EncodeRsaPrivateKey as _};
use rsa::traits::{PrivateKeyParts as _, PublicKeyParts as _};
use rsa::{Oaep, Pkcs1v15Sign, Pss, RsaPrivateKey};
use sha1::Sha1;
use sha2::{Sha224, Sha256, Sha384, Sha512};
use std::boxed::Box;
use std::vec::Vec;

/// Smallest allowed public exponent.
const MIN_RSA_EXPONENT: RsaExponent = RsaExponent(3);

/// [`crypto::Rsa`] implementation based on RustCrypto `rsa`.
#[derive(Default)]
pub struct OmmegaRsa {
    _priv: core::marker::PhantomData<()>,
}

impl crypto::Rsa for OmmegaRsa {
    fn generate_key(
        &self,
        _rng: &mut dyn crypto::Rng,
        key_size: KeySizeInBits,
        pub_exponent: RsaExponent,
        _params: &[keymint::KeyParam],
    ) -> Result<crypto::KeyMaterial, Error> {
        if pub_exponent < MIN_RSA_EXPONENT {
            return Err(km_err!(
                InvalidArgument,
                "Invalid public exponent, {:?} < {:?}",
                pub_exponent,
                MIN_RSA_EXPONENT
            ));
        }
        if pub_exponent.0 % 2 != 1 {
            return Err(km_err!(
                InvalidArgument,
                "Invalid public exponent {:?} (even number)",
                pub_exponent
            ));
        }
        let mut rng = rand_core::OsRng;
        let exp = rsa::BigUint::from_bytes_be(&pub_exponent.0.to_be_bytes());
        let rsa_key =
            RsaPrivateKey::new_with_exp(&mut rng, key_size.0 as usize, &exp).map_err(|e| {
                km_err!(
                    UnknownError,
                    "failed to generate RSA key size {key_size:?}: {e:?}"
                )
            })?;
        let asn1_data = rsa_key
            .to_pkcs1_der()
            .map_err(|e| km_err!(UnknownError, "failed to serialize RSA key: {e:?}"))?
            .as_bytes()
            .to_vec();
        Ok(crypto::KeyMaterial::Rsa(crypto::rsa::Key(asn1_data).into()))
    }

    fn begin_decrypt(
        &self,
        key: OpaqueOr<crypto::rsa::Key>,
        mode: DecryptionMode,
    ) -> Result<Box<dyn crypto::AccumulatingOperation>, Error> {
        let key = explicit!(key)?;
        let max_size = key.size();
        Ok(Box::new(OmmegaRsaDecryptOperation {
            key,
            mode,
            pending_input: Vec::new(),
            max_size,
        }))
    }

    fn begin_sign(
        &self,
        key: OpaqueOr<crypto::rsa::Key>,
        mode: SignMode,
    ) -> Result<Box<dyn crypto::AccumulatingOperation>, Error> {
        let key = explicit!(key)?;
        match mode {
            SignMode::NoPadding | SignMode::Pkcs1_1_5Padding(Digest::None) => {
                Ok(Box::new(OmmegaRsaUndigestSignOperation::new(key, mode)?))
            }
            SignMode::Pkcs1_1_5Padding(digest) | SignMode::PssPadding(digest) => {
                let _ = digest;
                Ok(Box::new(OmmegaRsaDigestSignOperation::new(key, mode)?))
            }
        }
    }
}

fn parse_key(key: &crypto::rsa::Key) -> Result<RsaPrivateKey, Error> {
    RsaPrivateKey::from_pkcs1_der(&key.0)
        .map_err(|e| km_err!(UnknownError, "failed to parse RSA key: {e:?}"))
}

/// RSA decryption operation.
pub struct OmmegaRsaDecryptOperation {
    key: crypto::rsa::Key,
    mode: DecryptionMode,
    pending_input: Vec<u8>,
    max_size: usize,
}

impl crypto::AccumulatingOperation for OmmegaRsaDecryptOperation {
    fn max_input_size(&self) -> Option<usize> {
        Some(self.max_size)
    }

    fn update(&mut self, data: &[u8]) -> Result<(), Error> {
        self.pending_input.try_extend_from_slice(data)?;
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<Vec<u8>, Error> {
        let priv_key = parse_key(&self.key)?;
        let ciphertext = &self.pending_input;
        let decrypted = match self.mode {
            DecryptionMode::NoPadding => {
                // Raw RSA decrypt: m = c^d mod n, big-endian zero-padded to key size.
                let n = priv_key.n();
                let c = rsa::BigUint::from_bytes_be(ciphertext);
                let m = c.modpow(priv_key.d(), n);
                let key_len = self.max_size;
                let mut out = vec_try![0; key_len]?;
                let m_bytes = m.to_bytes_be();
                let offset = key_len.saturating_sub(m_bytes.len());
                out[offset..].copy_from_slice(&m_bytes);
                out
            }
            DecryptionMode::OaepPadding {
                msg_digest,
                mgf_digest,
            } => {
                let scheme = oaep_scheme_with_mgf(msg_digest, mgf_digest)?;
                priv_key
                    .decrypt(scheme, ciphertext)
                    .map_err(|e| km_err!(VerificationFailed, "RSA-OAEP decrypt failed: {e:?}"))?
            }
            DecryptionMode::Pkcs1_1_5Padding => {
                // PKCS#1 v1.5 decryption uses Pkcs1v15Encrypt scheme.
                let scheme = rsa::Pkcs1v15Encrypt;
                priv_key
                    .decrypt(scheme, ciphertext)
                    .map_err(|e| km_err!(VerificationFailed, "RSA-PKCS1 decrypt failed: {e:?}"))?
            }
        };
        Ok(decrypted)
    }
}

fn oaep_scheme_with_mgf(msg_digest: Digest, mgf_digest: Digest) -> Result<Oaep, Error> {
    Ok(match (msg_digest, mgf_digest) {
        (Digest::Sha1, Digest::Sha1) => Oaep::new_with_mgf_hash::<Sha1, Sha1>(),
        (Digest::Sha1, Digest::Sha224) => Oaep::new_with_mgf_hash::<Sha1, Sha224>(),
        (Digest::Sha1, Digest::Sha256) => Oaep::new_with_mgf_hash::<Sha1, Sha256>(),
        (Digest::Sha1, Digest::Sha384) => Oaep::new_with_mgf_hash::<Sha1, Sha384>(),
        (Digest::Sha1, Digest::Sha512) => Oaep::new_with_mgf_hash::<Sha1, Sha512>(),
        (Digest::Sha224, Digest::Sha1) => Oaep::new_with_mgf_hash::<Sha224, Sha1>(),
        (Digest::Sha224, Digest::Sha224) => Oaep::new_with_mgf_hash::<Sha224, Sha224>(),
        (Digest::Sha224, Digest::Sha256) => Oaep::new_with_mgf_hash::<Sha224, Sha256>(),
        (Digest::Sha224, Digest::Sha384) => Oaep::new_with_mgf_hash::<Sha224, Sha384>(),
        (Digest::Sha224, Digest::Sha512) => Oaep::new_with_mgf_hash::<Sha224, Sha512>(),
        (Digest::Sha256, Digest::Sha1) => Oaep::new_with_mgf_hash::<Sha256, Sha1>(),
        (Digest::Sha256, Digest::Sha224) => Oaep::new_with_mgf_hash::<Sha256, Sha224>(),
        (Digest::Sha256, Digest::Sha256) => Oaep::new_with_mgf_hash::<Sha256, Sha256>(),
        (Digest::Sha256, Digest::Sha384) => Oaep::new_with_mgf_hash::<Sha256, Sha384>(),
        (Digest::Sha256, Digest::Sha512) => Oaep::new_with_mgf_hash::<Sha256, Sha512>(),
        (Digest::Sha384, Digest::Sha1) => Oaep::new_with_mgf_hash::<Sha384, Sha1>(),
        (Digest::Sha384, Digest::Sha224) => Oaep::new_with_mgf_hash::<Sha384, Sha224>(),
        (Digest::Sha384, Digest::Sha256) => Oaep::new_with_mgf_hash::<Sha384, Sha256>(),
        (Digest::Sha384, Digest::Sha384) => Oaep::new_with_mgf_hash::<Sha384, Sha384>(),
        (Digest::Sha384, Digest::Sha512) => Oaep::new_with_mgf_hash::<Sha384, Sha512>(),
        (Digest::Sha512, Digest::Sha1) => Oaep::new_with_mgf_hash::<Sha512, Sha1>(),
        (Digest::Sha512, Digest::Sha224) => Oaep::new_with_mgf_hash::<Sha512, Sha224>(),
        (Digest::Sha512, Digest::Sha256) => Oaep::new_with_mgf_hash::<Sha512, Sha256>(),
        (Digest::Sha512, Digest::Sha384) => Oaep::new_with_mgf_hash::<Sha512, Sha384>(),
        (Digest::Sha512, Digest::Sha512) => Oaep::new_with_mgf_hash::<Sha512, Sha512>(),
        (_, _) => {
            return Err(km_err!(
                UnsupportedDigest,
                "unsupported OAEP digest msg={msg_digest:?} mgf={mgf_digest:?}"
            ))
        }
    })
}

/// RSA digest signing operation (PKCS1/PSS).
pub struct OmmegaRsaDigestSignOperation {
    key: crypto::rsa::Key,
    mode: SignMode,
    pending_input: Vec<u8>,
}

impl OmmegaRsaDigestSignOperation {
    fn new(key: crypto::rsa::Key, mode: SignMode) -> Result<Self, Error> {
        Ok(Self {
            key,
            mode,
            pending_input: Vec::new(),
        })
    }
}

impl crypto::AccumulatingOperation for OmmegaRsaDigestSignOperation {
    fn update(&mut self, data: &[u8]) -> Result<(), Error> {
        self.pending_input.try_extend_from_slice(data)?;
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<Vec<u8>, Error> {
        let priv_key = parse_key(&self.key)?;
        let digest = match self.mode {
            SignMode::Pkcs1_1_5Padding(d) | SignMode::PssPadding(d) => d,
            _ => return Err(km_err!(UnsupportedPaddingMode, "not a digest sign mode")),
        };
        let hashed = hash_digest(digest, &self.pending_input)?;
        let sig = match self.mode {
            SignMode::Pkcs1_1_5Padding(_) => priv_key
                .sign(Pkcs1v15Sign::new::<Sha256>(), &hashed)
                .map_err(|e| km_err!(UnknownError, "RSA-PKCS1 sign failed: {e:?}"))?,
            SignMode::PssPadding(_) => {
                // RSA-PSS needs a random salt, so a rng is mandatory: the
                // rsa crate's `Pss::sign` returns `InvalidPaddingScheme` when
                // given `None`, which surfaced as UNKNOWN_ERROR (-1000) on
                // RSA-PSS signature operations.
                let mut rng = OsRng;
                priv_key
                    .sign_with_rng(&mut rng, Pss::new::<Sha256>(), &hashed)
                    .map_err(|e| km_err!(UnknownError, "RSA-PSS sign failed: {e:?}"))?
            }
            _ => return Err(km_err!(UnsupportedPaddingMode, "unsupported sign mode")),
        };
        Ok(sig)
    }
}

/// RSA undigested signing operation (NoPadding / Pkcs1_1_5Padding(None)).
pub struct OmmegaRsaUndigestSignOperation {
    key: crypto::rsa::Key,
    left_pad: bool,
    pending_input: Vec<u8>,
    max_size: usize,
}

impl OmmegaRsaUndigestSignOperation {
    fn new(key: crypto::rsa::Key, mode: SignMode) -> Result<Self, Error> {
        let rsa_key = parse_key(&key)?;
        let key_len = rsa_key.size();
        let (left_pad, max_size) = match mode {
            SignMode::NoPadding => (true, key_len),
            SignMode::Pkcs1_1_5Padding(Digest::None) => {
                (false, key_len - PKCS1_UNDIGESTED_SIGNATURE_PADDING_OVERHEAD)
            }
            _ => {
                return Err(km_err!(
                    UnsupportedPaddingMode,
                    "sign undigested mode {:?}",
                    mode
                ))
            }
        };
        Ok(Self {
            key,
            left_pad,
            pending_input: Vec::new(),
            max_size,
        })
    }
}

impl crypto::AccumulatingOperation for OmmegaRsaUndigestSignOperation {
    fn max_input_size(&self) -> Option<usize> {
        Some(self.max_size)
    }

    fn update(&mut self, data: &[u8]) -> Result<(), Error> {
        self.pending_input.try_extend_from_slice(data)?;
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<Vec<u8>, Error> {
        let priv_key = parse_key(&self.key)?;
        let key_len = priv_key.size();
        let input = if self.left_pad {
            zero_pad_left(&self.pending_input, self.max_size)?
        } else {
            self.pending_input.clone()
        };
        let n = priv_key.n().clone();
        let d = priv_key.d().clone();
        let m = rsa::BigUint::from_bytes_be(&input);
        let sig = m.modpow(&d, &n);
        let mut buf = vec_try![0; key_len]?;
        let sig_bytes = sig.to_bytes_be();
        let offset = key_len.saturating_sub(sig_bytes.len());
        buf[offset..].copy_from_slice(&sig_bytes);
        Ok(buf)
    }
}

fn zero_pad_left(data: &[u8], len: usize) -> Result<Vec<u8>, Error> {
    let mut dest = vec_try![0; len]?;
    let padding_len = len - data.len();
    dest[padding_len..].copy_from_slice(data);
    Ok(dest)
}

fn hash_digest(digest: Digest, data: &[u8]) -> Result<Vec<u8>, Error> {
    use sha2::Digest as _;
    let out = match digest {
        Digest::Sha1 => Sha1::digest(data).to_vec(),
        Digest::Sha224 => Sha224::digest(data).to_vec(),
        Digest::Sha256 => Sha256::digest(data).to_vec(),
        Digest::Sha384 => Sha384::digest(data).to_vec(),
        Digest::Sha512 => Sha512::digest(data).to_vec(),
        d => return Err(km_err!(UnsupportedDigest, "unsupported digest {:?}", d)),
    };
    try_to_vec(out.as_slice())
}
