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

//! Pure-Rust implementation of HMAC (replaces BoringSSL; RustCrypto `hmac`).
//! Also provides HKDF via the default `impl<T: Hmac> Hkdf for T`.

use hmac::{Hmac, Mac as _};
use kmr_common::{crypto, crypto::OpaqueOr, explicit, km_err, try_to_vec, Error};
use kmr_wire::keymint::Digest;
use md5::Md5;
use sha1::Sha1;
use sha2::{Sha224, Sha256, Sha384, Sha512};
use std::boxed::Box;
use std::vec::Vec;

/// [`crypto::Hmac`] implementation backed by RustCrypto `hmac`.
pub struct OmmegaHmac;

impl crypto::Hmac for OmmegaHmac {
    fn begin(
        &self,
        key: OpaqueOr<crypto::hmac::Key>,
        digest: Digest,
    ) -> Result<Box<dyn crypto::AccumulatingOperation>, Error> {
        let key = explicit!(key)?;
        Ok(Box::new(OmmegaHmacOperation {
            inner: match digest {
                Digest::Md5 => HmacImpl::Md5(
                    Hmac::<Md5>::new_from_slice(&key.0)
                        .map_err(|_| km_err!(UnknownError, "invalid HMAC-MD5 key"))?,
                ),
                Digest::Sha1 => HmacImpl::Sha1(
                    Hmac::<Sha1>::new_from_slice(&key.0)
                        .map_err(|_| km_err!(UnknownError, "invalid HMAC-SHA1 key"))?,
                ),
                Digest::Sha224 => HmacImpl::Sha224(
                    Hmac::<Sha224>::new_from_slice(&key.0)
                        .map_err(|_| km_err!(UnknownError, "invalid HMAC-SHA224 key"))?,
                ),
                Digest::Sha256 => HmacImpl::Sha256(
                    Hmac::<Sha256>::new_from_slice(&key.0)
                        .map_err(|_| km_err!(UnknownError, "invalid HMAC-SHA256 key"))?,
                ),
                Digest::Sha384 => HmacImpl::Sha384(
                    Hmac::<Sha384>::new_from_slice(&key.0)
                        .map_err(|_| km_err!(UnknownError, "invalid HMAC-SHA384 key"))?,
                ),
                Digest::Sha512 => HmacImpl::Sha512(
                    Hmac::<Sha512>::new_from_slice(&key.0)
                        .map_err(|_| km_err!(UnknownError, "invalid HMAC-SHA512 key"))?,
                ),
                d => return Err(km_err!(UnsupportedDigest, "unknown digest {:?}", d)),
            },
        }))
    }
}

/// HMAC operation backed by RustCrypto `hmac`.
pub struct OmmegaHmacOperation {
    inner: HmacImpl,
}

enum HmacImpl {
    Md5(Hmac<Md5>),
    Sha1(Hmac<Sha1>),
    Sha224(Hmac<Sha224>),
    Sha256(Hmac<Sha256>),
    Sha384(Hmac<Sha384>),
    Sha512(Hmac<Sha512>),
}

impl HmacImpl {
    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Md5(h) => h.update(data),
            Self::Sha1(h) => h.update(data),
            Self::Sha224(h) => h.update(data),
            Self::Sha256(h) => h.update(data),
            Self::Sha384(h) => h.update(data),
            Self::Sha512(h) => h.update(data),
        }
    }

    fn finalize(self) -> Vec<u8> {
        match self {
            Self::Md5(h) => h.finalize().into_bytes().as_slice().to_vec(),
            Self::Sha1(h) => h.finalize().into_bytes().as_slice().to_vec(),
            Self::Sha224(h) => h.finalize().into_bytes().as_slice().to_vec(),
            Self::Sha256(h) => h.finalize().into_bytes().as_slice().to_vec(),
            Self::Sha384(h) => h.finalize().into_bytes().as_slice().to_vec(),
            Self::Sha512(h) => h.finalize().into_bytes().as_slice().to_vec(),
        }
    }
}

impl crypto::AccumulatingOperation for OmmegaHmacOperation {
    fn update(&mut self, data: &[u8]) -> Result<(), Error> {
        self.inner.update(data);
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<Vec<u8>, Error> {
        try_to_vec(self.inner.finalize().as_slice())
    }
}

// The `TryInto` import is required by the `hmac` crate's bounds on some versions;
// keep it referenced to avoid a spurious unused-import warning.
#[allow(unused_imports)]
use core::convert::TryInto as _;
