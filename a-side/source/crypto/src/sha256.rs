// Copyright 2023, The Android Open Source Project
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

//! Pure-Rust implementation of SHA-256 (replaces BoringSSL).
use kmr_common::{crypto, Error};
use sha2::{Digest as _, Sha256};

/// [`crypto::Sha256`] implementation (pure Rust, `sha2`).
pub struct OmmegaSha256;

impl crypto::Sha256 for OmmegaSha256 {
    fn hash(&self, data: &[u8]) -> Result<[u8; 32], Error> {
        let mut hasher = Sha256::new();
        hasher.update(data);
        Ok(hasher.finalize().into())
    }
}
