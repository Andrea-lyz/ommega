// Copyright 2024, The Android Open Source Project
// SPDX-License-Identifier: Apache-2.0
//! Pure AOSP certificate assembly/fallback contract. No device calls.
use super::{Certificate, CertificateChain};

pub(super) fn process_chain_with<E>(
    enabled: bool,
    mut certificates: Vec<Certificate>,
    attestation_certs: Vec<u8>,
    processor: impl FnOnce(CertificateChain) -> Result<CertificateChain, E>,
) -> Vec<Certificate> {
    if !enabled || certificates.is_empty() {
        certificates.push(Certificate {
            encodedCertificate: attestation_certs,
        });
        return certificates;
    }
    let request = CertificateChain {
        leafCertificate: certificates[0].encodedCertificate.clone(),
        remainingChain: attestation_certs.clone(),
    };
    match processor(request) {
        Ok(processed) => vec![
            Certificate {
                encodedCertificate: processed.leafCertificate,
            },
            Certificate {
                encodedCertificate: processed.remainingChain,
            },
        ],
        Err(_) => {
            certificates.push(Certificate {
                encodedCertificate: attestation_certs,
            });
            certificates
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cert(bytes: &[u8]) -> Certificate {
        Certificate {
            encodedCertificate: bytes.to_vec(),
        }
    }
    fn encoded(certificates: &[Certificate]) -> Vec<Vec<u8>> {
        certificates
            .iter()
            .map(|cert| cert.encodedCertificate.clone())
            .collect()
    }

    #[test]
    fn disabled_never_looks_up_or_calls_the_service() {
        let result = process_chain_with::<()>(
            false,
            vec![cert(b"leaf"), cert(b"extra")],
            b"rkp".to_vec(),
            |_| panic!("postprocessor disabled"),
        );
        assert_eq!(
            encoded(&result),
            vec![b"leaf".to_vec(), b"extra".to_vec(), b"rkp".to_vec()]
        );
    }
    #[test]
    fn no_leaf_never_calls_the_service() {
        let result = process_chain_with::<()>(true, vec![], b"rkp".to_vec(), |_| panic!("no leaf"));
        assert_eq!(encoded(&result), vec![b"rkp".to_vec()]);
    }
    #[test]
    fn success_replaces_both_parts_with_exact_service_output() {
        let result = process_chain_with::<()>(
            true,
            vec![cert(b"leaf"), cert(b"unexpected")],
            b"rkp".to_vec(),
            |request| {
                assert_eq!(request.leafCertificate, b"leaf");
                assert_eq!(request.remainingChain, b"rkp");
                Ok(CertificateChain {
                    leafCertificate: b"new-leaf".to_vec(),
                    remainingChain: b"new-rkp".to_vec(),
                })
            },
        );
        assert_eq!(
            encoded(&result),
            vec![b"new-leaf".to_vec(), b"new-rkp".to_vec()]
        );
    }
    #[test]
    fn processing_failure_retains_all_original_certificates() {
        let result = process_chain_with(
            true,
            vec![cert(b"leaf"), cert(b"unexpected")],
            b"rkp".to_vec(),
            |_| Err("binder failure"),
        );
        assert_eq!(
            encoded(&result),
            vec![b"leaf".to_vec(), b"unexpected".to_vec(), b"rkp".to_vec()]
        );
    }
    #[test]
    fn unavailable_service_is_a_lossless_fallback() {
        let result = process_chain_with(true, vec![cert(b"leaf")], b"rkp".to_vec(), |_| {
            Err("service not registered")
        });
        assert_eq!(encoded(&result), vec![b"leaf".to_vec(), b"rkp".to_vec()]);
    }
    #[test]
    fn empty_service_output_is_not_synthesized_or_rewritten() {
        let result = process_chain_with::<()>(true, vec![cert(b"leaf")], b"rkp".to_vec(), |_| {
            Ok(CertificateChain {
                leafCertificate: vec![],
                remainingChain: vec![],
            })
        });
        assert_eq!(encoded(&result), vec![Vec::<u8>::new(), Vec::<u8>::new()]);
    }
}
