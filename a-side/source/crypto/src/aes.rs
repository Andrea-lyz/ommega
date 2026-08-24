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

//! Pure-Rust implementation of AES (replaces BoringSSL; RustCrypto `aes`).
//!
//! ECB/CBC/CTR are implemented with `aes` block cipher + manual block chaining;
//! GCM uses the `aes-gcm` crate with a buffering scheme (inputs are accumulated
//! and processed at `finish`), which is correct for the (small) data sizes KeyMint
//! deals with.

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit as _, KeyIvInit as _, StreamCipher as _};
use aes::{Aes128, Aes192, Aes256};
use ctr::Ctr128BE;
use kmr_common::{crypto, crypto::OpaqueOr, explicit, km_err, vec_try, Error};
use std::boxed::Box;
use std::vec::Vec;

const BLOCK: usize = crypto::aes::BLOCK_SIZE;

/// [`crypto::Aes`] implementation backed by RustCrypto `aes`.
pub struct OmmegaAes;

trait BlockCipher: Send {
    fn encrypt_block(&self, block: &mut [u8; BLOCK]);
    fn decrypt_block(&self, block: &mut [u8; BLOCK]);
}

macro_rules! impl_block_cipher {
    ($t:ty) => {
        impl BlockCipher for $t {
            fn encrypt_block(&self, block: &mut [u8; BLOCK]) {
                let b = cipher::generic_array::GenericArray::from_mut_slice(block);
                BlockEncrypt::encrypt_block(self, b);
            }
            fn decrypt_block(&self, block: &mut [u8; BLOCK]) {
                let b = cipher::generic_array::GenericArray::from_mut_slice(block);
                BlockDecrypt::decrypt_block(self, b);
            }
        }
    };
}
impl_block_cipher!(Aes128);
impl_block_cipher!(Aes192);
impl_block_cipher!(Aes256);

fn block_cipher(key: &crypto::aes::Key) -> Box<dyn BlockCipher> {
    match key {
        crypto::aes::Key::Aes128(k) => Box::new(Aes128::new_from_slice(k).expect("key len")),
        crypto::aes::Key::Aes192(k) => Box::new(Aes192::new_from_slice(k).expect("key len")),
        crypto::aes::Key::Aes256(k) => Box::new(Aes256::new_from_slice(k).expect("key len")),
    }
}

fn key_bytes(key: &crypto::aes::Key) -> &[u8] {
    match key {
        crypto::aes::Key::Aes128(k) => &k[..],
        crypto::aes::Key::Aes192(k) => &k[..],
        crypto::aes::Key::Aes256(k) => &k[..],
    }
}

fn pkcs7_pad(data: &[u8]) -> Vec<u8> {
    let pad = BLOCK - data.len() % BLOCK;
    let mut out = data.to_vec();
    out.extend(std::iter::repeat_n(pad as u8, pad));
    out
}

fn pkcs7_unpad(data: &[u8]) -> Result<Vec<u8>, Error> {
    let &pad = data
        .last()
        .ok_or_else(|| km_err!(InvalidInputLength, "empty data"))?;
    let pad = pad as usize;
    if pad == 0 || pad > data.len() || !data.len().is_multiple_of(BLOCK) {
        return Err(km_err!(InvalidInputLength, "bad PKCS#7 padding"));
    }
    let start = data.len() - pad;
    if data[start..].iter().any(|&b| b as usize != pad) {
        return Err(km_err!(InvalidInputLength, "bad PKCS#7 padding bytes"));
    }
    Ok(data[..start].to_vec())
}

/// AES plain operation (ECB/CBC/CTR), buffering until finish.
pub struct OmmegaAesOperation {
    mode: crypto::aes::CipherMode,
    dir: crypto::SymmetricOperation,
    cipher: Box<dyn BlockCipher>,
    /// Original key bytes (for CTR re-keying).
    key: Vec<u8>,
    data: Vec<u8>,
}

