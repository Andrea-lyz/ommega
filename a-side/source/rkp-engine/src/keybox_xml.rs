use std::fmt::Write as _;

use anyhow::{Context, Result, anyhow, ensure};
use base64ct::{Base64, Encoding};
use p256::SecretKey as P256SecretKey;
use serde::Serialize;
use x509_cert::{
    Certificate,
    der::{Decode, Encode},
};

#[derive(Debug, Clone)]
pub struct ParsedCertificate {
    pub der: Vec<u8>,
    pub subject_der: Vec<u8>,
    pub issuer_der: Vec<u8>,
    pub subject_summary: String,
    pub issuer_summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CertificateChainSummary {
    pub certificates: usize,
    pub subjects: Vec<String>,
}

pub fn parse_der_cert_chain(data: &[u8]) -> Result<Vec<ParsedCertificate>> {
    if data.is_empty() {
        return Err(anyhow!("certificate chain is empty"));
    }

    let mut certificates = Vec::new();
    let mut offset = 0usize;

    while offset < data.len() {
        if data[offset] != 0x30 {
            return Err(anyhow!(
                "certificate chain contains trailing non-DER data at offset {offset}"
            ));
        }

        let (header_len, body_len) = parse_der_len(&data[offset..])?;
        let total_len = header_len
            .checked_add(body_len)
            .ok_or_else(|| anyhow!("DER length overflow"))?;
        let end = offset
            .checked_add(total_len)
            .ok_or_else(|| anyhow!("DER length overflow"))?;
        if end > data.len() {
            return Err(anyhow!("DER certificate is truncated"));
        }

        let slice = &data[offset..end];
        let certificate = Certificate::from_der(slice).context("parse X.509 certificate")?;
        let tbs = certificate.tbs_certificate();
        let subject_der = tbs.subject().to_der().context("encode subject name")?;
        let issuer_der = tbs.issuer().to_der().context("encode issuer name")?;

        certificates.push(ParsedCertificate {
            der: slice.to_vec(),
            subject_der,
            issuer_der,
            subject_summary: tbs.subject().to_string(),
            issuer_summary: tbs.issuer().to_string(),
        });

        offset = end;
    }

    if certificates.is_empty() {
        return Err(anyhow!("certificate chain is empty"));
    }

    Ok(certificates)
}

pub fn summarize_chain(certs: &[ParsedCertificate]) -> CertificateChainSummary {
    CertificateChainSummary {
        certificates: certs.len(),
        subjects: certs
            .iter()
            .map(|cert| cert.subject_summary.clone())
            .collect(),
    }
}

/// encodes the caller's verified leaf-to-root order without discarding equal-name issuers
pub fn build_keybox_xml(
    secret_key: &P256SecretKey,
    ec_cert_chain: &[ParsedCertificate],
    device_id: &str,
) -> Result<String> {
    if ec_cert_chain.is_empty() {
        return Err(anyhow!("certificate chain is empty"));
    }

    let cert_chain = ec_cert_chain;
    let key_der = secret_key
        .to_sec1_der()
        .context("encode EC private key as SEC1 DER")?;
    let key_pem = pem_wrap("EC PRIVATE KEY", key_der.as_slice());
    let device_id = escape_xml_attribute(device_id);

    let mut xml = String::new();
    writeln!(xml, "<?xml version=\"1.0\"?>").unwrap();
    writeln!(xml, "<AndroidAttestation>").unwrap();
    writeln!(xml, "    <NumberOfKeyboxes>1</NumberOfKeyboxes>").unwrap();
    writeln!(xml, "    <Keybox DeviceID=\"{device_id}\">").unwrap();
    writeln!(xml, "        <Key algorithm=\"ecdsa\">").unwrap();
    writeln!(xml, "            <PrivateKey format=\"pem\">").unwrap();
    writeln!(xml, "{}", indent_block(&key_pem, "                ")).unwrap();
    writeln!(xml, "            </PrivateKey>").unwrap();
    writeln!(xml, "            <CertificateChain>").unwrap();
    writeln!(
        xml,
        "                <NumberOfCertificates>{}</NumberOfCertificates>",
        cert_chain.len()
    )
    .unwrap();

    for cert in cert_chain {
        let cert_pem = pem_wrap("CERTIFICATE", &cert.der);
        writeln!(xml, "                <Certificate format=\"pem\">").unwrap();
        writeln!(xml, "{}", indent_block(&cert_pem, "                    ")).unwrap();
        writeln!(xml, "                </Certificate>").unwrap();
    }

    writeln!(xml, "            </CertificateChain>").unwrap();
    writeln!(xml, "        </Key>").unwrap();
    writeln!(xml, "    </Keybox>").unwrap();
    writeln!(xml, "</AndroidAttestation>").unwrap();

    Ok(xml)
}

pub fn validate_keybox_roundtrip(
    xml: &str,
    secret: &P256SecretKey,
    chain: &[ParsedCertificate],
    device_id: &str,
) -> Result<()> {
    let root = parse_keybox_tree(xml)?;
    ensure!(
        root.name == "AndroidAttestation" && root.attributes.is_empty() && root.children.len() == 2,
        "invalid keybox document"
    );
    ensure!(
        root.child(0, "NumberOfKeyboxes")?.text.trim() == "1",
        "invalid keybox count"
    );
    let keybox = root.child(1, "Keybox")?;
    ensure!(
        keybox.attributes == [("DeviceID".into(), device_id.into())] && keybox.children.len() == 1,
        "keybox device identifier differs"
    );
    let key = keybox.child(0, "Key")?;
    ensure!(
        key.attributes == [("algorithm".into(), "ecdsa".into())] && key.children.len() == 2,
        "invalid keybox key"
    );
    let private = key.child(0, "PrivateKey")?;
    let private_der = zeroize::Zeroizing::new(decode_pem_node(private, "EC PRIVATE KEY")?);
    let parsed_secret =
        P256SecretKey::from_sec1_der(&private_der).context("read back keybox private key")?;
    ensure!(&parsed_secret == secret, "keybox private key differs");
    let certificates = key.child(1, "CertificateChain")?;
    ensure!(
        certificates.attributes.is_empty() && certificates.children.len() == chain.len() + 1,
        "invalid keybox certificate count"
    );
    let count = certificates.child(0, "NumberOfCertificates")?;
    ensure!(
        count.text.trim().parse::<usize>()? == chain.len(),
        "keybox certificate count differs"
    );
    let mut stream = Vec::new();
    for (index, expected) in chain.iter().enumerate() {
        let node = certificates.child(index + 1, "Certificate")?;
        let der = decode_pem_node(node, "CERTIFICATE")?;
        Certificate::from_der(&der).context("read back keybox certificate")?;
        ensure!(
            der == expected.der,
            "keybox certificate order or content differs"
        );
        stream.extend_from_slice(&der);
    }
    crate::certificates::select_chain_for_p256(&[stream], &secret.public_key().to_sec1_bytes())?;
    Ok(())
}

struct XmlNode {
    name: String,
    attributes: Vec<(String, String)>,
    text: zeroize::Zeroizing<String>,
    children: Vec<Self>,
}

impl XmlNode {
    fn child(&self, index: usize, expected: &str) -> Result<&Self> {
        let child = self
            .children
            .get(index)
            .context("missing keybox XML element")?;
        ensure!(child.name == expected, "unexpected keybox XML element");
        if matches!(expected, "NumberOfKeyboxes" | "NumberOfCertificates") {
            ensure!(
                child.attributes.is_empty() && child.children.is_empty(),
                "invalid keybox count element"
            );
        }
        Ok(child)
    }
}

fn decode_pem_node(node: &XmlNode, label: &str) -> Result<Vec<u8>> {
    ensure!(
        node.attributes == [("format".into(), "pem".into())] && node.children.is_empty(),
        "invalid PEM element"
    );
    let mut lines = node.text.trim().lines().map(str::trim).collect::<Vec<_>>();
    ensure!(lines.len() >= 3, "incomplete PEM block");
    ensure!(
        lines.remove(0) == format!("-----BEGIN {label}-----")
            && lines.pop() == Some(format!("-----END {label}-----").as_str()),
        "unexpected PEM label"
    );
    let encoded = zeroize::Zeroizing::new(lines.join(""));
    Base64::decode_vec(&encoded).context("decode keybox PEM")
}

fn parse_keybox_tree(xml: &str) -> Result<XmlNode> {
    use quick_xml::{Reader, XmlVersion, events::Event};
    ensure!(
        xml.len() <= 256 * 1024
            && xml
                .chars()
                .all(|ch| matches!(ch, '\t' | '\r' | '\n') || ch >= ' '),
        "invalid XML size or character"
    );
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root = None;
    let mut nodes = 0;
    loop {
        match reader.read_event()? {
            Event::Start(tag) => {
                nodes += 1;
                ensure!(
                    nodes <= 32 && stack.len() < 8 && root.is_none(),
                    "invalid keybox XML structure"
                );
                let mut attributes = Vec::new();
                for attribute in tag.attributes() {
                    let attribute = attribute?;
                    attributes.push((
                        attribute.key.as_ref().to_owned(),
                        attribute
                            .normalized_value(XmlVersion::Implicit1_0)?
                            .into_owned(),
                    ));
                }
                stack.push(XmlNode {
                    name: tag.name().as_ref().to_owned(),
                    attributes,
                    text: zeroize::Zeroizing::new(String::new()),
                    children: Vec::new(),
                });
            }
            Event::End(_) => {
                let node = stack.pop().context("unexpected XML end tag")?;
                ensure!(
                    node.children.is_empty() || node.text.trim().is_empty(),
                    "mixed keybox XML content"
                );
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    ensure!(root.is_none(), "multiple XML documents");
                    root = Some(node);
                }
            }
            Event::Text(text) => {
                let value = text.xml10_content();
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&value);
                } else {
                    ensure!(value.trim().is_empty(), "trailing XML content");
                }
            }
            Event::Decl(_) => ensure!(
                root.is_none() && stack.is_empty(),
                "misplaced XML declaration"
            ),
            Event::Eof => {
                ensure!(stack.is_empty(), "incomplete XML document");
                return root.context("missing keybox XML document");
            }
            _ => anyhow::bail!("unsupported keybox XML construct"),
        }
    }
}

