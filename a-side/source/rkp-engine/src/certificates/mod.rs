use anyhow::{Context, Result, ensure};
use p256::PublicKey;
use x509_cert::{
    Certificate,
    der::Decode,
    ext::pkix::{BasicConstraints, KeyUsage},
};

use crate::keybox_xml::{ParsedCertificate, parse_der_cert_chain};

mod uds;
mod validation;

pub use uds::validate_uds;

const MAX_CHAINS: usize = 32;
const MAX_CHAIN_BYTES: usize = 64 * 1024;
const MAX_CERTIFICATES: usize = 8;

/// selects the single issued path bound to the requested key, irrespective of response order
pub fn select_chain_for_p256(
    chains: &[Vec<u8>],
    expected_public: &[u8],
) -> Result<Vec<ParsedCertificate>> {
    ensure!(
        !chains.is_empty() && chains.len() <= MAX_CHAINS,
        "invalid certificate chain count"
    );
    let expected = PublicKey::from_sec1_bytes(expected_public)
        .context("invalid requested P-256 public key")?;
    let mut selected = None;
    for chain in chains {
        let parsed = parse_chain(chain)?;
        let certs = decode_certificates(&parsed)?;
        let matching = certs
            .iter()
            .enumerate()
            .filter(|(_, cert)| validation::p256_public_key(cert).as_ref() == Some(&expected))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            continue;
        }
        ensure!(
            matching.len() == 1,
            "requested key appears in multiple certificates"
        );
        let order = validation::ordered_path(&certs, matching[0])?;
        validate_attestation_path(&certs, &order)?;
        ensure!(
            selected.is_none(),
            "multiple certificate chains match the requested key"
        );
        selected = Some(
            order
                .into_iter()
                .map(|index| parsed[index].clone())
                .collect(),
        );
    }
    selected.context("no issued certificate chain matches the requested P-256 key")
}

fn validate_attestation_path(certificates: &[Certificate], order: &[usize]) -> Result<()> {
    let target = certificates[order[0]].tbs_certificate();
    let (critical, constraints) = target
        .get_extension::<BasicConstraints>()?
        .context("attestation target lacks CA basic constraints")?;
    ensure!(
        critical && constraints.ca,
        "attestation target is not a certificate authority"
    );
    if let Some((_, usage)) = target.get_extension::<KeyUsage>()? {
        ensure!(
            usage.key_cert_sign(),
            "attestation target key usage forbids certificate signing"
        );
    }
    // the exported key will become an intermediate when it signs an application certificate
    for (position, index) in order.iter().enumerate().skip(1) {
        let issuer = certificates[*index].tbs_certificate();
        let (_, constraints) = issuer
            .get_extension::<BasicConstraints>()?
            .context("issuer lacks CA basic constraints")?;
        if let Some(limit) = constraints.path_len_constraint {
            let intermediates = order[..position]
                .iter()
                .filter(|index| {
                    let tbs = certificates[**index].tbs_certificate();
                    tbs.subject() != tbs.issuer()
                })
                .count();
            ensure!(
                intermediates <= usize::from(limit),
                "issuer path length cannot accommodate a future application certificate"
            );
        }
    }
    Ok(())
}

fn parse_chain(bytes: &[u8]) -> Result<Vec<ParsedCertificate>> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_CHAIN_BYTES,
        "invalid certificate chain size"
    );
    let parsed = parse_der_cert_chain(bytes)?;
    ensure!(
        parsed.len() <= MAX_CERTIFICATES,
        "too many certificates in one chain"
    );
    for (index, cert) in parsed.iter().enumerate() {
        ensure!(
            !parsed[..index]
                .iter()
                .any(|previous| previous.der == cert.der),
            "duplicate certificate in chain"
        );
    }
    Ok(parsed)
}

fn decode_certificates(parsed: &[ParsedCertificate]) -> Result<Vec<Certificate>> {
    parsed
        .iter()
        .map(|cert| Certificate::from_der(&cert.der).context("decode certificate"))
        .collect()
}
