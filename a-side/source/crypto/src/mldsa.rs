// Copyright 2025, The Android Open Source Project
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

//! Pure-Rust implementation of ML-DSA (replaces BoringSSL; `ml-dsa` crate).
use kmr_common::crypto::{self, mldsa::Key, AccumulatingOperation, MlDsa, OpaqueOr};
use kmr_common::{explicit, try_to_vec, Error};
use ml_dsa::{KeyExport as _, MlDsa65, MlDsa87, Seed, SigningKey, VerifyingKey};
use std::boxed::Box;
use std::vec::Vec;

/// [`kmr_common::crypto::MlDsa`] implementation based on the `ml-dsa` crate.
pub struct OmmegaMlDsa;

fn seed_from_key(key: &Key) -> Result<Seed, Error> {
    let arr: [u8; 32] = key
        .private_key_bytes()
        .try_into()
        .map_err(|_| kmr_common::km_err!(InvalidArgument, "ML-DSA seed must be 32 bytes"))?;
    Ok(Seed::from(arr))
}

impl MlDsa for OmmegaMlDsa {
    fn subject_public_key(&self, key: &OpaqueOr<Key>) -> Result<Vec<u8>, Error> {
        let key = explicit!(key)?;
        let seed = seed_from_key(key)?;
        let pk: Vec<u8> = match key.variant() {
            kmr_wire::keymint::MlDsaVariant::MlDsa65 => {
                let sk = SigningKey::<MlDsa65>::from_seed(&seed);
                let vk: VerifyingKey<MlDsa65> = sk.expanded_key().verifying_key();
                vk.to_bytes().as_slice().to_vec()
            }
            kmr_wire::keymint::MlDsaVariant::MlDsa87 => {
                let sk = SigningKey::<MlDsa87>::from_seed(&seed);
                let vk: VerifyingKey<MlDsa87> = sk.expanded_key().verifying_key();
                vk.to_bytes().as_slice().to_vec()
            }
        };
        Ok(pk)
    }

    fn begin_sign(&self, key: OpaqueOr<Key>) -> Result<Box<dyn AccumulatingOperation>, Error> {
        let key = explicit!(key)?;
        let seed = seed_from_key(&key)?;
        let op = match key.variant() {
            kmr_wire::keymint::MlDsaVariant::MlDsa65 => OmmegaMlDsaSignOperation::MlDsa65(
                SigningKey::<MlDsa65>::from_seed(&seed),
                Vec::new(),
            ),
            kmr_wire::keymint::MlDsaVariant::MlDsa87 => OmmegaMlDsaSignOperation::MlDsa87(
                SigningKey::<MlDsa87>::from_seed(&seed),
                Vec::new(),
            ),
        };
        Ok(Box::new(op))
    }
}

/// ML-DSA signing operation based on the `ml-dsa` crate.
enum OmmegaMlDsaSignOperation {
    MlDsa65(SigningKey<MlDsa65>, Vec<u8>),
    MlDsa87(SigningKey<MlDsa87>, Vec<u8>),
}

impl OmmegaMlDsaSignOperation {
    fn data_mut(&mut self) -> &mut Vec<u8> {
        match self {
            Self::MlDsa65(_, data) | Self::MlDsa87(_, data) => data,
        }
    }

    fn sign_and_encode(self) -> Result<Vec<u8>, Error> {
        match self {
            Self::MlDsa65(sk, data) => {
                let sig: ml_dsa::Signature<MlDsa65> = sk
                    .expanded_key()
                    .sign_deterministic(&data, &[])
                    .map_err(|e| kmr_common::km_err!(UnknownError, "ML-DSA-65 sign: {e:?}"))?;
                let enc: ml_dsa::EncodedSignature<MlDsa65> = sig.encode();
                Ok(enc.as_slice().to_vec())
            }
            Self::MlDsa87(sk, data) => {
                let sig: ml_dsa::Signature<MlDsa87> = sk
                    .expanded_key()
                    .sign_deterministic(&data, &[])
                    .map_err(|e| kmr_common::km_err!(UnknownError, "ML-DSA-87 sign: {e:?}"))?;
                let enc: ml_dsa::EncodedSignature<MlDsa87> = sig.encode();
                Ok(enc.as_slice().to_vec())
            }
        }
    }
}

impl crypto::AccumulatingOperation for OmmegaMlDsaSignOperation {
    fn update(&mut self, data: &[u8]) -> Result<(), Error> {
        self.data_mut().extend_from_slice(data);
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<Vec<u8>, Error> {
        let sig = self.sign_and_encode()?;
        try_to_vec(sig.as_slice())
    }
}
