use anyhow::{Context, Result, anyhow};
use base64ct::{Base64Url, Encoding};
use ciborium::value::Value;
use reqwest::{Client, StatusCode};
use serde::Serialize;

use crate::{
    cbor::{as_array, as_bytes, as_i128, as_map, decode, encode, int, map_get},
    cose::{RPC_CURVE_25519, RPC_CURVE_P256},
    error::RkpError,
};

mod transport;
pub use transport::{fetch_challenge, rkp_client, validate_endpoint};
use transport::{read_body, rpc_url};

pub const RKP_SERVER_URL: &str = "https://remoteprovisioning.googleapis.com/v1";

#[derive(Debug, Clone, Serialize)]
pub struct EekResponse {
    pub challenge_hex: String,
    pub challenge: Vec<u8>,
    pub eek_public_hex: String,
    pub eek_public: Vec<u8>,
    #[serde(skip)]
    pub eek_id: Vec<u8>,
    pub eek_curve: i128,
    /// Complete, curve-indexed chains for the legacy HAL, which verifies the EEK root.
    #[serde(skip)]
    pub eek_chains: Vec<EekChain>,
}

#[derive(Debug, Clone)]
pub struct EekChain {
    pub curve: i128,
    pub encoded: Vec<u8>,
}

#[derive(Debug, Clone)]
struct SelectedEek {
    public_key: Vec<u8>,
    key_id: Vec<u8>,
    curve: i128,
}

const MAX_RKP_CHALLENGE_LEN: usize = 64;
const COSE_KEY_TYPE_OKP: i128 = 1;
const COSE_KEY_TYPE_EC2: i128 = 2;
const COSE_KEY_ID_LABEL: i128 = 2;
const COSE_ALGORITHM_ECDH_ES_HKDF_256: i128 = -25;
const COSE_CURVE_P256: i128 = 1;
const COSE_CURVE_X25519: i128 = 4;
const P256_COORD_LEN: usize = 32;
const X25519_PUBLIC_KEY_LEN: usize = 32;

pub async fn fetch_eek(
    client: &Client,
    fingerprint: &str,
    server_url: &str,
) -> Result<EekResponse> {
    let url = rpc_url(server_url, "fetchEekChain")?;
    let payload = encode(&Value::Map(vec![
        (
            Value::Text("fingerprint".into()),
            Value::Text(fingerprint.into()),
        ),
        (Value::Text("id".into()), int(42)),
    ]))?;

    let response = client
        .post(url)
        .header("Content-Type", "application/cbor")
        .header("Accept", "application/cbor")
        .body(payload)
        .send()
        .await
        .context("send fetchEekChain request")?;

    let status = response.status();
    let body = read_body(response, false).await?;
    if !status.is_success() {
        return Err(map_http_error(status, &body).into());
    }

    parse_fetch_eek_response(&body)
}

pub async fn submit_csr(
    client: &Client,
    csr_bytes: &[u8],
    challenge: &[u8],
    server_url: &str,
) -> Result<Vec<Vec<u8>>> {
    let mut url = rpc_url(server_url, "signCertificates")?;
    url.query_pairs_mut()
        .append_pair("challenge", &Base64Url::encode_string(challenge));

    let response = client
        .post(url)
        .header("Content-Type", "application/cbor")
        .header("Accept", "application/cbor")
        .body(csr_bytes.to_vec())
        .send()
        .await
        .map_err(|_| RkpError::SubmissionUnknown)?;

    let status = response.status();
    if !status.is_success() {
        return Err(map_http_error(status, &[]).into());
    }

    let body = read_body(response, true)
        .await
        .context(RkpError::SubmissionUnknown)?;

    parse_sign_certificates_response(&body).context(RkpError::SubmissionUnknown)
}

fn map_http_error(status: StatusCode, _body: &[u8]) -> RkpError {
    // the server may reflect submitted material, so only expose its status
    let message = format!("HTTP {}", status.as_u16());
    if status.as_u16() == 444 {
        return RkpError::DeviceNotRegistered(message);
    }
    if status.is_client_error() {
        return RkpError::RkpClient(format!("HTTP {}: {message}", status.as_u16()));
    }
    if status.is_server_error() {
        return RkpError::RkpServer(format!("HTTP {}: {message}", status.as_u16()));
    }

    RkpError::RkpServer(format!("HTTP {}: {message}", status.as_u16()))
}

