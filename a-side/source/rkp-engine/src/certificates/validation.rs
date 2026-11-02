use std::{
    collections::BTreeSet,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use x509_cert::{
    Certificate,
    der::{Decode, Encode, asn1::AnyRef},
    ext::pkix::{AuthorityKeyIdentifier, BasicConstraints, KeyUsage, SubjectKeyIdentifier},
};

pub(super) fn p256_public_key(cert: &Certificate) -> Option<p256::PublicKey> {
    let spki = cert.tbs_certificate().subject_public_key_info();
    if spki.algorithm.oid.to_string() != "1.2.840.10045.2.1" {
        return None;
    }
    let curve = spki
        .algorithm
        .parameters
        .as_ref()?
        .decode_as::<x509_cert::der::asn1::ObjectIdentifier>()
        .ok()?;
    if curve.to_string() != "1.2.840.10045.3.1.7" {
        return None;
    }
    p256::PublicKey::from_sec1_bytes(spki.subject_public_key.as_bytes()?).ok()
}

pub(super) fn ordered_path(certs: &[Certificate], leaf: usize) -> Result<Vec<usize>> {
    let mut order = vec![leaf];
    loop {
        let current = *order.last().context("empty certificate path")?;
        let certificate = &certs[current];
        validate_extensions(certificate)?;
        let tbs = certificate.tbs_certificate();
        if tbs.subject() == tbs.issuer() && verify_signature(certificate, certificate).is_ok() {
            ensure!(
                order.len() == certs.len(),
                "certificate chain contains unrelated certificates"
            );
            break;
        }
        let issuers = certs
            .iter()
            .enumerate()
            .filter(|(index, issuer)| {
                !order.contains(index)
                    && issuer.tbs_certificate().subject() == tbs.issuer()
                    && verify_signature(certificate, issuer).is_ok()
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        ensure!(
            issuers.len() == 1,
            "certificate issuer is missing, ambiguous, cyclic or has an invalid signature"
        );
        validate_issuer(certificate, &certs[issuers[0]], &order, certs)?;
        order.push(issuers[0]);
    }
    Ok(order)
}

fn validate_extensions(cert: &Certificate) -> Result<()> {
    let tbs = cert.tbs_certificate();
    let mut seen = BTreeSet::new();
    for extension in tbs.extensions().into_iter().flatten() {
        ensure!(seen.insert(extension.extn_id), "duplicate X.509 extension");
        if extension.critical {
            ensure!(
                ["2.5.29.19", "2.5.29.15", "2.5.29.35", "2.5.29.14"]
                    .contains(&extension.extn_id.to_string().as_str()),
                "unsupported critical X.509 extension"
            );
        }
    }
    let constraints = tbs.get_extension::<BasicConstraints>()?;
    if let Some((_, constraints)) = constraints {
        ensure!(
            constraints.ca || constraints.path_len_constraint.is_none(),
            "non-CA certificate has path length constraint"
        );
    }
    tbs.get_extension::<KeyUsage>()?;
    tbs.get_extension::<AuthorityKeyIdentifier>()?;
    tbs.get_extension::<SubjectKeyIdentifier>()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let validity = tbs.validity();
    ensure!(
        validity.not_before.to_unix_duration() <= now
            && now <= validity.not_after.to_unix_duration(),
        "certificate is outside its validity period"
    );
    Ok(())
}

fn validate_issuer(
    child: &Certificate,
    issuer: &Certificate,
    below: &[usize],
    certs: &[Certificate],
) -> Result<()> {
    let tbs = issuer.tbs_certificate();
    let (critical, constraints) = tbs
        .get_extension::<BasicConstraints>()?
        .context("issuer lacks CA basic constraints")?;
    ensure!(
        critical && constraints.ca,
        "issuer is not a certificate authority"
    );
    if let Some((_, usage)) = tbs.get_extension::<KeyUsage>()? {
        ensure!(
            usage.key_cert_sign(),
            "issuer key usage forbids certificate signing"
        );
    }
    if let Some(limit) = constraints.path_len_constraint {
        let intermediates = below
            .iter()
            .skip(1)
            .filter(|index| {
                let tbs = certs[**index].tbs_certificate();
                tbs.subject() != tbs.issuer()
            })
            .count();
        ensure!(
            intermediates <= usize::from(limit),
            "issuer path length constraint exceeded"
        );
    }
    if let Some((_, authority)) = child
        .tbs_certificate()
        .get_extension::<AuthorityKeyIdentifier>()?
    {
        if let Some(identifier) = authority.key_identifier
            && let Some((_, subject)) = tbs.get_extension::<SubjectKeyIdentifier>()?
        {
            ensure!(
                identifier == subject.0,
                "authority and subject key identifiers disagree"
            );
        }
        if let Some(serial) = authority.authority_cert_serial_number {
            ensure!(
                &serial == tbs.serial_number(),
                "authority certificate serial disagrees"
            );
        }
    }
    Ok(())
}

pub(super) fn verify_signature(cert: &Certificate, issuer: &Certificate) -> Result<()> {
    let tbs = cert.tbs_certificate();
    ensure!(
        tbs.signature() == cert.signature_algorithm(),
        "certificate signature algorithms disagree"
    );
    let public = issuer.tbs_certificate().subject_public_key_info();
    let public_algorithm = public.algorithm.to_der()?;
    let signature_algorithm = cert.signature_algorithm().to_der()?;
    let public_algorithm = AnyRef::from_der(&public_algorithm)?;
    let signature_algorithm = AnyRef::from_der(&signature_algorithm)?;
    let message = tbs.to_der()?;
    // Port note: the original pulled the aws-lc provider; this build uses the
    // ring provider the rest of the module already ships for Android.
    let provider = rustls::crypto::ring::default_provider();
    ensure!(
        provider
            .signature_verification_algorithms
            .all
            .iter()
            .any(|algorithm| {
                algorithm.public_key_alg_id().as_ref() == public_algorithm.value()
                    && algorithm.signature_alg_id().as_ref() == signature_algorithm.value()
                    && public
                        .subject_public_key
                        .as_bytes()
                        .zip(cert.signature().as_bytes())
                        .is_some_and(|(key, signature)| {
                            algorithm.verify_signature(key, &message, signature).is_ok()
                        })
            }),
        "certificate signature is invalid or unsupported"
    );
    Ok(())
}
