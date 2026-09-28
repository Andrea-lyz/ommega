//! Synthetic SOTER parcels for decoder tests and the read-only self-test.
//!
//! These values are constructed from the documented wire layout. They contain
//! no real device identifier, exported key, captured signature, or user data.

use anyhow::{anyhow, bail, Context, Result};
use rsbinder::Parcel;
use serde_json::{json, Value};

use super::hal::{read_soter_data, read_status, Backend, SoterData};

const SYNTHETIC_ID: &str = "0123456789abcdef0123456789abcdef";
const SYNTHETIC_UID: &str = "12345";
const DUMMY_PUBLIC_KEY: &str =
    "-----BEGIN PUBLIC KEY-----\nSYNTHETIC-TEST-ONLY\n-----END PUBLIC KEY-----";

/// Encode a Trustonic AIDL `SoterData` reply with the same field widths and
/// padding as the vendor parcelable, without using a real HAL capture.
fn trustonic_reply(data: &[u8]) -> String {
    let len = data.len();
    let padded = (len + 3) & !3;
    let mut bytes = Vec::with_capacity(24 + padded);
    for value in [0i32, 1, (16 + padded) as i32, 0, len as i32] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(data);
    bytes.resize(20 + padded, 0);
    bytes.extend_from_slice(&(len as i32).to_le_bytes());
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn synthetic_device_reply() -> String {
    let mut data = SYNTHETIC_ID.as_bytes().to_vec();
    data.push(0);
    trustonic_reply(&data)
}

fn synthetic_attk_reply() -> String {
    trustonic_reply(DUMMY_PUBLIC_KEY.as_bytes())
}

fn synthetic_ask_reply() -> String {
    let doc = json!({
        "pub_key": DUMMY_PUBLIC_KEY,
        "cpu_id": SYNTHETIC_ID,
        "counter": 7,
        "uid": SYNTHETIC_UID,
        "rsa_pss_saltlen": 32,
    })
    .to_string();
    let mut data = Vec::new();
    data.extend_from_slice(&(doc.len() as i32).to_le_bytes());
    data.extend_from_slice(doc.as_bytes());
    data.extend_from_slice(&[0x5a; 256]); // shape only; not a real signature
    trustonic_reply(&data)
}

/// Decode a `SoterData` reply represented as hex.
pub fn decode_soter_data_reply(hex: &str) -> Result<SoterData> {
    decode_soter_data_reply_as(hex, Backend::Trustonic)
}

pub fn decode_soter_data_reply_as(hex: &str, backend: Backend) -> Result<SoterData> {
    let mut parcel = Parcel::from_vec(hex_decode(hex)?);
    read_status(&mut parcel)?;
    read_soter_data(&mut parcel, backend)
}

fn hex_decode(hex: &str) -> Result<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        bail!("hex string has an odd length ({})", hex.len());
    }
    let src = hex.as_bytes();
    let mut out = Vec::with_capacity(src.len() / 2);
    let mut i = 0;
    while i < src.len() {
        out.push((hex_nibble(src[i])? << 4) | hex_nibble(src[i + 1])?);
        i += 2;
    }
    Ok(out)
}

fn hex_nibble(c: u8) -> Result<u8> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        other => bail!("not a hex digit: {:?}", other as char),
    }
}

/// Exercise both vendor reply layouts without issuing a HAL transaction.
pub fn selftest() -> Value {
    let checks = vec![
        check_device_id(),
        check_attk(),
        check_ask(),
        check_qti_framing(),
    ];
    let ok = checks.iter().all(|c| c.get("ok") == Some(&json!(true)));
    json!({ "ok": ok, "checks": checks })
}

fn check(name: &str, run: impl FnOnce() -> Result<Value>) -> Value {
    match run() {
        Ok(detail) => json!({ "name": name, "ok": true, "detail": detail }),
        Err(e) => json!({ "name": name, "ok": false, "error": format!("{e:#}") }),
    }
}

fn check_device_id() -> Value {
    check("synthetic getDeviceId reply", || {
        let data = decode_soter_data_reply(&synthetic_device_reply())?;
        let text = data.text().ok_or_else(|| anyhow!("payload is not UTF-8"))?;
        if data.error_code != 0 || text != SYNTHETIC_ID || data.length != 33 {
            bail!("synthetic device reply did not round-trip");
        }
        Ok(json!({ "error_code": data.error_code, "length": data.length }))
    })
}

fn check_attk() -> Value {
    check("synthetic exportAttkPublicKey reply", || {
        let data = decode_soter_data_reply(&synthetic_attk_reply())?;
        if data.error_code != 0 || data.text() != Some(DUMMY_PUBLIC_KEY) {
            bail!("synthetic public-key reply did not round-trip");
        }
        Ok(json!({ "error_code": data.error_code, "length": data.length }))
    })
}

fn check_ask() -> Value {
    check("synthetic exportAskPublicKey reply", || {
        let data = decode_soter_data_reply(&synthetic_ask_reply())?;
        let (doc_bytes, signature) = split_ask_payload(&data.data)?;
        let doc: Value = serde_json::from_slice(doc_bytes).context("parse synthetic ASK JSON")?;
        if data.error_code != 0
            || doc["cpu_id"] != SYNTHETIC_ID
            || doc["uid"] != SYNTHETIC_UID
            || signature != [0x5a; 256]
        {
            bail!("synthetic ASK reply did not round-trip");
        }
        Ok(json!({
            "error_code": data.error_code,
            "length": data.length,
            "json_bytes": doc_bytes.len(),
            "signature_bytes": signature.len(),
        }))
    })
}

fn check_qti_framing() -> Value {
    check("synthetic qti SoterData framing", || {
        let mut bytes = Vec::new();
        for field in [0i32, 0, 1, 48, 33] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        bytes.extend_from_slice(SYNTHETIC_ID.as_bytes());
        bytes.extend_from_slice(&[0u8; 4]);
        bytes.extend_from_slice(&33i32.to_le_bytes());
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let data = decode_soter_data_reply_as(&hex, Backend::Qti)?;
        if data.error_code != 0 || data.length != 33 || data.text() != Some(SYNTHETIC_ID) {
            bail!("synthetic QTI reply did not round-trip");
        }
        if decode_soter_data_reply(&hex).is_ok() {
            bail!("Trustonic framing accepted a QTI reply");
        }
        Ok(json!({ "error_code": data.error_code, "length": data.length }))
    })
}

fn json_length(data: &[u8]) -> Result<usize> {
    if data.len() < 4 {
        bail!("payload is too short to hold a JSON length prefix");
    }
    let len = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if len <= 0 || 4 + len as usize > data.len() {
        bail!("inner JSON length {len} does not fit the payload");
    }
    Ok(len as usize)
}

pub fn split_ask_payload(data: &[u8]) -> Result<(&[u8], &[u8])> {
    let json_len = json_length(data)?;
    Ok((&data[4..4 + json_len], &data[4 + json_len..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_replies_cover_both_vendor_layouts() {
        let report = selftest();
        assert_eq!(report["ok"], json!(true), "selftest report: {report}");
        let data = decode_soter_data_reply(&synthetic_ask_reply()).unwrap();
        let (doc, signature) = split_ask_payload(&data.data).unwrap();
        assert!(!doc.is_empty());
        assert_eq!(signature.len(), 256);
    }
}
