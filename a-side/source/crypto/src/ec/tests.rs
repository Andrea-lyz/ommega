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
use p521::ecdsa::SigningKey as SkP521;
use p521::ecdsa::VerifyingKey as VkP521;

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

fn p521_verifying_key(private: &[u8]) -> VkP521 {
    VkP521::from(&SkP521::from_slice(private).expect("P-521 key"))
}

/// P-521 signing used to fail with `UnsupportedEcCurve` for every request, which
/// AndroidKeyStore surfaced as SignatureException: an application that created a
/// secp521r1 key simply could not sign. AOSP signs these through BoringSSL, and
/// AndroidKeyStore also allows a P-521 key restricted to SHA-256, where the digest
/// is shorter than the field and has to be left-padded into the 66-byte `z`.
#[test]
fn ecdsa_p521_signs_every_requested_digest() {
    let (key, private) = test_key(ec::NistCurve::P521);
    let verifying = p521_verifying_key(&private);
    for digest in [Digest::Sha256, Digest::Sha384, Digest::Sha512] {
        let signature = sign(&key, digest, MESSAGE);
        let hashed = hash_digest(digest, MESSAGE).expect("digest");
        let z = prehash_to_field_bytes(ec::NistCurve::P521, &hashed).expect("z value");
        let signature = SigP521::from_der(&signature).expect("DER signature");
        assert!(
            verifying.verify_prehash(&z, &signature).is_ok(),
            "P-521 with {digest:?} must sign the requested digest"
        );
    }
}

