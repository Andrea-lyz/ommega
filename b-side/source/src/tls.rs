//! TLS server authentication policy for the relay daemon.
//!
//! A relay with a public CA certificate is verified against the bundled Web PKI
//! roots. A self-signed relay is authenticated by pinning the SHA-256 of its
//! certificate's SubjectPublicKeyInfo; the handshake signature is still checked
//! against that key, so a peer must hold the pinned private key. Skipping
//! verification is only possible through an explicit
//! `OMMEGA_RELAY_TLS_INSECURE` opt-in.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use base64::Engine as _;
use reqwest::blocking::ClientBuilder;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};
use x509_cert::der::{Decode, Encode};

pub type SpkiPin = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsPolicy {
    /// Standard certificate chain and hostname verification.
    Verify,
    /// Accept only a server key whose SPKI SHA-256 is listed.
    Pinned(Vec<SpkiPin>),
    /// Explicit opt-in: accept any server certificate.
    Insecure,
}

impl TlsPolicy {
    /// A configured pin always wins over `insecure`; an unparsable pin is an
    /// error rather than a silent downgrade.
    pub fn from_settings(pins: &str, insecure: bool) -> Result<Self> {
        let pins = parse_pins(pins)?;
        Ok(if !pins.is_empty() {
            Self::Pinned(pins)
        } else if insecure {
            Self::Insecure
        } else {
            Self::Verify
        })
    }

    pub fn apply(&self, builder: ClientBuilder) -> Result<ClientBuilder> {
        Ok(match self {
            Self::Verify => builder,
            Self::Insecure => builder.danger_accept_invalid_certs(true),
            Self::Pinned(pins) => builder.use_preconfigured_tls(pinned_client_config(pins)?),
        })
    }
}

/// Parses a comma-separated pin list. Each entry is the SHA-256 of the DER
/// SubjectPublicKeyInfo, as 64 hex digits (colons allowed) or base64,
/// optionally prefixed with `sha256/` or `sha256//`.
pub fn parse_pins(raw: &str) -> Result<Vec<SpkiPin>> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(parse_pin)
        .collect()
}

fn parse_pin(entry: &str) -> Result<SpkiPin> {
    let value = entry
        .strip_prefix("sha256//")
        .or_else(|| entry.strip_prefix("sha256/"))
        .unwrap_or(entry);
    let hex_digits: String = value.chars().filter(|c| *c != ':').collect();
    let bytes = if hex_digits.len() == 64 && hex_digits.chars().all(|c| c.is_ascii_hexdigit()) {
        hex::decode(&hex_digits).map_err(|e| anyhow!("invalid TLS pin {entry:?}: {e}"))?
    } else {
        base64::engine::general_purpose::STANDARD
            .decode(value)
            .map_err(|_| anyhow!("invalid TLS pin {entry:?}: expected SHA-256 hex or base64"))?
    };
    bytes
        .try_into()
        .map_err(|_| anyhow!("invalid TLS pin {entry:?}: expected a 32-byte SHA-256"))
}

/// SHA-256 of the DER SubjectPublicKeyInfo of `cert`.
pub fn spki_sha256(cert: &[u8]) -> Result<SpkiPin> {
    let cert = x509_cert::Certificate::from_der(cert)
        .map_err(|e| anyhow!("malformed server certificate: {e}"))?;
    let spki = cert
        .tbs_certificate()
        .subject_public_key_info()
        .to_der()
        .map_err(|e| anyhow!("cannot encode server SPKI: {e}"))?;
    let digest = ring::digest::digest(&ring::digest::SHA256, &spki);
    digest
        .as_ref()
        .try_into()
        .map_err(|_| anyhow!("unexpected SHA-256 length"))
}

fn pinned_client_config(pins: &[SpkiPin]) -> Result<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(SpkiPinVerifier {
        pins: pins.to_vec(),
        algorithms: provider.signature_verification_algorithms,
    });
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| anyhow!("TLS protocol setup failed: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth())
}

#[derive(Debug)]
struct SpkiPinVerifier {
    pins: Vec<SpkiPin>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for SpkiPinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let hash = spki_sha256(end_entity)
            .map_err(|_| rustls::Error::InvalidCertificate(CertificateError::BadEncoding))?;
        if self.pins.contains(&hash) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    fn self_signed() -> rcgen::CertifiedKey<rcgen::KeyPair> {
        rcgen::generate_simple_self_signed(vec!["relay.test".to_string()]).unwrap()
    }