pub fn sort_cert_chain(certs: &[ParsedCertificate]) -> Vec<ParsedCertificate> {
    if certs.len() <= 1 {
        return certs.to_vec();
    }

    let mut leaf = None;
    for candidate in certs {
        let is_issuer = certs.iter().any(|other| {
            other.subject_der != other.issuer_der && other.issuer_der == candidate.subject_der
        });
        if !is_issuer {
            leaf = Some(candidate.clone());
            break;
        }
    }

    let Some(mut current) = leaf else {
        return certs.to_vec();
    };

    let mut ordered = vec![current.clone()];
    while let Some(next) = certs
        .iter()
        .find(|candidate| {
            candidate.subject_der == current.issuer_der
                && candidate.subject_der != current.subject_der
        })
        .cloned()
    {
        if ordered
            .iter()
            .any(|existing| existing.subject_der == next.subject_der)
        {
            break;
        }
        current = next.clone();
        ordered.push(next);
    }

    for cert in certs {
        if !ordered
            .iter()
            .any(|existing| existing.subject_der == cert.subject_der)
        {
            ordered.push(cert.clone());
        }
    }

    ordered
}

fn pem_wrap(label: &str, der: &[u8]) -> String {
    let encoded = Base64::encode_string(der);
    let mut pem = String::new();
    writeln!(pem, "-----BEGIN {label}-----").unwrap();
    for chunk in encoded.as_bytes().chunks(64) {
        writeln!(pem, "{}", String::from_utf8_lossy(chunk)).unwrap();
    }
    write!(pem, "-----END {label}-----").unwrap();
    pem
}

