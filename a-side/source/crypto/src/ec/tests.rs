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
use crate::rng::OmmegaRng;
use kmr_common::crypto::Ec as _;
use p256::ecdsa::signature::hazmat::PrehashVerifier as _;
use p256::ecdsa::SigningKey as SkP256;
use p384::ecdsa::SigningKey as SkP384;

const MESSAGE: &[u8] = b"ommega ecdsa signing";

/// A NIST key in the stored (SEC1 DER) form together with its private scalar.
fn test_key(curve: ec::NistCurve) -> (ec::Key, Vec<u8>) {
    let imp = OmmegaEc::default();
    let mut rng = OmmegaRng;
    let material = imp
        .generate_nist_key(&mut rng, curve, &[])
        .expect("generate NIST key");
    let crypto::KeyMaterial::Ec(_, _, key) = material else {
        panic!("expected EC key material");
    };
    let key = match key {
        OpaqueOr::Explicit(key) => key,
        OpaqueOr::Opaque(_) => panic!("expected an explicit EC key"),
    };
    let private = nist_priv_bytes(&key).expect("private scalar");
    (key, private)
}

fn sign(key: &ec::Key, digest: Digest, message: &[u8]) -> Vec<u8> {
    let imp = OmmegaEc::default();
    let mut op = imp
        .begin_sign(key.clone().into(), digest)
        .expect("begin EC sign");
    op.update(message).expect("update");
    op.finish()
        .expect("EC signing must use the requested digest")
}

/// The signing operation used to hash with the curve default digest (P-256 ->
/// SHA-256, P-384 -> SHA-384) and ignored the requested one, so a key whose
/// digest differs from the default produced a valid-looking signature over the
/// wrong digest. The peer rejected it, and nothing on the device logged an
/// error. Signatures must therefore verify against the requested digest.
#[test]
fn ecdsa_sign_uses_the_requested_digest() {
    for curve in [ec::NistCurve::P256, ec::NistCurve::P384] {
        let (key, private) = test_key(curve);
        for digest in [Digest::Sha256, Digest::Sha384, Digest::Sha512] {
            let signature = sign(&key, digest, MESSAGE);
            let hashed = hash_digest(digest, MESSAGE).expect("digest");
            let verified = match curve {
                ec::NistCurve::P256 => {
                    let signing = SkP256::from_slice(&private).expect("P-256 key");
                    let verifying = signing.verifying_key();
                    let signature = SigP256::from_der(&signature).expect("DER signature");
                    verifying.verify_prehash(&hashed, &signature).is_ok()
                }
                _ => {
                    let signing = SkP384::from_slice(&private).expect("P-384 key");
                    let verifying = signing.verifying_key();
                    let signature = SigP384::from_der(&signature).expect("DER signature");
                    verifying.verify_prehash(&hashed, &signature).is_ok()
                }
            };
            assert!(
                verified,
                "{curve:?} with {digest:?} must sign the requested digest"
            );
        }
    }
}

/// Digest::None means the caller supplies the hash, so the input must be signed
/// as the prehash rather than hashed again.
#[test]
fn ecdsa_sign_with_no_digest_signs_the_input_as_a_prehash() {
    let (key, private) = test_key(ec::NistCurve::P256);
    let prehash = hash_digest(Digest::Sha256, MESSAGE).expect("digest");
    let signature = sign(&key, Digest::None, &prehash);
    let signing = SkP256::from_slice(&private).expect("P-256 key");
    let verifying = signing.verifying_key();
    let signature = SigP256::from_der(&signature).expect("DER signature");
    assert!(
        verifying.verify_prehash(&prehash, &signature).is_ok(),
        "Digest::None must sign the input as a prehash"
    );
}