fn select_eek_material(chains_value: &Value) -> Result<SelectedEek> {
    let chains = as_array(chains_value, "EEK chains")?;
    let mut last_error = None;

    for preferred in [RPC_CURVE_25519, RPC_CURVE_P256] {
        for chain_entry in chains {
            let chain_entry_items = as_array(chain_entry, "EEK chain entry")?;
            let curve = as_i128(
                chain_entry_items
                    .first()
                    .ok_or_else(|| anyhow!("EEK chain entry missing curve"))?,
                "curve",
            )?;
            if curve != preferred {
                continue;
            }

            match extract_eek_material(chain_entry_items, curve) {
                Ok(selected) => return Ok(selected),
                Err(error) => last_error = Some(error),
            }
        }
    }

    if let Some(error) = last_error {
        return Err(error);
    }

    Err(anyhow!("RKP server did not return a usable EEK public key"))
}

fn extract_eek_material(chain_entry_items: &[Value], curve: i128) -> Result<SelectedEek> {
    let chain = as_array(
        chain_entry_items
            .get(1)
            .ok_or_else(|| anyhow!("EEK chain entry missing certificates"))?,
        "EEK certificate chain",
    )?;
    let last_cert = chain.last().ok_or_else(|| anyhow!("EEK chain is empty"))?;
    let cert_items = as_array(last_cert, "EEK certificate")?;
    let payload_bytes = as_bytes(
        cert_items
            .get(2)
            .ok_or_else(|| anyhow!("EEK certificate missing payload"))?,
        "EEK payload",
    )?;
    let payload = decode(payload_bytes)?;
    let payload_entries = as_map(&payload, "EEK payload")?;
    parse_eek_cose_key(payload_entries, curve)
}

fn parse_eek_cose_key(entries: &[(Value, Value)], eek_curve: i128) -> Result<SelectedEek> {
    let key_type = required_cose_int(entries, 1, "EEK key type")?;
    let algorithm = required_cose_int(entries, 3, "EEK algorithm")?;
    let curve_id = required_cose_int(entries, -1, "EEK curve id")?;
    let key_id = optional_cose_bytes(entries, COSE_KEY_ID_LABEL, "EEK key identifier")?
        .unwrap_or_default()
        .to_vec();
    if algorithm != COSE_ALGORITHM_ECDH_ES_HKDF_256 {
        return Err(anyhow!(
            "EEK algorithm must be {COSE_ALGORITHM_ECDH_ES_HKDF_256}, got {algorithm}"
        ));
    }

    match eek_curve {
        RPC_CURVE_25519 => {
            if key_type != COSE_KEY_TYPE_OKP {
                return Err(anyhow!("X25519 EEK key type must be {COSE_KEY_TYPE_OKP}"));
            }
            if curve_id != COSE_CURVE_X25519 {
                return Err(anyhow!("X25519 EEK curve id must be {COSE_CURVE_X25519}"));
            }

            let public_key = required_cose_bytes(entries, -2, "X25519 EEK public key")?.to_vec();
            if public_key.len() != X25519_PUBLIC_KEY_LEN {
                return Err(anyhow!(
                    "X25519 EEK public key must be {X25519_PUBLIC_KEY_LEN} bytes"
                ));
            }

            Ok(SelectedEek {
                public_key,
                key_id,
                curve: eek_curve,
            })
        }
        RPC_CURVE_P256 => {
            if key_type != COSE_KEY_TYPE_EC2 {
                return Err(anyhow!("P-256 EEK key type must be {COSE_KEY_TYPE_EC2}"));
            }
            if curve_id != COSE_CURVE_P256 {
                return Err(anyhow!("P-256 EEK curve id must be {COSE_CURVE_P256}"));
            }

            let x = required_cose_bytes(entries, -2, "P-256 EEK x coordinate")?;
            let y = required_cose_bytes(entries, -3, "P-256 EEK y coordinate")?;
            if x.len() != P256_COORD_LEN || y.len() != P256_COORD_LEN {
                return Err(anyhow!(
                    "P-256 EEK coordinates must each be {P256_COORD_LEN} bytes"
                ));
            }

            let mut public_key = Vec::with_capacity(P256_COORD_LEN * 2);
            public_key.extend_from_slice(x);
            public_key.extend_from_slice(y);

            Ok(SelectedEek {
                public_key,
                key_id,
                curve: eek_curve,
            })
        }
        other => Err(anyhow!("unsupported EEK curve {other} in response")),
    }
}

fn required_cose_int(entries: &[(Value, Value)], key: i128, label: &str) -> Result<i128> {
    as_i128(
        map_get(entries, key).ok_or_else(|| anyhow!("{label} is missing"))?,
        label,
    )
}

fn required_cose_bytes<'a>(
    entries: &'a [(Value, Value)],
    key: i128,
    label: &str,
) -> Result<&'a [u8]> {
    as_bytes(
        map_get(entries, key).ok_or_else(|| anyhow!("{label} is missing"))?,
        label,
    )
}

fn optional_cose_bytes<'a>(
    entries: &'a [(Value, Value)],
    key: i128,
    label: &str,
) -> Result<Option<&'a [u8]>> {
    map_get(entries, key)
        .map(|value| as_bytes(value, label))
        .transpose()
}

