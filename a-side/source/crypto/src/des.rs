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

//! Pure-Rust implementation of 3-DES (replaces BoringSSL; RustCrypto `des`).
use cipher::{generic_array::GenericArray, BlockDecrypt, BlockEncrypt, KeyInit};
use des::TdesEde3;
use kmr_common::{crypto, crypto::OpaqueOr, explicit, km_err, Error};
use std::boxed::Box;
use std::vec::Vec;

/// [`crypto::Des`] implementation backed by RustCrypto `des` (3-DES/EDE).
pub struct OmmegaDes;

impl crypto::Des for OmmegaDes {
    fn begin(
        &self,
        key: OpaqueOr<crypto::des::Key>,
        mode: crypto::des::Mode,
        dir: crypto::SymmetricOperation,
    ) -> Result<Box<dyn crypto::EmittingOperation>, Error> {
        let key = explicit!(key)?;
        let cipher = TdesEde3::new_from_slice(&key.0)
            .map_err(|_| km_err!(UnknownError, "invalid 3-DES key"))?;
        let cbc_prev = match mode {
            crypto::des::Mode::CbcNoPadding { nonce }
            | crypto::des::Mode::CbcPkcs7Padding { nonce } => nonce,
            _ => [0; BLOCK],
        };
        Ok(Box::new(OmmegaDesOperation {
            cipher,
            mode,
            dir,
            cbc_prev,
            pending: Vec::new(),
        }))
    }
}

/// DES operation backed by RustCrypto `des`.
pub struct OmmegaDesOperation {
    cipher: TdesEde3,
    mode: crypto::des::Mode,
    dir: crypto::SymmetricOperation,
    /// CBC chaining value, initially the nonce and then the last ciphertext block.
    cbc_prev: [u8; BLOCK],
    /// Buffered input, including the final full block during PKCS#7 decryption.
    pending: Vec<u8>,
}

const BLOCK: usize = crypto::des::BLOCK_SIZE;

fn pkcs7_pad(data: &[u8], block: usize) -> Vec<u8> {
    let pad = block - data.len() % block;
    let mut out = data.to_vec();
    out.extend(std::iter::repeat_n(pad as u8, pad));
    out
}

fn pkcs7_unpad(data: &[u8]) -> Result<Vec<u8>, Error> {
    let &pad = data
        .last()
        .ok_or_else(|| km_err!(InvalidInputLength, "empty block"))?;
    if !data.len().is_multiple_of(BLOCK) {
        return Err(km_err!(InvalidInputLength, "input is not a full block"));
    }
    let pad = pad as usize;
    if !(1..=BLOCK).contains(&pad) {
        return Err(km_err!(InvalidArgument, "bad PKCS#7 padding"));
    }
    let start = data.len() - pad;
    if data[start..].iter().any(|&b| b as usize != pad) {
        return Err(km_err!(InvalidArgument, "bad PKCS#7 padding bytes"));
    }
    Ok(data[..start].to_vec())
}

impl OmmegaDesOperation {
    fn process_block(&mut self, block: &mut [u8; BLOCK]) {
        let key = GenericArray::from_mut_slice(block);
        match self.dir {
            crypto::SymmetricOperation::Encrypt => {
                self.cipher.encrypt_block(key);
            }
            crypto::SymmetricOperation::Decrypt => {
                self.cipher.decrypt_block(key);
            }
        }
    }

    fn run_cipher(&mut self, input: &[u8], output: &mut Vec<u8>) {
        match self.mode {
            crypto::des::Mode::EcbNoPadding | crypto::des::Mode::EcbPkcs7Padding => {
                for chunk in input.as_chunks::<8>().0 {
                    let mut blk = [0u8; BLOCK];
                    blk.copy_from_slice(chunk);
                    self.process_block(&mut blk);
                    output.extend_from_slice(&blk);
                }
            }
            crypto::des::Mode::CbcNoPadding { .. } | crypto::des::Mode::CbcPkcs7Padding { .. } => {
                for chunk in input.as_chunks::<8>().0 {
                    let mut blk = [0u8; BLOCK];
                    blk.copy_from_slice(chunk);
                    match self.dir {
                        crypto::SymmetricOperation::Encrypt => {
                            for (byte, previous) in blk.iter_mut().zip(self.cbc_prev) {
                                *byte ^= previous;
                            }
                            self.process_block(&mut blk);
                            self.cbc_prev = blk;
                        }
                        crypto::SymmetricOperation::Decrypt => {
                            let orig = blk;
                            self.process_block(&mut blk);
                            for (byte, previous) in blk.iter_mut().zip(self.cbc_prev) {
                                *byte ^= previous;
                            }
                            self.cbc_prev = orig;
                        }
                    }
                    output.extend_from_slice(&blk);
                }
            }
        }
    }
}