impl OmmegaAesOperation {
    fn run_blocks(&self, input: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        match self.mode {
            crypto::aes::CipherMode::EcbNoPadding | crypto::aes::CipherMode::EcbPkcs7Padding => {
                for chunk in input.as_chunks::<16>().0 {
                    let mut blk = [0u8; BLOCK];
                    blk.copy_from_slice(chunk);
                    match self.dir {
                        crypto::SymmetricOperation::Encrypt => self.cipher.encrypt_block(&mut blk),
                        crypto::SymmetricOperation::Decrypt => self.cipher.decrypt_block(&mut blk),
                    }
                    out.extend_from_slice(&blk);
                }
            }
            crypto::aes::CipherMode::CbcNoPadding { nonce }
            | crypto::aes::CipherMode::CbcPkcs7Padding { nonce } => {
                let mut prev: [u8; BLOCK] = nonce;
                for chunk in input.as_chunks::<16>().0 {
                    let mut blk = [0u8; BLOCK];
                    blk.copy_from_slice(chunk);
                    match self.dir {
                        crypto::SymmetricOperation::Encrypt => {
                            for i in 0..BLOCK {
                                blk[i] ^= prev[i];
                            }
                            self.cipher.encrypt_block(&mut blk);
                            prev = blk;
                        }
                        crypto::SymmetricOperation::Decrypt => {
                            let orig = blk;
                            self.cipher.decrypt_block(&mut blk);
                            for i in 0..BLOCK {
                                blk[i] ^= prev[i];
                            }
                            prev = orig;
                        }
                    }
                    out.extend_from_slice(&blk);
                }
            }
            crypto::aes::CipherMode::Ctr { .. } => {
                unreachable!("CTR handled as a stream")
            }
        }
        out
    }
}

impl crypto::EmittingOperation for OmmegaAesOperation {
    fn update(&mut self, data: &[u8]) -> Result<Vec<u8>, Error> {
        self.data.extend_from_slice(data);
        Ok(Vec::new())
    }

    fn finish(self: Box<Self>) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        match self.mode {
            crypto::aes::CipherMode::EcbNoPadding
            | crypto::aes::CipherMode::CbcNoPadding { .. } => {
                if !self.data.is_empty() && !self.data.len().is_multiple_of(16) {
                    return Err(km_err!(
                        InvalidInputLength,
                        "input length {} not a multiple of block size",
                        self.data.len()
                    ));
                }
                out.extend_from_slice(&self.run_blocks(&self.data));
            }
            crypto::aes::CipherMode::EcbPkcs7Padding
            | crypto::aes::CipherMode::CbcPkcs7Padding { .. } => match self.dir {
                crypto::SymmetricOperation::Encrypt => {
                    let padded = pkcs7_pad(&self.data);
                    out.extend_from_slice(&self.run_blocks(&padded));
                }
                crypto::SymmetricOperation::Decrypt => {
                    let dec = self.run_blocks(&self.data);
                    out.extend_from_slice(&pkcs7_unpad(&dec)?);
                }
            },
            crypto::aes::CipherMode::Ctr { nonce } => {
                let mut nonce12 = [0u8; 12];
                nonce12.copy_from_slice(&nonce[..12]);
                out = ctr_transform_keyed_bytes(&self.key, &nonce12, &self.data)?;
            }
        }
        Ok(out)
    }
}

/// CTR-transform data with the given raw key bytes and 12-byte nonce. Uses AES-128,
/// AES-192 or AES-256 based on key length.
fn ctr_transform_keyed_bytes(key: &[u8], nonce: &[u8; 12], data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut iv = [0u8; 16];
    iv[..12].copy_from_slice(nonce);
    let mut ks = vec_try![0; data.len()]?;
    let apply = |out: &mut [u8]| -> Result<(), Error> {
        match key.len() {
            16 => {
                let mut c = Ctr128BE::<Aes128>::new_from_slices(key, &iv)
                    .map_err(|_| km_err!(UnknownError, "invalid AES-128-CTR key"))?;
                c.apply_keystream(out);
            }
            24 => {
                let mut c = Ctr128BE::<Aes192>::new_from_slices(key, &iv)
                    .map_err(|_| km_err!(UnknownError, "invalid AES-192-CTR key"))?;
                c.apply_keystream(out);
            }
            32 => {
                let mut c = Ctr128BE::<Aes256>::new_from_slices(key, &iv)
                    .map_err(|_| km_err!(UnknownError, "invalid AES-256-CTR key"))?;
                c.apply_keystream(out);
            }
            _ => return Err(km_err!(UnknownError, "invalid CTR key length")),
        }
        Ok(())
    };
    apply(&mut ks)?;
    for (o, d) in ks.iter_mut().zip(data.iter()) {
        *o ^= d;
    }
    Ok(ks)
}