fn indent_block(block: &str, indent: &str) -> String {
    block
        .lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_der_len(bytes: &[u8]) -> Result<(usize, usize)> {
    if bytes.len() < 2 {
        anyhow::bail!("DER sequence is truncated");
    }
    if bytes[0] != 0x30 {
        anyhow::bail!("DER certificate must start with SEQUENCE");
    }

    if bytes[1] & 0x80 == 0 {
        return Ok((2, usize::from(bytes[1])));
    }

    if bytes[1] == 0x80 {
        anyhow::bail!("DER indefinite lengths are not supported");
    }

    let count = usize::from(bytes[1] & 0x7F);
    if count == 0 {
        anyhow::bail!("DER length uses an invalid long-form encoding");
    }
    if count > std::mem::size_of::<usize>() {
        anyhow::bail!("DER length is too large");
    }
    let len_bytes = bytes
        .get(2..2 + count)
        .context("DER length bytes are truncated")?;
    let mut length = 0usize;
    for byte in len_bytes {
        length = (length << 8) | usize::from(*byte);
    }

    Ok((2 + count, length))
}

fn escape_xml_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{
        ParsedCertificate, build_keybox_xml, parse_der_cert_chain, sort_cert_chain,
        validate_keybox_roundtrip,
    };
    use base64ct::{Base64, Encoding};
    use p256::SecretKey;

    const TEST_CERT_DER_BASE64: &str = "MIICqTCCAZGgAwIBAgIJAL3L/xZFRhnmMA0GCSqGSIb3DQEBCwUAMBQxEjAQBgNVBAMTCWR1Y2stdGVzdDAeFw0yNjAzMjkxMDEwMDZaFw0yNjA0MjkxMDEwMDZaMBQxEjAQBgNVBAMTCWR1Y2stdGVzdDCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAKHYqyiPX9viZpkNft3874CaGdTLP2G83ATQO/KRCwJGq6FZmej76CMLxxYplrCwG1tYc6Jr36KcDG0LbspAczsz3SUkXpeIuxaBPBtdpY7CoT05UjW9uJM+te6rWJ4fiR01hfqmjl0t02cS/zPYsRlaIAEg81Rj0lmpK6APS+GARz+OxIuSf9qURnG2O/92wFtIYVEjt2IrgCsL4i6/tYPuyFhv1TuX+QMOgNWyFGSbydexfCaY2GhT4Jvlvl7VPi1uTzPQzAFf636fRpuWuQSWzboich4REDPd/MrFizx/wDc9zVUt+Bv+x2PpRygTmiwoaAvgK7V0Mpa0EWlqhrUCAwEAATANBgkqhkiG9w0BAQsFAAOCAQEASImyG6qe+SQN7FSEkqDPeCJd9BQaTK+uFH/koESrOaRUZb6CDGXzBgbbxXuPZuoqD6EbzGX/Ca3/8sS4p7klt07uEot1l86/iyUY1DnAl/eTmEF3aoahOpsvSQEoEMb+qAOld6Oi6uVxBQYJ13LRJDxJ2lutRKlmk1Dyp03OgDK4iH5Ja5KkVm+VHMBgKNtnuF1A3cHISF+X8TzKDm08d85VitHjdNMrVcC+MPDAw8HDjVJzu7wMHh/xmqpunjjCJ2yujM3HLqp+uRRSJ+EyBoVheYLyxXwHAg/xHsT8vhqQBOFS39OAhqX/mDUfWcCUYXAwKTy/zeylTkeyZyUHSQ==";

    fn sample_cert_der() -> Vec<u8> {
        Base64::decode_vec(TEST_CERT_DER_BASE64).unwrap()
    }

    #[test]
    fn roundtrip_checks_private_key_certificates_counts_and_device_id() {
        use p256::pkcs8::DecodePrivateKey as _;
        let pair = rcgen::KeyPair::generate().unwrap();
        let secret = SecretKey::from_pkcs8_der(pair.serialized_der()).unwrap();
        let mut parameters = rcgen::CertificateParams::default();
        parameters.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
        parameters.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
        let cert = parameters.self_signed(&pair).unwrap();
        let chain = parse_der_cert_chain(cert.der()).unwrap();
        let xml = build_keybox_xml(&secret, &chain, "duck<&\"").unwrap();
        validate_keybox_roundtrip(&xml, &secret, &chain, "duck<&\"").unwrap();
        assert!(
            validate_keybox_roundtrip(
                &xml.replace(">1</NumberOfCertificates>", ">2</NumberOfCertificates>"),
                &secret,
                &chain,
                "duck<&\""
            )
            .is_err()
        );
        assert!(validate_keybox_roundtrip(&xml, &secret, &chain, "other").is_err());
        assert!(
            validate_keybox_roundtrip(
                &xml,
                &SecretKey::from_slice(&[0x11; 32]).unwrap(),
                &chain,
                "duck<&\""
            )
            .is_err()
        );
    }

    #[test]
    fn sort_cert_chain_orders_leaf_first() {
        let root = ParsedCertificate {
            der: vec![1],
            subject_der: vec![0x01],
            issuer_der: vec![0x01],
            subject_summary: "root".into(),
            issuer_summary: "root".into(),
        };
        let leaf = ParsedCertificate {
            der: vec![2],
            subject_der: vec![0x02],
            issuer_der: vec![0x01],
            subject_summary: "leaf".into(),
            issuer_summary: "root".into(),
        };

        let ordered = sort_cert_chain(&[root.clone(), leaf.clone()]);
        assert_eq!(ordered[0].subject_summary, "leaf");
        assert_eq!(ordered[1].subject_summary, "root");
    }

    #[test]
    fn keybox_xml_contains_expected_nodes() {
        let secret_key = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let cert = ParsedCertificate {
            der: vec![0x30, 0x00],
            subject_der: vec![0x01],
            issuer_der: vec![0x01],
            subject_summary: "root".into(),
            issuer_summary: "root".into(),
        };
        let xml = build_keybox_xml(&secret_key, &[cert], "duck-123").unwrap();
        assert!(xml.contains("<AndroidAttestation>"));
        assert!(xml.contains("DeviceID=\"duck-123\""));
        assert!(xml.contains("<CertificateChain>"));
    }

    #[test]
    fn keybox_xml_escapes_device_id_attribute() {
        let secret_key = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let cert = ParsedCertificate {
            der: vec![0x30, 0x00],
            subject_der: vec![0x01],
            issuer_der: vec![0x01],
            subject_summary: "root".into(),
            issuer_summary: "root".into(),
        };
        let xml = build_keybox_xml(&secret_key, &[cert], "duck\"<&>'").unwrap();

        assert!(xml.contains("DeviceID=\"duck&quot;&lt;&amp;&gt;&apos;\""));
    }

    #[test]
    fn parse_der_cert_chain_parses_single_certificate() {
        let parsed = parse_der_cert_chain(&sample_cert_der()).unwrap();
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn parse_der_cert_chain_rejects_truncated_certificate() {
        let mut der = sample_cert_der();
        der.pop();

        let error = parse_der_cert_chain(&der).unwrap_err();
        assert!(error.to_string().contains("truncated"));
    }

    #[test]
    fn parse_der_cert_chain_rejects_trailing_garbage() {
        let mut der = sample_cert_der();
        der.extend_from_slice(&[0x00, 0x01]);

        let error = parse_der_cert_chain(&der).unwrap_err();
        assert!(error.to_string().contains("trailing non-DER data"));
    }
}
