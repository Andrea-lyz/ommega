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
use kmr_common::crypto::{AccumulatingOperation as _, Rsa as _};
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

fn sign(imp: &OmmegaRsa, der: &[u8], mode: SignMode, message: &[u8]) -> Result<Vec<u8>, Error> {
    let mut op = imp.begin_sign(RsaKey(der.to_vec()).into(), mode)?;
    op.update(message)?;
    op.finish()
}

fn recover_signature(public: &RsaPublicKey, signature: &[u8]) -> Vec<u8> {
    assert_eq!(signature.len(), public.size());
    let recovered = rsa::BigUint::from_bytes_be(signature)
        .modpow(public.e(), public.n())
        .to_bytes_be();
    let mut encoded = vec![0; public.size()];
    let offset = encoded.len() - recovered.len();
    encoded[offset..].copy_from_slice(&recovered);
    encoded
}

#[test]
fn pkcs1_none_signs_type1_encoded_raw_messages() {
    let imp = OmmegaRsa::default();
    let (der, public) = test_key();
    let key_len = public.size();
    let max_size = key_len - PKCS1_UNDIGESTED_SIGNATURE_PADDING_OVERHEAD;
    for message in [Vec::new(), vec![0], MESSAGE.to_vec(), vec![0xa5; max_size]] {
        let signature = sign(
            &imp,
            &der,
            SignMode::Pkcs1_1_5Padding(Digest::None),
            &message,
        )
        .expect("NONEwithRSA signature");
        public
            .verify(Pkcs1v15Sign::new_unprefixed(), &message, &signature)
            .expect("signature must verify over the raw message");

        let mut expected = vec![0xff; key_len];
        expected[0] = 0;
        expected[1] = 1;
        expected[key_len - message.len() - 1] = 0;
        expected[key_len - message.len()..].copy_from_slice(&message);
        assert_eq!(recover_signature(&public, &signature), expected);
        assert!(public
            .verify(
                Pkcs1v15Sign::new_unprefixed(),
                b"not the signed message",
                &signature,
            )
            .is_err());
    }
}

#[test]
fn pkcs1_none_chunked_updates_match_one_shot() {
    let imp = OmmegaRsa::default();
    let (der, public) = test_key();
    let max_size = public.size() - PKCS1_UNDIGESTED_SIGNATURE_PADDING_OVERHEAD;
    let mut message = vec![0xa5; max_size];
    message[0] = 0;
    message[max_size / 2] = 0;
    let mode = SignMode::Pkcs1_1_5Padding(Digest::None);
    let expected = sign(&imp, &der, mode, &message).expect("one-shot signature");
    let mut op = imp
        .begin_sign(RsaKey(der).into(), mode)
        .expect("begin chunked signing");
    assert_eq!(op.max_input_size(), Some(max_size));
    op.update(&[]).expect("empty update");
    for chunk in message.chunks(17) {
        op.update(chunk).expect("chunk update");
    }
    op.update(&[]).expect("empty final update");
    let signature = op.finish().expect("chunked signature");
    assert_eq!(signature, expected);
    public
        .verify(Pkcs1v15Sign::new_unprefixed(), &message, &signature)
        .expect("chunked signature must verify over the raw message");
}

#[test]
fn undigested_sign_rejects_excess_input() {
    let imp = OmmegaRsa::default();
    let (der, public) = test_key();
    for (mode, max_size) in [
        (SignMode::NoPadding, public.size()),
        (
            SignMode::Pkcs1_1_5Padding(Digest::None),
            public.size() - PKCS1_UNDIGESTED_SIGNATURE_PADDING_OVERHEAD,
        ),
    ] {
        let error = sign(&imp, &der, mode, &vec![0; max_size + 1])
            .expect_err("oversized input must not be signed");
        assert!(matches!(
            error.kind(),
            kmr_common::ErrorKind::Hal(keymint::ErrorCode::InvalidInputLength, _)
        ));

        let mut op = imp
            .begin_sign(RsaKey(der.clone()).into(), mode)
            .expect("begin signing");
        assert_eq!(op.max_input_size(), Some(max_size));
        op.update(&vec![0; max_size]).expect("maximum input");
        let error = op.update(&[1]).expect_err("excess chunk must be rejected");
        assert!(matches!(
            error.kind(),
            kmr_common::ErrorKind::Hal(keymint::ErrorCode::InvalidInputLength, _)
        ));

        // Also exercise finish independently of the update/TA size checks.
        let mut op = OmmegaRsaUndigestSignOperation::new(RsaKey(der.clone()), mode)
            .expect("undigested operation");
        op.pending_input = vec![0; max_size + 1];
        let error = Box::new(op)
            .finish()
            .expect_err("finish must reject excess input without panicking");
        assert!(matches!(
            error.kind(),
            kmr_common::ErrorKind::Hal(keymint::ErrorCode::InvalidInputLength, _)
        ));
    }
}

#[test]
fn no_padding_signs_unmodified_left_padded_messages() {
    let imp = OmmegaRsa::default();
    let (der, public) = test_key();
    let key_len = public.size();
    let below_modulus = (public.n() - rsa::BigUint::from(1u8)).to_bytes_be();
    for message in [
        Vec::new(),
        vec![0],
        MESSAGE.to_vec(),
        vec![0x42; key_len],
        below_modulus,
    ] {
        let signature = sign(&imp, &der, SignMode::NoPadding, &message).expect("raw RSA signature");
        let mut expected = vec![0; key_len];
        expected[key_len - message.len()..].copy_from_slice(&message);
        assert_eq!(recover_signature(&public, &signature), expected);
        assert!(public
            .verify(Pkcs1v15Sign::new_unprefixed(), &message, &signature)
            .is_err());
    }
}

#[test]
fn no_padding_rejects_representatives_at_or_above_modulus() {
    let imp = OmmegaRsa::default();
    let (der, public) = test_key();
    for message in [public.n().to_bytes_be(), vec![0xff; public.size()]] {
        let error = sign(&imp, &der, SignMode::NoPadding, &message)
            .expect_err("out-of-range representative must not be reduced modulo n");
        assert!(matches!(
            error.kind(),
            kmr_common::ErrorKind::Hal(keymint::ErrorCode::InvalidArgument, _)
        ));
    }
}

#[test]
fn zero_pad_left_rejects_excess_input() {
    let error = zero_pad_left(&[0, 1], 1).expect_err("oversized input must not panic");
    assert!(matches!(
        error.kind(),
        kmr_common::ErrorKind::Hal(keymint::ErrorCode::InvalidInputLength, _)
    ));
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
        let signature = sign(&imp, &der, SignMode::Pkcs1_1_5Padding(digest), MESSAGE)
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
    for digest in [
        Digest::Sha1,
        Digest::Sha224,
        Digest::Sha256,
        Digest::Sha384,
        Digest::Sha512,
    ] {
        let signature = sign(&imp, &der, SignMode::PssPadding(digest), MESSAGE)
            .unwrap_or_else(|e| panic!("RSA-PSS signing with {digest:?} failed: {e:?}"));
        let hashed = hash_digest(digest, MESSAGE).expect("digest");
        public
            .verify(pss_scheme(digest).expect("scheme"), &hashed, &signature)
            .unwrap_or_else(|e| panic!("RSA-PSS {digest:?} signature must verify: {e:?}"));
    }
}