/// AES-GCM encrypt operation (buffers AAD + data, processes at finish).
pub struct OmmegaAesGcmEncryptOperation {
    tag_len: usize,
    aad: Vec<u8>,
    data: Vec<u8>,
    key: Vec<u8>,
    nonce: [u8; 12],
}

/// AES-GCM decrypt operation (buffers AAD + data, processes at finish).
pub struct OmmegaAesGcmDecryptOperation {
    tag_len: usize,
    aad: Vec<u8>,
    data: Vec<u8>,
    key: Vec<u8>,
    nonce: [u8; 12],
}

impl crypto::Aes for OmmegaAes {
    fn begin(
        &self,
        key: OpaqueOr<crypto::aes::Key>,
        mode: crypto::aes::CipherMode,
        dir: crypto::SymmetricOperation,
    ) -> Result<Box<dyn crypto::EmittingOperation>, Error> {
        let key = explicit!(key)?;
        let cipher = block_cipher(&key);
        let key_bytes = key_bytes(&key).to_vec();
        Ok(Box::new(OmmegaAesOperation {
            mode,
            dir,
            cipher,
            key: key_bytes,
            data: Vec::new(),
        }))
    }

    fn begin_aead(
        &self,
        key: OpaqueOr<crypto::aes::Key>,
        mode: crypto::aes::GcmMode,
        dir: crypto::SymmetricOperation,
    ) -> Result<Box<dyn crypto::AadOperation>, Error> {
        let key = explicit!(key)?;
        let nonce: [u8; 12] = match mode {
            crypto::aes::GcmMode::GcmTag12 { nonce }
            | crypto::aes::GcmMode::GcmTag13 { nonce }
            | crypto::aes::GcmMode::GcmTag14 { nonce }
            | crypto::aes::GcmMode::GcmTag15 { nonce }
            | crypto::aes::GcmMode::GcmTag16 { nonce } => nonce,
        };
        let tag_len = mode.tag_len();
        let key_bytes = key_bytes(&key).to_vec();
        match dir {
            crypto::SymmetricOperation::Encrypt => Ok(Box::new(OmmegaAesGcmEncryptOperation {
                tag_len,
                aad: Vec::new(),
                data: Vec::new(),
                key: key_bytes,
                nonce,
            })),
            crypto::SymmetricOperation::Decrypt => Ok(Box::new(OmmegaAesGcmDecryptOperation {
                tag_len,
                aad: Vec::new(),
                data: Vec::new(),
                key: key_bytes,
                nonce,
            })),
        }
    }
}

impl crypto::AadOperation for OmmegaAesGcmEncryptOperation {
    fn update_aad(&mut self, aad: &[u8]) -> Result<(), Error> {
        self.aad.extend_from_slice(aad);
        Ok(())
    }
}

impl crypto::EmittingOperation for OmmegaAesGcmEncryptOperation {
    fn update(&mut self, data: &[u8]) -> Result<Vec<u8>, Error> {
        self.data.extend_from_slice(data);
        Ok(Vec::new())
    }

    fn finish(self: Box<Self>) -> Result<Vec<u8>, Error> {
        if self.tag_len != 16 {
            return Err(km_err!(
                InvalidArgument,
                "only 16-byte GCM tags supported, requested {}",
                self.tag_len
            ));
        }
        gcm_encrypt(&self.key, &self.nonce, &self.aad, &self.data)
    }
}

