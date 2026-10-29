//! Known-answer and authentication regressions for the software AES backend.
use super::*;
use kmr_common::crypto::{Aes as _, SymmetricOperation};
use kmr_common::ErrorKind;
use kmr_wire::keymint::ErrorCode;

fn hex(value: &str) -> Vec<u8> {
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty());
    pairs
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn assert_code<T>(result: Result<T, Error>, expected: ErrorCode) {
    match result {
        Err(error) => assert!(
            matches!(error.kind(), ErrorKind::Hal(code, _) if *code == expected),
            "{error:?}"
        ),
        Ok(_) => panic!("expected {expected:?}"),
    }
}

fn gcm_mode(tag_len: usize, nonce: [u8; 12]) -> crypto::aes::GcmMode {
    match tag_len {
        12 => crypto::aes::GcmMode::GcmTag12 { nonce },
        13 => crypto::aes::GcmMode::GcmTag13 { nonce },
        14 => crypto::aes::GcmMode::GcmTag14 { nonce },
        15 => crypto::aes::GcmMode::GcmTag15 { nonce },
        16 => crypto::aes::GcmMode::GcmTag16 { nonce },
        _ => panic!("invalid test tag length"),
    }
}

fn aead_operation(
    key: &[u8],
    nonce: [u8; 12],
    tag_len: usize,
    aad: &[u8],
    data: &[u8],
    dir: SymmetricOperation,
    chunk: usize,
) -> Result<Vec<u8>, Error> {
    let key = crypto::aes::Key::new_from(key).unwrap();
    let mut op = OmmegaAes.begin_aead(key.into(), gcm_mode(tag_len, nonce), dir)?;
    for part in aad.chunks(chunk) {
        op.update_aad(part)?;
    }
    let mut output = Vec::new();
    for part in data.chunks(chunk) {
        let emitted = op.update(part)?;
        if dir == SymmetricOperation::Decrypt {
            assert!(
                emitted.is_empty(),
                "plaintext must not escape before authentication"
            );
        }
        output.extend_from_slice(&emitted);
    }
    output.extend_from_slice(&op.finish()?);
    Ok(output)
}

#[test]
fn gcm_nist_zero_key_vectors_all_sizes_and_tags() {
    // NIST SP 800-38D zero-key examples; AES-192 was also cross-checked with OpenSSL.
    let vectors = [
        (
            16,
            "58e2fccefa7e3061367f1d57a4e7455a",
            "0388dace60b6a392f328c2b971b2fe78",
            "ab6e47d42cec13bdf53a67b21257bddf",
        ),
        (
            24,
            "cd33b28ac773f74ba00ed1f312572435",
            "98e7247c07f0fe411c267e4384b0f600",
            "2ff58d80033927ab8ef4d4587514f0fb",
        ),
        (
            32,
            "530f8afbc74536b9a963b4f1c4cb738b",
            "cea7403d4d606b6e074ec5d3baf39d18",
            "d0d1c8a799996bf0265b98b5d48ab919",
        ),
    ];
    for (size, empty_tag, ciphertext, full_tag) in vectors {
        let key = vec![0; size];
        for len in 12..=16 {
            for (plain, cipher, tag) in [
                (Vec::new(), Vec::new(), hex(empty_tag)),
                (vec![0; 16], hex(ciphertext), hex(full_tag)),
            ] {
                let mut expected = cipher;
                expected.extend_from_slice(&tag[..len]);
                assert_eq!(
                    gcm_encrypt(&key, &[0; 12], &[], &plain, len).unwrap(),
                    expected
                );
                for chunk in [1, 7, 16, 31] {
                    assert_eq!(
                        aead_operation(
                            &key,
                            [0; 12],
                            len,
                            &[],
                            &plain,
                            SymmetricOperation::Encrypt,
                            chunk
                        )
                        .unwrap(),
                        expected
                    );
                    assert_eq!(
                        aead_operation(
                            &key,
                            [0; 12],
                            len,
                            &[],
                            &expected,
                            SymmetricOperation::Decrypt,
                            chunk
                        )
                        .unwrap(),
                        plain
                    );
                }
            }
        }
    }
}