impl crypto::EmittingOperation for OmmegaDesOperation {
    fn update(&mut self, data: &[u8]) -> Result<Vec<u8>, Error> {
        self.pending.extend_from_slice(data);
        // Retain a complete final block for PKCS#7 validation. A subsequent byte
        // proves that the preceding block cannot contain the final padding.
        let usable = if self.dir == crypto::SymmetricOperation::Decrypt
            && matches!(
                self.mode,
                crypto::des::Mode::EcbPkcs7Padding | crypto::des::Mode::CbcPkcs7Padding { .. }
            ) {
            self.pending.len().saturating_sub(1) / BLOCK * BLOCK
        } else {
            self.pending.len() / BLOCK * BLOCK
        };
        let mut out = Vec::new();
        let pending_slice = self.pending[..usable].to_vec();
        self.run_cipher(&pending_slice, &mut out);
        self.pending.drain(..usable);
        Ok(out)
    }

    fn finish(mut self: Box<Self>) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        match self.mode {
            crypto::des::Mode::EcbNoPadding | crypto::des::Mode::CbcNoPadding { .. } => {
                if !self.pending.is_empty() {
                    return Err(km_err!(
                        InvalidInputLength,
                        "input length {} not a multiple of block size",
                        self.pending.len()
                    ));
                }
            }
            crypto::des::Mode::EcbPkcs7Padding | crypto::des::Mode::CbcPkcs7Padding { .. } => {
                match self.dir {
                    crypto::SymmetricOperation::Encrypt => {
                        let padded = pkcs7_pad(&self.pending, BLOCK);
                        self.run_cipher(&padded, &mut out);
                    }
                    crypto::SymmetricOperation::Decrypt => {
                        if self.pending.len() != BLOCK {
                            return Err(km_err!(
                                InvalidInputLength,
                                "missing or incomplete final 3-DES block"
                            ));
                        }
                        let final_block = self.pending.clone();
                        self.run_cipher(&final_block, &mut out);
                        out = pkcs7_unpad(&out)?;
                    }
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kmr_common::crypto::{des::Mode, Des as _, SymmetricOperation};
    use kmr_common::ErrorKind;
    use kmr_wire::keymint::ErrorCode;
    use SymmetricOperation::{Decrypt, Encrypt};

    const KEY: [u8; 24] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
    ];
    const NONCE: [u8; BLOCK] = [0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef];
    const NO_PADDING: [Mode; 2] = [Mode::EcbNoPadding, Mode::CbcNoPadding { nonce: NONCE }];
    const PKCS7: [Mode; 2] = [
        Mode::EcbPkcs7Padding,
        Mode::CbcPkcs7Padding { nonce: NONCE },
    ];

    fn from_hex(hex: &str) -> Vec<u8> {
        assert!(hex.len().is_multiple_of(2));
        (0..hex.len())
            .step_by(2)
            .map(|offset| u8::from_str_radix(&hex[offset..offset + 2], 16).unwrap())
            .collect()
    }

    fn assert_hal_error(result: Result<Vec<u8>, Error>, expected: ErrorCode) {
        let error = result.expect_err("invalid input must not succeed");
        match error.kind() {
            ErrorKind::Hal(actual, _) => assert_eq!(*actual, expected),
            other => panic!("expected {expected:?}, got {other:?}"),
        }
    }

    fn check_chunks(
        mode: Mode,
        dir: SymmetricOperation,
        input: &[u8],
        expected: &Result<&[u8], ErrorCode>,
        ends: &[usize],
    ) {
        let mut op = OmmegaDes
            .begin(crypto::des::Key(KEY).into(), mode, dir)
            .unwrap();
        let retain_final =
            dir == Decrypt && matches!(mode, Mode::EcbPkcs7Padding | Mode::CbcPkcs7Padding { .. });
        let mut output = Vec::new();
        let mut start = 0;
        for &end in ends {
            output.extend(op.update(&input[start..end]).unwrap());
            // EVP emits all complete blocks except an exactly aligned final
            // block during padded decryption (8 bytes -> 0, 9 bytes -> 8).
            let emitted = if retain_final {
                end.saturating_sub(1)
            } else {
                end
            } / BLOCK
                * BLOCK;
            assert_eq!(output.len(), emitted, "{mode:?} {dir:?}, cuts {ends:?}");
            if let Ok(want) = expected {
                assert_eq!(
                    output.as_slice(),
                    &want[..emitted],
                    "{mode:?} {dir:?}, cuts {ends:?}"
                );
            }
            assert!(
                op.update(&[]).unwrap().is_empty(),
                "empty update must not flush a block"
            );
            start = end;
        }
        assert_eq!(start, input.len());
        let result = op.finish();
        match expected {
            Ok(want) => {
                output.extend(result.unwrap());
                assert_eq!(output.as_slice(), *want, "{mode:?} {dir:?}, cuts {ends:?}");
            }
            Err(code) => assert_hal_error(result, *code),
        }
    }

    fn check_all_splits(
        mode: Mode,
        dir: SymmetricOperation,
        input: &[u8],
        expected: Result<&[u8], ErrorCode>,
    ) {
        check_chunks(mode, dir, input, &expected, &[input.len()]);
        if input.is_empty() {
            check_chunks(mode, dir, input, &expected, &[]);
        }
        // Every pair of cut positions includes all two- and three-part splits,
        // including empty updates before, between, and after data.
        for first in 0..=input.len() {
            for second in first..=input.len() {
                check_chunks(mode, dir, input, &expected, &[first, second, input.len()]);
            }
        }
        // Also exercise arbitrarily many updates, down to one byte at a time.
        for size in 1..=input.len().max(1) {
            let mut ends: Vec<_> = (size..input.len()).step_by(size).collect();
            ends.push(input.len());
            check_chunks(mode, dir, input, &expected, &ends);
        }
    }

    // Fixed vectors generated independently with Node.js node:crypto using
    // createCipheriv("des-ede3" / "des-ede3-cbc", KEY, null / NONCE), followed
    // by update + final. setAutoPadding(false) selects NoPadding; otherwise
    // PKCS#7 is enabled. Plaintext is the byte sequence 00, 01, ... .
    #[test]
    fn no_padding_fixed_vectors_match_for_all_update_splits() {
        let vectors = [
            (0, ["", ""]),
            (8, ["1ea0e5b92a35e19e", "3bd0249d89b56676"]),
            (
                24,
                [
                    "1ea0e5b92a35e19e1b093414b11b1d4a983422af76b63189",
                    "3bd0249d89b5667613c074710acf02b21f7cf44dd0b0c9e7",
                ],
            ),
        ];
        for (length, ciphertexts) in vectors {
            let plaintext: Vec<u8> = (0..length).collect();
            for (mode, ciphertext) in NO_PADDING.into_iter().zip(ciphertexts) {
                let ciphertext = from_hex(ciphertext);
                check_all_splits(mode, Encrypt, &plaintext, Ok(&ciphertext));
                check_all_splits(mode, Decrypt, &ciphertext, Ok(&plaintext));
            }
        }
    }

    #[test]
    fn pkcs7_fixed_vectors_match_for_all_update_splits() {
        let vectors = [
            (0, ["538a86fcaffb9a6a", "ed34f69d00068bcd"]),
            (1, ["9abcb8baf605359e", "de9730eb98fcd5f3"]),
            (2, ["6b3b3ddcd0166644", "cdad5a92c55bc0fb"]),
            (3, ["a4b5846c2c5445f1", "78d101427a785286"]),
            (4, ["9b90479e0006f0bd", "8675af7c67d4fcca"]),
            (5, ["b4b57011a6024d25", "07a9101cdad6474b"]),
            (6, ["d5aa06aa9e3448fd", "3c8e532e060d088b"]),
            (7, ["de997424dc348a74", "edf4710571e26c40"]),
            (
                8,
                [
                    "1ea0e5b92a35e19e538a86fcaffb9a6a",
                    "3bd0249d89b56676ef53a62ac9af48f5",
                ],
            ),
            (
                24,
                [
                    "1ea0e5b92a35e19e1b093414b11b1d4a983422af76b63189538a86fcaffb9a6a",
                    "3bd0249d89b5667613c074710acf02b21f7cf44dd0b0c9e797ae3145f606b6d9",
                ],
            ),
            (
                29,
                [
                    "1ea0e5b92a35e19e1b093414b11b1d4a983422af76b631896274bf121274119e",
                    "3bd0249d89b5667613c074710acf02b21f7cf44dd0b0c9e707ca693196f47938",
                ],
            ),
        ];
        // Lengths 0..=7 exercise every legal padding length 8..=1. Aligned
        // nonempty inputs require a separate full padding block at finish.
        for (length, ciphertexts) in vectors {
            let plaintext: Vec<u8> = (0..length).collect();
            for (mode, ciphertext) in PKCS7.into_iter().zip(ciphertexts) {
                let ciphertext = from_hex(ciphertext);
                check_all_splits(mode, Encrypt, &plaintext, Ok(&ciphertext));
                check_all_splits(mode, Decrypt, &ciphertext, Ok(&plaintext));
            }
        }
    }

    #[test]
    fn pkcs7_rejects_invalid_padding_for_all_update_splits() {
        // Independently encrypted with NoPadding: pad=0, sixteen bytes of
        // 0x09 (invalid even though nine bytes fit in the message), and a
        // mismatched three-byte padding suffix ending 02 03 03.
        let invalid = [
            ["30978094100ff75c", "01a2248ca8a482bb"],
            [
                "4afe42799ae17ad44afe42799ae17ad4",
                "bf21d5264d9078efc62e097bfd62af7c",
            ],
            ["175ce9b6c2bb555b", "8a94766cb5c2f84e"],
        ];
        // Android 17 boringssl/src/err.rs maps CIPHER_R_BAD_DECRYPT to
        // InvalidArgument, unlike incomplete or missing final blocks.
        for ciphertexts in invalid {
            for (mode, ciphertext) in PKCS7.into_iter().zip(ciphertexts) {
                check_all_splits(
                    mode,
                    Decrypt,
                    &from_hex(ciphertext),
                    Err(ErrorCode::InvalidArgument),
                );
            }
        }
    }

    #[test]
    fn pkcs7_rejects_empty_and_truncated_ciphertext() {
        let ciphertexts = [
            "1ea0e5b92a35e19e1b093414b11b1d4a983422af76b631896274bf121274119e",
            "3bd0249d89b5667613c074710acf02b21f7cf44dd0b0c9e707ca693196f47938",
        ];
        for (mode, ciphertext) in PKCS7.into_iter().zip(ciphertexts) {
            let ciphertext = from_hex(ciphertext);
            for length in 0..ciphertext.len() {
                let expected = if length == 0 || !length.is_multiple_of(BLOCK) {
                    ErrorCode::InvalidInputLength
                } else {
                    // At block-aligned truncations, this vector ends in 07,
                    // 0f, or 17, none of which forms valid PKCS#7 padding.
                    ErrorCode::InvalidArgument
                };
                check_all_splits(mode, Decrypt, &ciphertext[..length], Err(expected));
            }
        }
    }

    #[test]
    fn no_padding_rejects_partial_blocks_in_both_directions() {
        let input: Vec<u8> = (0..24).collect();
        for mode in NO_PADDING {
            for dir in [Encrypt, Decrypt] {
                for length in 1..input.len() {
                    if !length.is_multiple_of(BLOCK) {
                        check_all_splits(
                            mode,
                            dir,
                            &input[..length],
                            Err(ErrorCode::InvalidInputLength),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn unpad_limits_padding_to_one_des_block() {
        // Guard the unpad helper itself against accepting pad=9 merely
        // because the entire message is longer than one block.
        assert_hal_error(pkcs7_unpad(&[9; 2 * BLOCK]), ErrorCode::InvalidArgument);
        assert_hal_error(pkcs7_unpad(&[]), ErrorCode::InvalidInputLength);
        assert_hal_error(pkcs7_unpad(&[1; BLOCK - 1]), ErrorCode::InvalidInputLength);
    }
}
