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

use super::*;
use kmr_common::crypto::rsa::Key as RsaKey;
use kmr_common::crypto::Rsa as _;
use rsa::rand_core::OsRng;
use rsa::{RsaPrivateKey, RsaPublicKey};

const MESSAGE: &[u8] = b"ommega digest signing";

/// A PKCS#1 DER private key in the form the TA stores, plus the public key used
/// to check the signatures.
fn test_key() -> (Vec<u8>, RsaPublicKey) {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).expect("generate RSA key");
    let public = RsaPublicKey::from(&private);
    let der = private.to_pkcs1_der().expect("encode PKCS#1 DER");
    (der.as_bytes().to_vec(), public)
}

fn sign(imp: &OmmegaRsa, der: &[u8], mode: SignMode) -> Result<Vec<u8>, Error> {
    let mut op = imp.begin_sign(RsaKey(der.to_vec()).into(), mode)?;
    op.update(MESSAGE)?;
    op.finish()
}

/// A hardcoded SHA-256 padding scheme made the `rsa` backend reject every other
/// digest with `InputNotHashed`, which reached callers as UNKNOWN_ERROR (-1000):
/// an RSA key created with SHA-384 or SHA-512 could not sign at all while
/// SHA-256 kept working, and applications do use the other digests.
#[test]
fn pkcs1_sign_uses_the_key_digest() {
    let imp = OmmegaRsa::default();
    let (der, public) = test_key();
    for digest in [
        Digest::Sha1,
        Digest::Sha224,
        Digest::Sha256,
        Digest::Sha384,
        Digest::Sha512,
    ] {
        let signature = sign(&imp, &der, SignMode::Pkcs1_1_5Padding(digest))
            .unwrap_or_else(|e| panic!("PKCS#1 v1.5 signing with {digest:?} failed: {e:?}"));
        let hashed = hash_digest(digest, MESSAGE).expect("digest");
        public
            .verify(
                pkcs1v15_scheme(digest).expect("scheme"),
                &hashed,
                &signature,
            )
            .unwrap_or_else(|e| panic!("PKCS#1 v1.5 {digest:?} signature must verify: {e:?}"));
    }
}

#[test]
fn pss_sign_uses_the_key_digest() {
    let imp = OmmegaRsa::default();
    let (der, public) = test_key();
    for digest in [Digest::Sha1, Digest::Sha256, Digest::Sha384, Digest::Sha512] {
        let signature = sign(&imp, &der, SignMode::PssPadding(digest))
            .unwrap_or_else(|e| panic!("RSA-PSS signing with {digest:?} failed: {e:?}"));
        let hashed = hash_digest(digest, MESSAGE).expect("digest");
        public
            .verify(pss_scheme(digest).expect("scheme"), &hashed, &signature)
            .unwrap_or_else(|e| panic!("RSA-PSS {digest:?} signature must verify: {e:?}"));
    }
}