fn parse_fetch_eek_response(body: &[u8]) -> Result<EekResponse> {
    let value = invalid_rkp_response(decode(body))?;
    let items = invalid_rkp_response(as_array(&value, "fetchEekChain response"))?;
    let eek_chains = items.first().ok_or_else(|| {
        RkpError::InvalidRkpResponse("fetchEekChain response is missing EEK chains".into())
    })?;
    let challenge = invalid_rkp_response(as_bytes(
        items.get(1).ok_or_else(|| {
            RkpError::InvalidRkpResponse("fetchEekChain response is missing challenge".into())
        })?,
        "challenge",
    ))?
    .to_vec();
    validate_challenge(&challenge)?;

    let selected = invalid_rkp_response(select_eek_material(eek_chains))?;
    let mut legacy_chains = Vec::new();
    for entry in invalid_rkp_response(as_array(eek_chains, "EEK chains"))? {
        // Additional unusable chains must not invalidate an already selected EEK.
        let Ok(pair) = as_array(entry, "EEK chain entry") else {
            continue;
        };
        let Some(curve) = pair.first().and_then(|v| as_i128(v, "curve").ok()) else {
            continue;
        };
        if [RPC_CURVE_25519, RPC_CURVE_P256].contains(&curve)
            && extract_eek_material(pair, curve).is_ok()
            && !legacy_chains
                .iter()
                .any(|chain: &EekChain| chain.curve == curve)
        {
            legacy_chains.push(EekChain {
                curve,
                encoded: encode(&pair[1])?,
            });
        }
    }
    Ok(EekResponse {
        challenge_hex: hex::encode(&challenge),
        challenge,
        eek_public_hex: hex::encode(&selected.public_key),
        eek_public: selected.public_key,
        eek_id: selected.key_id,
        eek_curve: selected.curve,
        eek_chains: legacy_chains,
    })
}

fn parse_challenge_response(body: &[u8]) -> Result<Vec<u8>> {
    let value = invalid_rkp_response(decode(body))?;
    let items = invalid_rkp_response(as_array(&value, "challenge response"))?;
    let challenge = invalid_rkp_response(as_bytes(
        items
            .get(1)
            .ok_or_else(|| RkpError::InvalidRkpResponse("missing challenge".into()))?,
        "challenge",
    ))?;
    validate_challenge(challenge)?;
    Ok(challenge.to_vec())
}

fn parse_sign_certificates_response(body: &[u8]) -> Result<Vec<Vec<u8>>> {
    let value = decode(body).map_err(|error| {
        let reason = error.to_string();
        // only fixed diagnostics are safe because CBOR map errors can include response keys
        let safe_reason = match reason.as_str() {
            "truncated CBOR"
            | "truncated CBOR string"
            | "trailing data after CBOR item"
            | "CBOR nesting exceeds 16 levels"
            | "CBOR container exceeds 64 entries" => reason,
            _ => "malformed CBOR".into(),
        };
        RkpError::InvalidRkpResponse(safe_reason)
    })?;
    let top = invalid_rkp_response(as_array(&value, "signCertificates response"))?;
    if top.is_empty() {
        return Err(
            RkpError::InvalidRkpResponse("signCertificates response is empty".into()).into(),
        );
    }

    let inner = match &top[0] {
        Value::Array(inner) => inner.as_slice(),
        _ => top,
    };

    if inner.len() < 2 {
        return Err(RkpError::InvalidRkpResponse(
            "signCertificates response is missing certificate chain data".into(),
        )
        .into());
    }

    let shared = invalid_rkp_response(as_bytes(&inner[0], "shared certificate chain"))?.to_vec();
    let unique_list = invalid_rkp_response(as_array(&inner[1], "unique certificate chains"))?;

    let mut chains = Vec::new();
    for item in unique_list {
        let unique = invalid_rkp_response(as_bytes(item, "unique certificate chain"))?;
        let mut chain = shared.clone();
        chain.extend_from_slice(unique);
        chains.push(chain);
    }

    if chains.is_empty() {
        return Err(RkpError::InvalidRkpResponse(
            "RKP server returned no certificate chains".into(),
        )
        .into());
    }

    Ok(chains)
}

fn validate_challenge(challenge: &[u8]) -> Result<()> {
    if challenge.len() <= MAX_RKP_CHALLENGE_LEN {
        return Ok(());
    }

    Err(RkpError::InvalidRkpResponse(format!(
        "fetchEekChain challenge must be at most {MAX_RKP_CHALLENGE_LEN} bytes, got {}",
        challenge.len()
    ))
    .into())
}

fn invalid_rkp_response<T>(result: Result<T>) -> Result<T> {
    result.map_err(|error| RkpError::InvalidRkpResponse(error.to_string()).into())
}
