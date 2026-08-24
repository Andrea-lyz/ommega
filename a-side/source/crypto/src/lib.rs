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

//! Implementations of [`kmr_common::crypto`] traits backed by pure-Rust
//! cryptographic crates (`ring`, RustCrypto), replacing the original
//! BoringSSL-based implementations.

extern crate std;

use kmr_common::crypto;

pub mod aes;
pub mod aes_cmac;
pub mod des;
pub mod ec;
pub mod eq;
pub mod error;
pub mod hmac;
pub mod km;
pub mod mldsa;
pub mod rng;
pub mod rsa;
pub mod sha256;
pub mod zvec;

/// Return a collection of pure-Rust cryptographic trait implementations (together
/// with the provided RNG and clock implementations).
pub fn implementation(
    rng: Box<dyn crypto::Rng>,
    clock: Box<dyn crypto::MonotonicClock>,
) -> crypto::Implementation {
    crypto::Implementation {
        rng,
        clock: Some(clock),
        compare: Box::new(eq::OmmegaEq),
        aes: Box::new(aes::OmmegaAes),
        des: Box::new(des::OmmegaDes),
        hmac: Box::new(hmac::OmmegaHmac),
        rsa: Box::<rsa::OmmegaRsa>::default(),
        ec: Box::<ec::OmmegaEc>::default(),
        ckdf: Box::new(aes_cmac::OmmegaAesCmac),
        hkdf: Box::new(hmac::OmmegaHmac),
        sha256: Box::new(sha256::OmmegaSha256),
        mldsa: Box::new(mldsa::OmmegaMlDsa),
    }
}