/// `Digest::None` signs the input as the prehash. For P-521 that is the
/// leftmost 521 bits of the supplied 66 bytes, not the 528-bit integer they
/// spell: the peer reduces them the same way before verifying.
#[test]
fn ecdsa_p521_sign_with_no_digest_signs_the_input_as_a_prehash() {
    let (key, private) = test_key(ec::NistCurve::P521);
    let verifying = p521_verifying_key(&private);
    // P-521 coordinates are 66 bytes; `coord_len` is not const, so name the size.
    let prehash = [0x5au8; 66];
    let signature = SigP521::from_der(&sign(&key, Digest::None, &prehash)).expect("DER signature");
    let z = prehash_to_field_bytes(ec::NistCurve::P521, &prehash).expect("z value");
    assert!(
        verifying.verify_prehash(&z, &signature).is_ok(),
        "Digest::None must sign the input as a prehash"
    );
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

/// Test-local big-endian right shift. Written out independently of the
/// production helper so the expected value is not produced by the code under
/// test.
fn shifted_right(bytes: &[u8], bits: usize) -> Vec<u8> {
    assert!(bits > 0 && bits < 8, "only sub-byte shifts are needed here");
    let mut out = bytes.to_vec();
    for index in 0..out.len() {
        let carried = if index == 0 {
            0
        } else {
            bytes[index - 1] & ((1u8 << bits) - 1)
        };
        out[index] = (bytes[index] >> bits) | (carried << (8 - bits));
    }
    out
}

/// AOSP accepts a `Digest::None` prehash shorter than the key: KeyMint's own
/// signing test uses a 10-byte message on P-256. RustCrypto's `bits2field`
/// rejects anything below half the field width (16 bytes on P-256), so the input
/// has to be widened before signing instead of being refused.
#[test]
fn ecdsa_p256_signs_short_raw_digests() {
    let (key, private) = test_key(ec::NistCurve::P256);
    let signing = SkP256::from_slice(&private).expect("P-256 key");
    let verifying = signing.verifying_key();
    let input = [0x5au8; 33];
    for len in [10usize, 15, 16, 31, 32, 33] {
        let prehash = &input[..len];
        let signature =
            SigP256::from_der(&sign(&key, Digest::None, prehash)).expect("DER signature");
        // bits2int reads the input as a big-endian integer: short inputs are
        // left-padded, and a full 32 bytes is copied unchanged.
        let mut z = [0u8; 32];
        let copy = core::cmp::min(32, len);
        z[32 - copy..].copy_from_slice(&prehash[..copy]);
        assert!(
            verifying.verify_prehash(&z, &signature).is_ok(),
            "P-256 must sign a {len}-byte prehash"
        );
    }
}

/// The engineering-mode self test also signs with P-224, whose half-field cutoff
/// is 14 bytes, so a 10-byte prehash has to work there too.
#[test]
fn ecdsa_p224_signs_a_short_raw_digest() {
    use p224::ecdsa::SigningKey as SkP224;
    let (key, private) = test_key(ec::NistCurve::P224);
    let signing = SkP224::from_slice(&private).expect("P-224 key");
    let input = [0x5au8; 10];
    let signature = SigP224::from_der(&sign(&key, Digest::None, &input)).expect("DER signature");
    let mut z = [0u8; 28];
    z[18..].copy_from_slice(&input);
    assert!(
        signing
            .verifying_key()
            .verify_prehash(&z, &signature)
            .is_ok(),
        "P-224 must sign a 10-byte prehash"
    );
}

/// A P-384 key may be restricted to SHA-1, and a caller may pass a 20-byte raw
/// prehash. Both are shorter than half of the 48-byte field, which is exactly
/// what RustCrypto's `bits2field` refuses.
#[test]
fn ecdsa_p384_signs_sha1_and_short_raw_digests() {
    let (key, private) = test_key(ec::NistCurve::P384);
    let signing = SkP384::from_slice(&private).expect("P-384 key");
    let verifying = signing.verifying_key();

    let sha1 = hash_digest(Digest::Sha1, MESSAGE).expect("sha1");
    let signature = SigP384::from_der(&sign(&key, Digest::Sha1, MESSAGE)).expect("DER signature");
    let mut z = [0u8; 48];
    z[48 - sha1.len()..].copy_from_slice(&sha1);
    assert!(
        verifying.verify_prehash(&z, &signature).is_ok(),
        "P-384 must sign a SHA-1 digest"
    );

    let input = [0x3cu8; 20];
    let signature = SigP384::from_der(&sign(&key, Digest::None, &input)).expect("DER signature");
    let mut z = [0u8; 48];
    z[48 - input.len()..].copy_from_slice(&input);
    assert!(
        verifying.verify_prehash(&z, &signature).is_ok(),
        "P-384 must sign a 20-byte prehash"
    );
}

/// The P-521 order is 521 of the 528 bits carried by 66 bytes, so a full-width
/// digest keeps only its leftmost 521 bits. Signing the whole 66 bytes instead
/// yields a signature that is valid only under that wrong reading, which is what
/// a detuning device produced before this was fixed.
#[test]
fn ecdsa_p521_keeps_the_leftmost_521_bits_of_a_full_width_prehash() {
    let (key, private) = test_key(ec::NistCurve::P521);
    let verifying = p521_verifying_key(&private);
    let mut prehash = [0x5au8; 66];
    prehash[0] |= 0x80; // force a carry out of the top byte when shifting
    let signature = SigP521::from_der(&sign(&key, Digest::None, &prehash)).expect("DER signature");

    let z = shifted_right(&prehash, 7);
    assert!(
        verifying.verify_prehash(&z, &signature).is_ok(),
        "P-521 must use the leftmost 521 bits of a full-width prehash"
    );
    assert!(
        verifying.verify_prehash(&prehash, &signature).is_err(),
        "a P-521 signature must not be valid under the unshifted 528-bit reading"
    );
}

/// 65 bytes is 520 bits, still below the 521-bit order: the value is unchanged
/// and only left-padded, so shifting it unconditionally would break it.
#[test]
fn ecdsa_p521_left_pads_a_short_raw_digest_without_shifting() {
    let (key, private) = test_key(ec::NistCurve::P521);
    let verifying = p521_verifying_key(&private);
    let prehash = [0x5au8; 65];
    let signature = SigP521::from_der(&sign(&key, Digest::None, &prehash)).expect("DER signature");

    let mut z = [0u8; 66];
    z[1..].copy_from_slice(&prehash);
    assert!(
        verifying.verify_prehash(&z, &signature).is_ok(),
        "a 520-bit prehash is left-padded, never shifted"
    );
    assert!(
        verifying
            .verify_prehash(&shifted_right(&z, 7), &signature)
            .is_err(),
        "a 520-bit prehash must not be reduced by seven bits"
    );
}

/// AOSP signs through BoringSSL, which draws a fresh nonce for every call, so a
/// stock device never repeats a signature for the same key and message. Plain
/// RFC6979 is deterministic, and that repeatability is an observable fingerprint
/// of a software implementation. Every curve must therefore produce a different
/// signature on a second call, and both signatures must still verify.
#[test]
fn ecdsa_uses_a_fresh_nonce_for_every_signature() {
    for curve in [
        ec::NistCurve::P224,
        ec::NistCurve::P256,
        ec::NistCurve::P384,
        ec::NistCurve::P521,
    ] {
        let (key, private) = test_key(curve);
        let digest = match curve {
            ec::NistCurve::P224 | ec::NistCurve::P256 => Digest::Sha256,
            ec::NistCurve::P384 => Digest::Sha384,
            ec::NistCurve::P521 => Digest::Sha512,
        };
        let first = sign(&key, digest, MESSAGE);
        let second = sign(&key, digest, MESSAGE);
        assert_ne!(
            first, second,
            "{curve:?} must draw a fresh nonce for every signature"
        );

        let hashed = hash_digest(digest, MESSAGE).expect("digest");
        let z = prehash_to_field_bytes(curve, &hashed).expect("z value");
        let verified = match curve {
            ec::NistCurve::P224 => {
                use p224::ecdsa::SigningKey as SkP224;
                let signing = SkP224::from_slice(&private).expect("P-224 key");
                let verifying = signing.verifying_key();
                [&first, &second].iter().all(|der| {
                    let signature = SigP224::from_der(der).expect("DER signature");
                    verifying.verify_prehash(&z, &signature).is_ok()
                })
            }
            ec::NistCurve::P256 => {
                let signing = SkP256::from_slice(&private).expect("P-256 key");
                let verifying = signing.verifying_key();
                [&first, &second].iter().all(|der| {
                    let signature = SigP256::from_der(der).expect("DER signature");
                    verifying.verify_prehash(&z, &signature).is_ok()
                })
            }
            ec::NistCurve::P384 => {
                let signing = SkP384::from_slice(&private).expect("P-384 key");
                let verifying = signing.verifying_key();
                [&first, &second].iter().all(|der| {
                    let signature = SigP384::from_der(der).expect("DER signature");
                    verifying.verify_prehash(&z, &signature).is_ok()
                })
            }
            ec::NistCurve::P521 => {
                let verifying = p521_verifying_key(&private);
                [&first, &second].iter().all(|der| {
                    let signature = SigP521::from_der(der).expect("DER signature");
                    verifying.verify_prehash(&z, &signature).is_ok()
                })
            }
        };
        assert!(
            verified,
            "{curve:?} must keep every randomized signature verifiable"
        );
    }
}