#[test]
fn gcm_openssl_aad_vectors_all_key_and_tag_sizes() {
    // Independent Node/OpenSSL AEAD output: key bytes 0..size, plaintext 0..37.
    let vectors = [
        (
            16,
            "06046a2537f059c23055074ea1d5633ad1daf54efab4fa7b45bc214cf8947243e546b36449",
            "acd587167cfa0f5ebfe9bac74d0d3cc5",
        ),
        (
            24,
            "7c791ee6d7f00d0b020d7d366e4ea2c609f02692a456354e1ddd2092b2c2579ba75930a97f",
            "5ec03421711720b6747ee1a6571c9576",
        ),
        (
            32,
            "05eb58d6e891f68144ab694c1c1ee4275252f3ed837846d2b946e33f82ab68daa4c5b9fc4a",
            "1c27e9a5a777ac6c2c23659426c1413c",
        ),
    ];
    let nonce: [u8; 12] = hex("0102030405060708090a0b0c").try_into().unwrap();
    let aad = b"keymint-gcm-authenticated-aad";
    let plain: Vec<u8> = (0..37).collect();
    for (size, ciphertext, tag) in vectors {
        let key: Vec<u8> = (0..size).collect();
        for len in 12..=16 {
            let mut expected = hex(ciphertext);
            expected.extend_from_slice(&hex(tag)[..len]);
            for chunk in [1, 8, 17, 64] {
                assert_eq!(
                    aead_operation(
                        &key,
                        nonce,
                        len,
                        aad,
                        &plain,
                        SymmetricOperation::Encrypt,
                        chunk
                    )
                    .unwrap(),
                    expected
                );
                assert_eq!(
                    aead_operation(
                        &key,
                        nonce,
                        len,
                        aad,
                        &expected,
                        SymmetricOperation::Decrypt,
                        chunk
                    )
                    .unwrap(),
                    plain
                );
            }
        }
    }
}

#[test]
fn gcm_rejects_corruption_before_returning_plaintext() {
    let aad = b"authenticated metadata";
    let plain = b"plaintext must not be returned for an invalid tag";
    for size in [16, 24, 32] {
        let key = vec![0x42; size];
        for len in 12..=16 {
            let encoded = gcm_encrypt(&key, &[0x11; 12], aad, plain, len).unwrap();
            for index in [0, plain.len(), encoded.len() - 1] {
                let mut corrupted = encoded.clone();
                corrupted[index] ^= 1;
                assert_code(
                    aead_operation(
                        &key,
                        [0x11; 12],
                        len,
                        aad,
                        &corrupted,
                        SymmetricOperation::Decrypt,
                        7,
                    ),
                    ErrorCode::VerificationFailed,
                );
            }
            assert_code(
                aead_operation(
                    &key,
                    [0x12; 12],
                    len,
                    aad,
                    &encoded,
                    SymmetricOperation::Decrypt,
                    7,
                ),
                ErrorCode::VerificationFailed,
            );
            assert_code(
                aead_operation(
                    &key,
                    [0x11; 12],
                    len,
                    b"wrong metadata",
                    &encoded,
                    SymmetricOperation::Decrypt,
                    7,
                ),
                ErrorCode::VerificationFailed,
            );
            assert_code(
                aead_operation(
                    &key,
                    [0x11; 12],
                    len,
                    aad,
                    &encoded[..len - 1],
                    SymmetricOperation::Decrypt,
                    1,
                ),
                ErrorCode::InvalidTag,
            );
        }
    }
}

#[test]
fn gcm_storage_helpers_keep_128_bit_tag_format() {
    for size in [16, 32] {
        let key = vec![0x72; size];
        let nonce = [0x13; 12];
        let aad = b"storage header";
        let plain = b"existing storage must remain readable";
        let algorithm = if size == 16 {
            &ring::aead::AES_128_GCM
        } else {
            &ring::aead::AES_256_GCM
        };
        let ring_key =
            ring::aead::LessSafeKey::new(ring::aead::UnboundKey::new(algorithm, &key).unwrap());
        let mut reference = plain.to_vec();
        let tag = ring_key
            .seal_in_place_separate_tag(
                ring::aead::Nonce::assume_unique_for_key(nonce),
                ring::aead::Aad::from(aad),
                &mut reference,
            )
            .unwrap();
        reference.extend_from_slice(tag.as_ref());
        let output = gcm_encrypt_raw(&key, &nonce, aad, plain).unwrap();
        assert_eq!(output, reference);
        assert_eq!(
            gcm_decrypt_raw(
                &key,
                &nonce,
                aad,
                &output[..plain.len()],
                &output[plain.len()..]
            )
            .unwrap(),
            plain
        );
        assert_code(
            gcm_decrypt_raw(
                &key,
                &nonce,
                aad,
                &output[..plain.len()],
                &output[plain.len()..plain.len() + 12],
            ),
            ErrorCode::InvalidMacLength,
        );
    }
    for len in [0, 4, 11, 17] {
        assert_code(
            gcm_encrypt(&[0; 16], &[0; 12], &[], &[], len),
            ErrorCode::InvalidMacLength,
        );
        assert_code(
            gcm_decrypt(&[0; 16], &[0; 12], &[], &[], &vec![0; len]),
            ErrorCode::InvalidMacLength,
        );
    }
}

fn ctr_operation(
    key: &[u8],
    iv: [u8; 16],
    input: &[u8],
    dir: SymmetricOperation,
    chunk: usize,
) -> Vec<u8> {
    let mut op = OmmegaAes
        .begin(
            crypto::aes::Key::new_from(key).unwrap().into(),
            crypto::aes::CipherMode::Ctr { nonce: iv },
            dir,
        )
        .unwrap();
    let mut output = Vec::new();
    for part in input.chunks(chunk) {
        output.extend_from_slice(&op.update(part).unwrap());
    }
    output.extend_from_slice(&op.finish().unwrap());
    output
}