impl crypto::AadOperation for OmmegaAesGcmDecryptOperation {
    fn update_aad(&mut self, aad: &[u8]) -> Result<(), Error> {
        self.aad.extend_from_slice(aad);
        Ok(())
    }
}

impl crypto::EmittingOperation for OmmegaAesGcmDecryptOperation {
    fn update(&mut self, data: &[u8]) -> Result<Vec<u8>, Error> {
        self.data.extend_from_slice(data);
        Ok(Vec::new())
    }

    fn finish(self: Box<Self>) -> Result<Vec<u8>, Error> {
        if self.tag_len != 16 {
            return Err(km_err!(
                InvalidArgument,
                "only 16-byte GCM tags supported, requested {}",
                self.tag_len
            ));
        }
        // The input is ciphertext || tag (16 bytes at the end).
        if self.data.len() < self.tag_len {
            return Err(km_err!(InvalidTag, "ciphertext too short for tag"));
        }
        let split = self.data.len() - self.tag_len;
        let ciphertext = &self.data[..split];
        let tag = &self.data[split..];
        gcm_decrypt(&self.key, &self.nonce, &self.aad, ciphertext, tag)
    }
}

fn gcm_encrypt(
    key: &[u8],
    nonce: &[u8; 12],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, Error> {
    gcm_encrypt_raw(key, nonce, aad, plaintext)
}

fn gcm_decrypt(
    key: &[u8],
    nonce: &[u8; 12],
    aad: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
) -> Result<Vec<u8>, Error> {
    gcm_decrypt_raw(key, nonce, aad, ciphertext, tag)
}

/// Public AES-GCM encrypt helper (used by `km`). Emits `ciphertext || 16-byte tag`.
pub(crate) fn gcm_encrypt_raw(
    key: &[u8],
    nonce: &[u8; 12],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, Error> {
    let alg = gcm_alg(key.len())?;
    let key = ring::aead::UnboundKey::new(alg, key)
        .map_err(|_| km_err!(UnknownError, "invalid GCM key"))?;
    let sealing = ring::aead::LessSafeKey::new(key);
    let nonce = ring::aead::Nonce::assume_unique_for_key(*nonce);
    let aad = ring::aead::Aad::from(aad);
    let mut in_out = plaintext.to_vec();
    sealing
        .seal_in_place_separate_tag(nonce, aad, &mut in_out)
        .map(|tag| {
            let mut out = in_out;
            out.extend_from_slice(tag.as_ref());
            out
        })
        .map_err(|_| km_err!(VerificationFailed, "GCM encrypt failed"))
}

/// Public AES-GCM decrypt helper (used by `km`). `ciphertext` and `tag` are separate.
pub(crate) fn gcm_decrypt_raw(
    key: &[u8],
    nonce: &[u8; 12],
    aad: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
) -> Result<Vec<u8>, Error> {
    let alg = gcm_alg(key.len())?;
    let key = ring::aead::UnboundKey::new(alg, key)
        .map_err(|_| km_err!(UnknownError, "invalid GCM key"))?;
    let opening = ring::aead::LessSafeKey::new(key);
    let nonce = ring::aead::Nonce::assume_unique_for_key(*nonce);
    let aad = ring::aead::Aad::from(aad);
    let mut in_out = ciphertext.to_vec();
    in_out.extend_from_slice(tag);
    opening
        .open_in_place(nonce, aad, &mut in_out)
        .map(|pt| pt.to_vec())
        .map_err(|_| km_err!(VerificationFailed, "GCM decrypt failed"))
}

fn gcm_alg(key_len: usize) -> Result<&'static ring::aead::Algorithm, Error> {
    match key_len {
        16 => Ok(&ring::aead::AES_128_GCM),
        24 => Err(km_err!(UnknownError, "AES-192-GCM unsupported")),
        32 => Ok(&ring::aead::AES_256_GCM),
        _ => Err(km_err!(UnknownError, "invalid GCM key length")),
    }
}
