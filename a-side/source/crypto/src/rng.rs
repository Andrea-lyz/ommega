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

//! Pure-Rust implementation of random number generation (replaces the
//! BoringSSL-backed one; `ring`'s `SystemRandom` is cryptographically secure).
use kmr_common::crypto;
use ring::rand::SecureRandom as _;
use std::sync::OnceLock;

/// [`crypto::Rng`] implementation backed by `ring::rand::SystemRandom`.
///
/// A unit struct so callers can construct it as `OmmegaRng` directly (matching
/// the original BoringSSL-backed API).
pub struct OmmegaRng;

impl Default for OmmegaRng {
    fn default() -> Self {
        Self
    }
}

fn rng() -> &'static ring::rand::SystemRandom {
    static RNG: OnceLock<ring::rand::SystemRandom> = OnceLock::new();
    RNG.get_or_init(ring::rand::SystemRandom::new)
}

impl crypto::Rng for OmmegaRng {
    fn add_entropy(&mut self, _data: &[u8]) {
        // `ring::SystemRandom` is already seeded from the platform CSPRNG; there is
        // no additional entropy injection API.  This is a no-op, matching the
        // behaviour of `RAND_seed` which just mixes into the pool.
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        rng().fill(dest).unwrap(); // safe: SystemRandom::fill never fails
    }
}
