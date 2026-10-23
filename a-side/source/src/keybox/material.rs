//! Keybox material parsing, validation, identity and security-level selection.

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use der::Encode;
use kmr_common::{
    crypto::{ec, rsa, KeyMaterial, Sha256},
    Error,
};
use kmr_crypto_ommega::{ec::OmmegaEc, mldsa::OmmegaMlDsa, rsa::OmmegaRsa, sha256::OmmegaSha256};
use kmr_ta::device::{SigningAlgorithm, SigningInfoSnapshot, SigningKey, SigningKeyType};
use kmr_wire::keymint;
use regex::Regex;
use x509_cert::der as x509_der;
use x509_cert::Certificate;

use super::BUNDLED_KEYBOX_XML;

lazy_static::lazy_static! {
    static ref KEY_BLOCK_RE: Regex =
        Regex::new(r#"(?s)<Key\s+algorithm="([^"]+)">\s*(.*?)\s*</Key>"#).unwrap();
    static ref PRIVATE_KEY_RE: Regex =
        Regex::new(r#"(?s)<PrivateKey[^>]*>\s*(.*?)\s*</PrivateKey>"#).unwrap();
    static ref CERT_COUNT_RE: Regex =
        Regex::new(r#"(?s)<NumberOfCertificates>\s*(\d+)\s*</NumberOfCertificates>"#).unwrap();
    static ref CERT_RE: Regex =
        Regex::new(r#"(?s)<Certificate(?:\s+[^>]*)?>\s*(.*?)\s*</Certificate>"#).unwrap();
}

#[derive(Clone)]
pub struct CertSignAlgoInfo {
    key: KeyMaterial,
    key_der: Vec<u8>,
    chain: Vec<keymint::Certificate>,
}

#[derive(Clone)]
pub struct KeyBox {
    rsa_info: Option<CertSignAlgoInfo>,
    ec_info: Option<CertSignAlgoInfo>,
    strongbox_rsa_info: Option<CertSignAlgoInfo>,
    strongbox_ec_info: Option<CertSignAlgoInfo>,
    identity_digest: [u8; 32],
}

#[derive(Clone, Copy)]
pub(super) enum KeyAlgorithm {
    Ec,
    Rsa,
}

struct ParsedKeyEntry {
    key_der: Vec<u8>,
    chain: Vec<Vec<u8>>,
}

impl KeyBox {
    pub fn new() -> Self {
        Self::from_xml_str(BUNDLED_KEYBOX_XML).expect("bundled keybox.xml must be valid")
    }

    pub fn from_xml_str(xml: &str) -> Result<Self> {
        let (rsa_entry, ec_entry) = Self::parse_key_entries(xml)?;
        let rsa_info = rsa_entry
            .map(|entry| Self::build_info(entry, KeyAlgorithm::Rsa))
            .transpose()?;
        let ec_info = ec_entry
            .map(|entry| Self::build_info(entry, KeyAlgorithm::Ec))
            .transpose()?;
        if rsa_info.is_none() && ec_info.is_none() {
            bail!("keybox.xml must contain at least one RSA or EC key entry");
        }
        let mut keybox = Self {
            rsa_info,
            ec_info,
            strongbox_rsa_info: None,
            strongbox_ec_info: None,
            identity_digest: [0u8; 32],
        };
        keybox.refresh_identity_digest()?;
        Ok(keybox)
    }

    /// Parse the RSA/EC `<Key>` entries of one keybox document.
    fn parse_key_entries(xml: &str) -> Result<(Option<ParsedKeyEntry>, Option<ParsedKeyEntry>)> {
        let mut rsa_entry = None;
        let mut ec_entry = None;
        for captures in KEY_BLOCK_RE.captures_iter(xml) {
            let algorithm = match captures.get(1).map(|m| m.as_str().trim()) {
                Some("ecdsa") | Some("ec") => KeyAlgorithm::Ec,
                Some("rsa") => KeyAlgorithm::Rsa,
                Some(other) => bail!("unsupported key algorithm `{other}` in keybox.xml"),
                None => bail!("missing key algorithm in keybox.xml"),
            };
            let body = captures
                .get(2)
                .map(|m| m.as_str())
                .ok_or_else(|| anyhow!("missing key block body"))?;
            let entry = ParsedKeyEntry::from_xml_block(body).with_context(|| {
                format!("failed to parse {:?} key entry", algorithm_name(algorithm))
            })?;
            match algorithm {
                KeyAlgorithm::Ec => ec_entry = Some(entry),
                KeyAlgorithm::Rsa => rsa_entry = Some(entry),
            }
        }
        Ok((rsa_entry, ec_entry))
    }

    /// Install dedicated StrongBox material from its own keybox document (same
    /// schema as `keybox.xml`). It stays strictly separate from the batch
    /// material, so a StrongBox attestation can never be produced from the TEE
    /// keychain.
    pub fn set_strongbox_material_from_xml(&mut self, xml: &str) -> Result<()> {
        let (rsa_entry, ec_entry) = Self::parse_key_entries(xml)?;
        let rsa_info = rsa_entry
            .map(|entry| Self::build_info(entry, KeyAlgorithm::Rsa))
            .transpose()?;
        let ec_info = ec_entry
            .map(|entry| Self::build_info(entry, KeyAlgorithm::Ec))
            .transpose()?;
        if rsa_info.is_none() && ec_info.is_none() {
            bail!("StrongBox keybox must contain at least one RSA or EC key entry");
        }
        self.strongbox_rsa_info = rsa_info;
        self.strongbox_ec_info = ec_info;
        self.refresh_identity_digest()
    }
    fn build_info(entry: ParsedKeyEntry, algorithm: KeyAlgorithm) -> Result<CertSignAlgoInfo> {
        let name = algorithm_name(algorithm);
        if entry.chain.is_empty() {
            bail!("{name} certificate chain is empty");
        }
        let key = match algorithm {
            KeyAlgorithm::Rsa => rsa::import_pkcs1_key(&entry.key_der).map(|(key, _, _)| key),
            KeyAlgorithm::Ec => ec::import_sec1_private_key(&entry.key_der),
        }
        .map_err(|e| anyhow!("failed to import {name} private key: {e:?}"))?;
        let chain: Vec<keymint::Certificate> = entry
            .chain
            .into_iter()
            .map(|encoded_certificate| keymint::Certificate {
                encoded_certificate,
            })
            .collect();
        validate_chain_matches_key(&key, &chain, algorithm)?;
        Ok(CertSignAlgoInfo {
            key,
            key_der: entry.key_der,
            chain,
        })
    }

    fn compute_identity_digest(
        rsa_info: Option<&CertSignAlgoInfo>,
        ec_info: Option<&CertSignAlgoInfo>,
        strongbox_rsa_info: Option<&CertSignAlgoInfo>,
        strongbox_ec_info: Option<&CertSignAlgoInfo>,
    ) -> Result<[u8; 32]> {
        let mut material = Vec::new();
        if let Some(rsa_info) = rsa_info {
            append_labeled_bytes(&mut material, b"rsa-key", &rsa_info.key_der);
            append_labeled_chain(&mut material, b"rsa-chain", &rsa_info.chain);
        }
        if let Some(ec_info) = ec_info {
            append_labeled_bytes(&mut material, b"ec-key", &ec_info.key_der);
            append_labeled_chain(&mut material, b"ec-chain", &ec_info.chain);
        }
        if let Some(rsa_info) = strongbox_rsa_info {
            append_labeled_bytes(&mut material, b"strongbox-rsa-key", &rsa_info.key_der);
            append_labeled_chain(&mut material, b"strongbox-rsa-chain", &rsa_info.chain);
        }
        if let Some(ec_info) = strongbox_ec_info {
            append_labeled_bytes(&mut material, b"strongbox-ec-key", &ec_info.key_der);
            append_labeled_chain(&mut material, b"strongbox-ec-chain", &ec_info.chain);
        }

        OmmegaSha256 {}
            .hash(&material)
            .map_err(|e| anyhow!("failed to hash keybox identity: {e:?}"))
    }

    fn refresh_identity_digest(&mut self) -> Result<()> {
        self.identity_digest = Self::compute_identity_digest(
            self.rsa_info.as_ref(),
            self.ec_info.as_ref(),
            self.strongbox_rsa_info.as_ref(),
            self.strongbox_ec_info.as_ref(),
        )?;
        Ok(())
    }

    pub fn identity_digest(&self) -> [u8; 32] {
        self.identity_digest
    }

    #[cfg(test)]
    fn signing_info(&self, key_type: SigningKeyType) -> Result<SigningInfoSnapshot, Error> {
        self.signing_info_at(keymint::SecurityLevel::TrustedEnvironment, key_type)
    }

    /// Snapshot the attestation signing material for one security level.
    ///
    /// StrongBox attestations use their own material only; the batch/TEE chain is
    /// never reused for them, because a TEE chain labelled StrongBox is externally
    /// distinguishable from a real device's separate StrongBox chain.
    pub fn signing_info_at(
        &self,
        security_level: keymint::SecurityLevel,
        key_type: SigningKeyType,
    ) -> Result<SigningInfoSnapshot, Error> {
        let strongbox = matches!(security_level, keymint::SecurityLevel::Strongbox);
        match key_type.which {
            SigningKey::DeviceUnique => {
                if !strongbox {
                    return Err(kmr_common::km_err!(
                        InvalidArgument,
                        "device unique attestation supported only by Strongbox TA"
                    ));
                }
                // AOSP explicitly allows a StrongBox implementation without
                // device-unique support to reject the request with this code.
                return Err(kmr_common::km_err!(
                    CannotAttestIds,
                    "device unique attestation is not supported"
                ));
            }
            SigningKey::Batch => {}
        }

        let info = match (strongbox, key_type.algo_hint) {
            // The issuer key type and the leaf key type are independent: an RSA
            // leaf can be signed by an EC key. Most distributed keyboxes carry
            // only an EC entry, so fall back to the other algorithm instead of
            // failing every RSA attestation. StrongBox material stays separate.
            (false, SigningAlgorithm::Rsa) => self.rsa_info.as_ref().or(self.ec_info.as_ref()),
            (false, SigningAlgorithm::Ec) => self.ec_info.as_ref().or(self.rsa_info.as_ref()),
            (true, SigningAlgorithm::Rsa) => self.strongbox_rsa_info.as_ref(),
            (true, SigningAlgorithm::Ec) => self.strongbox_ec_info.as_ref(),
        };
        let info = info.ok_or_else(|| {
            if strongbox {
                kmr_common::km_err!(
                    AttestationKeysNotProvisioned,
                    "no dedicated StrongBox keybox is configured"
                )
            } else {
                kmr_common::km_err!(
                    UnknownError,
                    "keybox has no {:?} signing key",
                    key_type.algo_hint
                )
            }
        })?;

        Ok(SigningInfoSnapshot {
            signing_key: info.key.clone(),
            cert_chain: info.chain.clone(),
            identity_digest: self.identity_digest,
        })
    }

    pub(super) fn signing_certificate_ders(&self) -> Vec<Vec<u8>> {
        [
            self.rsa_info.as_ref(),
            self.ec_info.as_ref(),
            self.strongbox_rsa_info.as_ref(),
            self.strongbox_ec_info.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(|info| info.chain[0].encoded_certificate.clone())
        .collect()
    }

    pub(super) fn update_keybox(
        &mut self,
        algorithm: KeyAlgorithm,
        key_der: Vec<u8>,
        chain: Vec<keymint::Certificate>,
    ) -> Result<()> {
        let entry = ParsedKeyEntry {
            key_der,
            chain: chain
                .into_iter()
                .map(|certificate| certificate.encoded_certificate)
                .collect(),
        };
        let info = Self::build_info(entry, algorithm)?;
        match algorithm {
            KeyAlgorithm::Ec => self.ec_info = Some(info),
            KeyAlgorithm::Rsa => self.rsa_info = Some(info),
        }
        self.refresh_identity_digest()
    }

    pub fn to_xml_string(&self) -> String {
        let blocks = [
            self.ec_info
                .as_ref()
                .map(|info| ("ecdsa", "EC PRIVATE KEY", info)),
            self.rsa_info
                .as_ref()
                .map(|info| ("rsa", "RSA PRIVATE KEY", info)),
        ]
        .into_iter()
        .flatten()
        .map(|(name, private_label, info)| Self::to_xml_block(name, private_label, info))
        .collect::<Vec<_>>()
        .join("\n");

        format!(
            concat!(
                "<?xml version=\"1.0\"?>\n",
                "<AndroidAttestation>\n",
                "<NumberOfKeyboxes>{keybox_count}</NumberOfKeyboxes>\n",
                "<Keybox DeviceID=\"sw\">\n",
                "{blocks}\n",
                "</Keybox>\n",
                "</AndroidAttestation>\n"
            ),
            keybox_count = if self.rsa_info.is_some() && self.ec_info.is_some() {
                2
            } else {
                1
            },
            blocks = blocks,
        )
    }

    fn to_xml_block(name: &str, private_label: &str, info: &CertSignAlgoInfo) -> String {
        let certificates = info
            .chain
            .iter()
            .map(|certificate| {
                format!(
                    "<Certificate format=\"pem\">\n{}\n</Certificate>",
                    encode_pem_block("CERTIFICATE", &certificate.encoded_certificate)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            concat!(
                "<Key algorithm=\"{name}\">\n",
                "<PrivateKey format=\"pem\">\n",
                "{private_key}\n",
                "</PrivateKey>\n",
                "<CertificateChain>\n",
                "<NumberOfCertificates>{cert_count}</NumberOfCertificates>\n",
                "{certificates}\n",
                "</CertificateChain>\n",
                "</Key>"
            ),
            name = name,
            private_key = encode_pem_block(private_label, &info.key_der),
            cert_count = info.chain.len(),
            certificates = certificates,
        )
    }
}

impl Default for KeyBox {
    fn default() -> Self {
        Self::new()
    }
}

impl ParsedKeyEntry {
    fn from_xml_block(block: &str) -> Result<Self> {
        let private_key_pem = PRIVATE_KEY_RE
            .captures(block)
            .and_then(|captures| captures.get(1))
            .map(|m| m.as_str())
            .context("missing <PrivateKey> block")?;
        let key_der = decode_pem(private_key_pem)?;

        let expected_cert_count = CERT_COUNT_RE
            .captures(block)
            .and_then(|captures| captures.get(1))
            .map(|m| m.as_str())
            .context("missing <NumberOfCertificates> in certificate chain")?
            .parse::<usize>()
            .context("invalid certificate count in keybox.xml")?;

        let chain = CERT_RE
            .captures_iter(block)
            .filter_map(|captures| captures.get(1).map(|m| m.as_str()))
            .map(decode_pem)
            .collect::<Result<Vec<_>>>()?;

        if chain.len() != expected_cert_count {
            bail!(
                "certificate count mismatch: declared {}, parsed {}",
                expected_cert_count,
                chain.len()
            );
        }

        Ok(Self { key_der, chain })
    }
}

fn append_labeled_bytes(buffer: &mut Vec<u8>, label: &[u8], data: &[u8]) {
    buffer.extend_from_slice(&(label.len() as u32).to_be_bytes());
    buffer.extend_from_slice(label);
    buffer.extend_from_slice(&(data.len() as u32).to_be_bytes());
    buffer.extend_from_slice(data);
}

fn append_labeled_chain(buffer: &mut Vec<u8>, label: &[u8], chain: &[keymint::Certificate]) {
    buffer.extend_from_slice(&(label.len() as u32).to_be_bytes());
    buffer.extend_from_slice(label);
    buffer.extend_from_slice(&(chain.len() as u32).to_be_bytes());
    for certificate in chain {
        buffer.extend_from_slice(&(certificate.encoded_certificate.len() as u32).to_be_bytes());
        buffer.extend_from_slice(&certificate.encoded_certificate);
    }
}

fn algorithm_name(algorithm: KeyAlgorithm) -> &'static str {
    match algorithm {
        KeyAlgorithm::Ec => "EC",
        KeyAlgorithm::Rsa => "RSA",
    }
}

fn validate_chain_matches_key(
    key: &KeyMaterial,
    chain: &[keymint::Certificate],
    algorithm: KeyAlgorithm,
) -> Result<()> {
    let first_cert = chain
        .first()
        .context("certificate chain must contain a leaf certificate")?;
    let certificate = <Certificate as x509_der::Decode>::from_der(&first_cert.encoded_certificate)
        .with_context(|| {
            format!(
                "failed to parse {} leaf certificate from keybox chain",
                algorithm_name(algorithm)
            )
        })?;
    let mut spki_buf = Vec::new();
    let derived_spki = key
        .subject_public_key_info(
            &mut spki_buf,
            &OmmegaEc::default(),
            &OmmegaRsa::default(),
            &OmmegaMlDsa,
        )
        .map_err(|e| {
            anyhow!(
                "failed to derive {} public key info from private key: {e:?}",
                algorithm_name(algorithm)
            )
        })?
        .context("symmetric key cannot back an attestation certificate")?
        .to_der()
        .with_context(|| {
            format!(
                "failed to encode {} public key info from private key",
                algorithm_name(algorithm)
            )
        })?;
    let certificate_spki =
        x509_der::Encode::to_der(certificate.tbs_certificate().subject_public_key_info())
            .with_context(|| {
                format!(
                    "failed to encode {} public key info from certificate chain",
                    algorithm_name(algorithm)
                )
            })?;
    if derived_spki != certificate_spki {
        bail!(
            "{} certificate chain does not match the supplied private key",
            algorithm_name(algorithm)
        );
    }
    Ok(())
}

fn decode_pem(pem: &str) -> Result<Vec<u8>> {
    let base64_body = pem
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("-----BEGIN ") && !line.starts_with("-----END "))
        .collect::<String>();

    if base64_body.is_empty() {
        bail!("empty PEM payload");
    }

    STANDARD
        .decode(base64_body.as_bytes())
        .context("failed to decode PEM payload")
}

fn encode_pem_block(label: &str, der: &[u8]) -> String {
    let mut pem = String::new();
    pem.push_str(&format!("-----BEGIN {label}-----\n"));
    let base64 = STANDARD.encode(der);
    for chunk in base64.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(chunk).expect("base64 is valid UTF-8"));
        pem.push('\n');
    }
    pem.push_str(&format!("-----END {label}-----"));
    pem
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bundled_template() {
        let keybox = KeyBox::from_xml_str(BUNDLED_KEYBOX_XML).unwrap();
        assert_eq!(keybox.ec_info.as_ref().unwrap().chain.len(), 2);
        assert_eq!(keybox.rsa_info.as_ref().unwrap().chain.len(), 2);
        assert_ne!(keybox.identity_digest(), [0u8; 32]);
    }

    #[test]
    fn parses_ec_only_keybox() {
        let keybox = KeyBox::from_xml_str(BUNDLED_KEYBOX_XML).unwrap();
        let ec_block =
            KeyBox::to_xml_block("ecdsa", "EC PRIVATE KEY", keybox.ec_info.as_ref().unwrap());
        let ec_only_xml = format!(
            "<?xml version=\"1.0\"?>\n<AndroidAttestation>\n<NumberOfKeyboxes>1</NumberOfKeyboxes>\n<Keybox DeviceID=\"yurikey\">\n{ec_block}\n</Keybox>\n</AndroidAttestation>\n"
        );

        let parsed = KeyBox::from_xml_str(&ec_only_xml).unwrap();
        assert!(parsed.ec_info.is_some());
        assert!(parsed.rsa_info.is_none());
        assert_eq!(
            parsed
                .signing_info(SigningKeyType {
                    which: SigningKey::Batch,
                    algo_hint: SigningAlgorithm::Ec,
                })
                .unwrap()
                .cert_chain,
            keybox.ec_info.as_ref().unwrap().chain
        );
        assert_eq!(
            parsed
                .signing_info(SigningKeyType {
                    which: SigningKey::Batch,
                    algo_hint: SigningAlgorithm::Rsa,
                })
                .unwrap()
                .cert_chain,
            keybox.ec_info.as_ref().unwrap().chain
        );
        assert_eq!(parsed.to_xml_string().matches("<Key algorithm=").count(), 1);
        // Round-trips to a valid, single-keybox document.
        let round_tripped = KeyBox::from_xml_str(&parsed.to_xml_string()).unwrap();
        assert_eq!(round_tripped.identity_digest(), parsed.identity_digest());
    }

    #[test]
    fn rejects_invalid_xml() {
        assert!(KeyBox::from_xml_str("<AndroidAttestation/>").is_err());
    }

    #[test]
    fn identity_changes_when_chain_changes() {
        let original = KeyBox::from_xml_str(BUNDLED_KEYBOX_XML).unwrap();
        let mut changed = original.clone();
        let extra_cert = changed.ec_info.as_ref().unwrap().chain[0].clone();
        changed.ec_info.as_mut().unwrap().chain.push(extra_cert);
        changed.refresh_identity_digest().unwrap();

        let modified = KeyBox::from_xml_str(&changed.to_xml_string()).unwrap();
        assert_ne!(original.identity_digest(), modified.identity_digest());
    }

    #[test]
    fn rejects_mismatched_private_key_and_certificate_chain() {
        let keybox = KeyBox::from_xml_str(BUNDLED_KEYBOX_XML).unwrap();
        let rsa_cert = encode_pem_block(
            "CERTIFICATE",
            &keybox.rsa_info.as_ref().unwrap().chain[0].encoded_certificate,
        );
        let ec_cert = encode_pem_block(
            "CERTIFICATE",
            &keybox.ec_info.as_ref().unwrap().chain[0].encoded_certificate,
        );
        let modified_xml = BUNDLED_KEYBOX_XML.replacen(&rsa_cert, &ec_cert, 1);
        assert!(KeyBox::from_xml_str(&modified_xml).is_err());
    }

    #[test]
    fn signing_snapshot_keeps_key_chain_and_digest_in_sync() {
        let keybox = KeyBox::from_xml_str(BUNDLED_KEYBOX_XML).unwrap();

        let rsa_snapshot = keybox
            .signing_info(SigningKeyType {
                which: SigningKey::Batch,
                algo_hint: SigningAlgorithm::Rsa,
            })
            .unwrap();
        assert_eq!(rsa_snapshot.identity_digest, keybox.identity_digest());
        validate_chain_matches_key(
            &rsa_snapshot.signing_key,
            &rsa_snapshot.cert_chain,
            KeyAlgorithm::Rsa,
        )
        .unwrap();

        let ec_snapshot = keybox
            .signing_info(SigningKeyType {
                which: SigningKey::Batch,
                algo_hint: SigningAlgorithm::Ec,
            })
            .unwrap();
        assert_eq!(ec_snapshot.identity_digest, keybox.identity_digest());
        validate_chain_matches_key(
            &ec_snapshot.signing_key,
            &ec_snapshot.cert_chain,
            KeyAlgorithm::Ec,
        )
        .unwrap();
    }

    #[test]
    fn strongbox_batch_attestation_never_uses_the_batch_keybox() {
        let keybox = KeyBox::from_xml_str(BUNDLED_KEYBOX_XML).unwrap();
        let batch = keybox
            .signing_info(SigningKeyType {
                which: SigningKey::Batch,
                algo_hint: SigningAlgorithm::Ec,
            })
            .unwrap();
        assert_eq!(batch.cert_chain.len(), 2);

        let error = keybox
            .signing_info_at(
                keymint::SecurityLevel::Strongbox,
                SigningKeyType {
                    which: SigningKey::Batch,
                    algo_hint: SigningAlgorithm::Ec,
                },
            )
            .err()
            .expect("StrongBox attestation must fail without dedicated material");
        assert!(format!("{error:?}").contains("AttestationKeysNotProvisioned"));
    }

    #[test]
    fn strongbox_material_is_selected_by_security_level() {
        let mut keybox = KeyBox::from_xml_str(BUNDLED_KEYBOX_XML).unwrap();
        let before = keybox.identity_digest();
        let ec_block =
            KeyBox::to_xml_block("ecdsa", "EC PRIVATE KEY", keybox.ec_info.as_ref().unwrap());
        let strongbox_xml = format!(
            r#"<?xml version=\"1.0\"?>
<AndroidAttestation>
<NumberOfKeyboxes>1</NumberOfKeyboxes>
<Keybox DeviceID=\"sb\">
{ec_block}
</Keybox>
</AndroidAttestation>
"#
        );
        keybox
            .set_strongbox_material_from_xml(&strongbox_xml)
            .unwrap();
        assert_ne!(keybox.identity_digest(), before);

        let strongbox_ec = keybox
            .signing_info_at(
                keymint::SecurityLevel::Strongbox,
                SigningKeyType {
                    which: SigningKey::Batch,
                    algo_hint: SigningAlgorithm::Ec,
                },
            )
            .unwrap();
        assert_eq!(
            strongbox_ec.cert_chain,
            keybox.ec_info.as_ref().unwrap().chain
        );

        // The batch RSA entry must stay invisible to the StrongBox level.
        assert!(keybox
            .signing_info_at(
                keymint::SecurityLevel::Strongbox,
                SigningKeyType {
                    which: SigningKey::Batch,
                    algo_hint: SigningAlgorithm::Rsa,
                },
            )
            .is_err());
        assert!(keybox
            .signing_info(SigningKeyType {
                which: SigningKey::Batch,
                algo_hint: SigningAlgorithm::Rsa,
            })
            .is_ok());
    }

    #[test]
    fn device_unique_attestation_is_rejected_per_level() {
        let keybox = KeyBox::from_xml_str(BUNDLED_KEYBOX_XML).unwrap();
        let tee_error = keybox
            .signing_info(SigningKeyType {
                which: SigningKey::DeviceUnique,
                algo_hint: SigningAlgorithm::Ec,
            })
            .err()
            .expect("TEE device-unique attestation must be rejected");
        assert!(format!("{tee_error:?}").contains("InvalidArgument"));

        let strongbox_error = keybox
            .signing_info_at(
                keymint::SecurityLevel::Strongbox,
                SigningKeyType {
                    which: SigningKey::DeviceUnique,
                    algo_hint: SigningAlgorithm::Ec,
                },
            )
            .err()
            .expect("StrongBox device-unique attestation must be rejected");
        assert!(format!("{strongbox_error:?}").contains("CannotAttestIds"));
    }
}