#[test]
fn ctr_nist_sp800_38a_full_iv_all_key_sizes() {
    let iv: [u8; 16] = hex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff").try_into().unwrap();
    let plain = hex(concat!(
        "6bc1bee22e409f96e93d7e117393172a",
        "ae2d8a571e03ac9c9eb76fac45af8e51",
        "30c81c46a35ce411e5fbc1191a0a52ef",
        "f69f2445df4f9b17ad2b417be66c3710"
    ));
    let vectors = [
        (
            "2b7e151628aed2a6abf7158809cf4f3c",
            concat!(
                "874d6191b620e3261bef6864990db6ce",
                "9806f66b7970fdff8617187bb9fffdff",
                "5ae4df3edbd5d35e5b4f09020db03eab",
                "1e031dda2fbe03d1792170a0f3009cee"
            ),
        ),
        (
            "8e73b0f7da0e6452c810f32b809079e562f8ead2522c6b7b",
            concat!(
                "1abc932417521ca24f2b0459fe7e6e0b",
                "090339ec0aa6faefd5ccc2c6f4ce8e94",
                "1e36b26bd1ebc670d1bd1d665620abf7",
                "4f78a7f6d29809585a97daec58c6b050"
            ),
        ),
        (
            "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4",
            concat!(
                "601ec313775789a5b7a7f504bbf3d228",
                "f443e3ca4d62b59aca84e990cacaf5c5",
                "2b0930daa23de94ce87017ba2d84988d",
                "dfc9c58db67aada613c2dd08457941a6"
            ),
        ),
    ];
    for (key, expected) in vectors {
        let key = hex(key);
        let expected = hex(expected);
        for len in [0, 1, 15, 16, 17, 64] {
            for chunk in [1, 7, 16, 31, 128] {
                assert_eq!(
                    ctr_operation(&key, iv, &plain[..len], SymmetricOperation::Encrypt, chunk),
                    expected[..len]
                );
                assert_eq!(
                    ctr_operation(
                        &key,
                        iv,
                        &expected[..len],
                        SymmetricOperation::Decrypt,
                        chunk
                    ),
                    plain[..len]
                );
            }
        }
    }
}

#[test]
fn ctr_carries_from_low_counter_into_full_iv() {
    let iv = hex("000000000000000000000000ffffffff").try_into().unwrap();
    let plain: Vec<u8> = (0..33).collect();
    // Independently generated using OpenSSL aes-256-ctr.
    let expected = hex("ebc95963cc0fcf01991b02a78d9cc496776c5b5eaf66dce244008a0e91bc310ba4");
    assert_eq!(
        ctr_operation(&[0; 32], iv, &plain, SymmetricOperation::Encrypt, 1),
        expected
    );
}

#[test]
fn pkcs7_rejects_partial_ciphertext_and_oversized_padding() {
    let key = crypto::aes::Key::Aes128([0x23; 16]);
    for cbc in [false, true] {
        let padded_mode = if cbc {
            crypto::aes::CipherMode::CbcPkcs7Padding { nonce: [0; 16] }
        } else {
            crypto::aes::CipherMode::EcbPkcs7Padding
        };
        let raw_mode = if cbc {
            crypto::aes::CipherMode::CbcNoPadding { nonce: [0; 16] }
        } else {
            crypto::aes::CipherMode::EcbNoPadding
        };
        let mut enc = OmmegaAes
            .begin(key.clone().into(), padded_mode, SymmetricOperation::Encrypt)
            .unwrap();
        enc.update(b"valid padded message").unwrap();
        let valid = enc.finish().unwrap();
        for extra in 1..16 {
            let mut malformed = valid.clone();
            malformed.extend_from_slice(&vec![0; extra]);
            let mut dec = OmmegaAes
                .begin(key.clone().into(), padded_mode, SymmetricOperation::Decrypt)
                .unwrap();
            dec.update(&malformed).unwrap();
            assert_code(dec.finish(), ErrorCode::InvalidInputLength);
        }
        let dec = OmmegaAes
            .begin(key.clone().into(), padded_mode, SymmetricOperation::Decrypt)
            .unwrap();
        assert_code(dec.finish(), ErrorCode::InvalidInputLength);
        // Two full blocks of 0x20 must not count as 32 bytes of PKCS#7 padding.
        for invalid in [vec![32; 32], vec![0; 16], vec![17; 32]] {
            let mut enc = OmmegaAes
                .begin(key.clone().into(), raw_mode, SymmetricOperation::Encrypt)
                .unwrap();
            enc.update(&invalid).unwrap();
            let ciphertext = enc.finish().unwrap();
            let mut dec = OmmegaAes
                .begin(key.clone().into(), padded_mode, SymmetricOperation::Decrypt)
                .unwrap();
            dec.update(&ciphertext).unwrap();
            assert_code(dec.finish(), ErrorCode::InvalidArgument);
        }
    }
}