    /// Serves one TLS connection and reports whether a request header
    /// reached the server.
    fn serve_once(key: &rcgen::CertifiedKey<rcgen::KeyPair>) -> (u16, mpsc::Receiver<bool>) {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![key.cert.der().clone()],
                rustls::pki_types::PrivateKeyDer::Pkcs8(key.signing_key.serialize_der().into()),
            )
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let conn = rustls::ServerConnection::new(Arc::new(config)).unwrap();
            let mut tls = rustls::StreamOwned::new(conn, tcp);
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match tls.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let got_token = String::from_utf8_lossy(&request)
                .to_ascii_lowercase()
                .contains("x-relay-token");
            let _ = tls
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
            let _ = tls.flush();
            let _ = tx.send(got_token);
        });
        (port, rx)
    }

    fn send(policy: &TlsPolicy, port: u16) -> reqwest::Result<reqwest::blocking::Response> {
        let client = policy
            .apply(reqwest::blocking::Client::builder().timeout(Duration::from_secs(5)))
            .unwrap()
            .build()
            .unwrap();
        client
            .get(format!("https://127.0.0.1:{port}/"))
            .header("X-Relay-Token", "secret")
            .send()
    }

    #[test]
    fn pins_accept_hex_and_base64_forms() {
        let pin = [0xabu8; 32];
        let hex = hex::encode(pin);
        let b64 = base64::engine::general_purpose::STANDARD.encode(pin);
        let colons = hex
            .as_bytes()
            .chunks(2)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join(":");
        for raw in [
            hex.clone(),
            hex.to_uppercase(),
            colons,
            b64.clone(),
            format!("sha256/{b64}"),
            format!("sha256//{b64}"),
        ] {
            assert_eq!(parse_pins(&raw).unwrap(), vec![pin], "{raw}");
        }
        assert_eq!(
            parse_pins(&format!(" {hex} , {b64} ")).unwrap(),
            vec![pin, pin]
        );
        assert!(parse_pins("").unwrap().is_empty());
        assert!(parse_pins("abcd").is_err());
        assert!(parse_pins(&hex[..62]).is_err());
    }

    #[test]
    fn policy_defaults_to_verification_and_never_downgrades_a_bad_pin() {
        assert_eq!(
            TlsPolicy::from_settings("", false).unwrap(),
            TlsPolicy::Verify
        );
        assert_eq!(
            TlsPolicy::from_settings("", true).unwrap(),
            TlsPolicy::Insecure
        );
        let pin = hex::encode([1u8; 32]);
        assert_eq!(
            TlsPolicy::from_settings(&pin, true).unwrap(),
            TlsPolicy::Pinned(vec![[1u8; 32]])
        );
        assert!(TlsPolicy::from_settings("not-a-pin", true).is_err());
    }

    #[test]
    fn spki_hash_matches_the_certificate_public_key() {
        let key = self_signed();
        let expected = ring::digest::digest(
            &ring::digest::SHA256,
            &rcgen::PublicKeyData::subject_public_key_info(&key.signing_key),
        );
        assert_eq!(
            spki_sha256(key.cert.der()).unwrap().as_slice(),
            expected.as_ref()
        );
    }

    #[test]
    fn pinned_server_is_accepted() {
        let key = self_signed();
        let pin = spki_sha256(key.cert.der()).unwrap();
        let (port, seen) = serve_once(&key);
        let response = send(&TlsPolicy::Pinned(vec![pin]), port).unwrap();
        assert!(response.status().is_success());
        assert!(seen.recv_timeout(Duration::from_secs(5)).unwrap());
    }

    #[test]
    fn other_self_signed_server_is_rejected_before_the_token_is_sent() {
        let pinned = self_signed();
        let attacker = self_signed();
        let pin = spki_sha256(pinned.cert.der()).unwrap();
        for policy in [TlsPolicy::Pinned(vec![pin]), TlsPolicy::Verify] {
            let (port, seen) = serve_once(&attacker);
            assert!(send(&policy, port).is_err(), "{policy:?}");
            assert!(
                !seen.recv_timeout(Duration::from_secs(5)).unwrap_or(false),
                "{policy:?}"
            );
        }
    }
}
