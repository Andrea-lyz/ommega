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

//! Pure-Rust implementation of AES-CMAC (replaces BoringSSL; RustCrypto `cmac`).
use cmac::{Cmac, Mac as _};
use kmr_common::{crypto, crypto::OpaqueOr, explicit, km_err, Error};
use std::boxed::Box;
use std::vec::Vec;

/// [`crypto::AesCmac`] implementation backed by RustCrypto `cmac`.
pub struct OmmegaAesCmac;

impl crypto::AesCmac for OmmegaAesCmac {
    fn begin(
        &self,
        key: OpaqueOr<crypto::aes::Key>,
    ) -> Result<Box<dyn crypto::AccumulatingOperation>, Error> {
        let key = explicit!(key)?;
        let op = match &key {
            crypto::aes::Key::Aes128(k) => CmacImpl::Aes128(
                Cmac::<aes::Aes128>::new_from_slice(k)
                    .map_err(|_| km_err!(UnknownError, "invalid AES-128-CMAC key"))?,
            ),
            crypto::aes::Key::Aes192(k) => CmacImpl::Aes192(
                Cmac::<aes::Aes192>::new_from_slice(k)
                    .map_err(|_| km_err!(UnknownError, "invalid AES-192-CMAC key"))?,
            ),
            crypto::aes::Key::Aes256(k) => CmacImpl::Aes256(
                Cmac::<aes::Aes256>::new_from_slice(k)
                    .map_err(|_| km_err!(UnknownError, "invalid AES-256-CMAC key"))?,
            ),
        };
        Ok(Box::new(OmmegaAesCmacOperation { inner: op }))
    }
}

/// AES-CMAC operation backed by RustCrypto `cmac`.
pub struct OmmegaAesCmacOperation {
    inner: CmacImpl,
}

enum CmacImpl {
    Aes128(Cmac<aes::Aes128>),
    Aes192(Cmac<aes::Aes192>),
    Aes256(Cmac<aes::Aes256>),
}

impl CmacImpl {
    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Aes128(m) => m.update(data),
            Self::Aes192(m) => m.update(data),
            Self::Aes256(m) => m.update(data),
        }
    }

    fn finalize(self) -> Vec<u8> {
        let bytes = match self {
            Self::Aes128(m) => m.finalize().into_bytes(),
            Self::Aes192(m) => m.finalize().into_bytes(),
            Self::Aes256(m) => m.finalize().into_bytes(),
        };
        bytes.to_vec()
    }
}

impl crypto::AccumulatingOperation for OmmegaAesCmacOperation {
    fn update(&mut self, data: &[u8]) -> Result<(), Error> {
        self.inner.update(data);
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<Vec<u8>, Error> {
        let out = self.inner.finalize();
        if out.len() != crypto::aes::BLOCK_SIZE {
            return Err(km_err!(
                BoringSslError,
                "unexpected CMAC output size of {}",
                out.len()
            ));
        }
        Ok(out)
    }
}
