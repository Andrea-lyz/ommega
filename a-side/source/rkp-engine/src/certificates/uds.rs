use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use ciborium::value::Value;
use x509_cert::Certificate;

use super::{MAX_CERTIFICATES, MAX_CHAINS, decode_certificates, parse_chain, validation};
use crate::cbor::{as_array, as_bytes, as_map};

/// checks internal UDS path consistency and its binding to the first DICE public key
pub fn validate_uds(value: &Value, root_public: &[u8], algorithm: i128) -> Result<()> {
    let expected_len = match algorithm {
        -8 => 32,
        -7 => 64,
        -35 => 96,
        _ => anyhow::bail!("unsupported DICE root public key algorithm"),
    };
    ensure!(
        root_public.len() == expected_len,
        "invalid DICE root public key length"
    );
    let entries = as_map(value, "UDS certificates")?;
    ensure!(entries.len() <= MAX_CHAINS, "invalid UDS signer count");
    let mut signers = BTreeSet::new();
    for (signer, chain) in entries {
        let Value::Text(name) = signer else {
            anyhow::bail!("UDS signer name must be text");
        };
        ensure!(
            !name.is_empty() && name.len() <= 128 && signers.insert(name),
            "invalid or duplicate UDS signer name"
        );
        let chain = as_array(chain, "UDS certificate chain")?;
        ensure!(
            !chain.is_empty() && chain.len() <= MAX_CERTIFICATES,
            "invalid UDS certificate count"
        );
        let mut stream = Vec::new();
        for entry in chain {
            let der = as_bytes(entry, "UDS certificate")?;
            ensure!(
                parse_chain(der)?.len() == 1,
                "UDS certificate entry must contain one DER certificate"
            );
            stream.extend_from_slice(der);
        }
        let parsed = parse_chain(&stream)?;
        let certificates = decode_certificates(&parsed)?;
        let leaf_index = certificates.len() - 1;
        ensure!(
            matches_root(&certificates[leaf_index], root_public, algorithm)?,
            "UDS leaf does not match the DICE root public key"
        );
        let order = validation::ordered_path(&certificates, leaf_index)?;
        ensure!(
            order.iter().copied().eq((0..certificates.len()).rev()),
            "UDS certificate chain must be root to leaf"
        );
    }
    Ok(())
}

fn matches_root(certificate: &Certificate, expected: &[u8], algorithm: i128) -> Result<bool> {
    let public = certificate.tbs_certificate().subject_public_key_info();
    if algorithm == -8 {
        return Ok(public.algorithm.oid.to_string() == "1.3.101.112"
            && public.algorithm.parameters.is_none()
            && public.subject_public_key.as_bytes() == Some(expected));
    }
    use x509_cert::der::asn1::ObjectIdentifier;
    let curve = public
        .algorithm
        .parameters
        .as_ref()
        .map(|value| value.decode_as::<ObjectIdentifier>())
        .transpose()?;
    if public.algorithm.oid.to_string() != "1.2.840.10045.2.1" {
        return Ok(false);
    }
    let curve = curve.map(|value| value.to_string());
    let mut point = vec![4];
    point.extend_from_slice(expected);
    let Some(encoded) = public.subject_public_key.as_bytes() else {
        return Ok(false);
    };
    Ok(match algorithm {
        -7 if curve.as_deref() == Some("1.2.840.10045.3.1.7") => {
            p256::PublicKey::from_sec1_bytes(encoded)
                .ok()
                .zip(p256::PublicKey::from_sec1_bytes(&point).ok())
                .is_some_and(|(actual, expected)| actual == expected)
        }
        -35 if curve.as_deref() == Some("1.3.132.0.34") => {
            p384::PublicKey::from_sec1_bytes(encoded)
                .ok()
                .zip(p384::PublicKey::from_sec1_bytes(&point).ok())
                .is_some_and(|(actual, expected)| actual == expected)
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::validate_uds;
    use ciborium::value::Value;

    // The path-building cases of the original crate used a shared certificate
    // fixture (certificates/tests.rs) that is not part of this port; the
    // engine-level verification is exercised by verify/ and by the host
    // examples instead.
    #[test]
    fn empty_optional_uds_map_is_valid_without_claiming_a_trust_anchor() {
        validate_uds(&Value::Map(vec![]), &[1; 32], -8).unwrap();
    }
}
