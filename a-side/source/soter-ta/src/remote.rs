//! Optional SOTER relay behind the software TA's native HAL service.
//!
//! This configuration is deliberately separate from the KeyMint/Integrity
//! relay. A disabled relay never touches the network or the local SOTER ledger.
//! An enabled relay that fails must not fall back to the local ledger: that
//! would mix two different ASK/AuthKey identities in the same application slot.

use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};

use crate::dispatch::{self, Outcome, Request};

pub const CONFIG_PATH: &str = "/data/adb/ommega/soterta/remote.conf";
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
static CLIENT: OnceLock<Mutex<Option<(bool, reqwest::blocking::Client)>>> = OnceLock::new();

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub enabled: bool,
    pub url: String,
    pub token: String,
    pub device_id: String,
    pub tls_insecure: bool,
    pub uid_map: String,
}

impl Config {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut config = Self::default();
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err("invalid SOTER remote config line".to_string());
            };
            let value = value.trim();
            match key.trim() {
                "enabled" => config.enabled = parse_bool(value)?,
                "url" => config.url = value.to_string(),
                "token" => config.token = value.to_string(),
                "device_id" => config.device_id = value.to_string(),
                "tls_insecure" => config.tls_insecure = parse_bool(value)?,
                "uid_map" => config.uid_map = value.to_string(),
                _ => return Err("unknown SOTER remote config key".to_string()),
            }
        }
        if config.enabled {
            let url = reqwest::Url::parse(&config.url).map_err(|_| "invalid SOTER URL")?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err("SOTER URL must be HTTP(S) without embedded credentials".to_string());
            }
            if config.token.is_empty() || config.device_id.is_empty() {
                return Err("SOTER token and device_id are required when enabled".to_string());
            }
            if [
                config.token.as_str(),
                config.device_id.as_str(),
                config.uid_map.as_str(),
            ]
            .iter()
            .any(|value| value.contains(['\r', '\n']))
            {
                return Err("SOTER remote config contains a newline".to_string());
            }
        }
        Ok(config)
    }

    pub fn load() -> Result<Self, String> {
        match std::fs::read_to_string(CONFIG_PATH) {
            Ok(raw) => Self::parse(&raw),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("cannot read SOTER remote config: {error}")),
        }
    }
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err("invalid SOTER remote boolean".to_string()),
    }
}

fn mapped_uid(uid: u32, mapping: &str) -> u32 {
    mapping
        .split([',', ';', ' '])
        .filter_map(|part| part.split_once('=').or_else(|| part.split_once(':')))
        .find_map(|(from, to)| {
            (from.trim().parse::<u32>().ok()? == uid)
                .then(|| to.trim().parse::<u32>().ok())
                .flatten()
        })
        .unwrap_or(uid)
}

fn request_body(config: &Config, tx: u32, request: &Request) -> Result<Value, String> {
    use dispatch::*;
    let op = match tx {
        TX_EXPORT_ASK => "export_ask_public_key",
        TX_EXPORT_AUTH => "export_auth_key_public_key",
        TX_FINISH_SIGN => "finish_sign",
        TX_GENERATE_ASK => "generate_ask_key_pair",
        TX_GENERATE_AUTH => "generate_auth_key_pair",
        TX_GET_DEVICE_ID => "get_device_id",
        TX_HAS_ASK => "has_ask_already",
        TX_HAS_AUTH => "has_auth_key",
        TX_INIT_SIGN => "init_sign",
        TX_REMOVE_ALL_UID_KEY => "remove_all_uid_key",
        TX_REMOVE_AUTH => "remove_auth_key",
        // The QTI B-side HAL does not expose a verified ATTK trio. Never
        // silently answer these with the local TA while using a remote ASK.
        TX_EXPORT_ATTK | TX_GENERATE_ATTK | TX_VERIFY_ATTK => {
            return Err("remote ATTK operation is unsupported".to_string())
        }
        _ => return Err("unknown SOTER transaction".to_string()),
    };
    let mut body = json!({"op": op, "device_id": config.device_id});
    match request {
        Request::None => {}
        Request::Uid(uid) => body["uid"] = json!(mapped_uid(*uid, &config.uid_map)),
        Request::UidKey { uid, kname } => {
            body["uid"] = json!(mapped_uid(*uid, &config.uid_map));
            body["alias"] = json!(kname);
        }
        Request::InitSign {
            uid,
            kname,
            challenge,
        } => {
            body["uid"] = json!(mapped_uid(*uid, &config.uid_map));
            body["alias"] = json!(kname);
            body["challenge"] = json!(challenge);
        }
        Request::Session(session) => {
            body["session"] = json!(*session as i64);
        }
        Request::Magic(_) => return Err("remote ATTK operation is unsupported".to_string()),
    }
    Ok(body)
}

fn decode_reply(tx: u32, body: &Value) -> Result<Outcome, String> {
    let code = body
        .get("error_code")
        .and_then(Value::as_i64)
        .and_then(|v| i32::try_from(v).ok())
        .ok_or("remote SOTER reply has no valid error_code")?;
    if tx == dispatch::TX_INIT_SIGN {
        let session = body
            .get("session")
            .and_then(Value::as_i64)
            .map(|v| v as u64)
            .ok_or("remote SOTER init_sign reply has no valid session")?;
        return Ok(Outcome::Init { code, session });
    }
    if matches!(
        tx,
        dispatch::TX_EXPORT_ASK
            | dispatch::TX_EXPORT_AUTH
            | dispatch::TX_FINISH_SIGN
            | dispatch::TX_GET_DEVICE_ID
    ) {
        let data = match body.get("data").and_then(Value::as_str) {
            Some(encoded) => base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| "remote SOTER reply has invalid base64")?,
            None if code != 0 => Vec::new(),
            None => return Err("remote SOTER success reply is missing data".to_string()),
        };
        let field = body
            .get("length")
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok())
            .unwrap_or(data.len() as i32);
        return Ok(Outcome::Buffer { code, data, field });
    }
    Ok(Outcome::Code(code))
}

fn http_client(tls_insecure: bool) -> Result<reqwest::blocking::Client, String> {
    let cache = CLIENT.get_or_init(|| Mutex::new(None));
    let mut slot = cache.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((current, client)) = slot.as_ref() {
        if *current == tls_insecure {
            return Ok(client.clone());
        }
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(20))
        .danger_accept_invalid_certs(tls_insecure)
        .build()
        .map_err(|error| format!("SOTER client setup failed: {error}"))?;
    *slot = Some((tls_insecure, client.clone()));
    Ok(client)
}

pub fn forward(config: &Config, tx: u32, request: &Request) -> Result<Outcome, String> {
    let body = request_body(config, tx, request)?;
    let url = format!("{}/api/soter/", config.url.trim_end_matches('/'));
    let client = http_client(config.tls_insecure)?;
    let response = client
        .post(url)
        .header("X-Relay-Token", &config.token)
        .json(&body)
        .send()
        .map_err(|error| format!("SOTER relay transport failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("SOTER relay returned HTTP {}", response.status()));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("SOTER relay read failed: {error}"))?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err("SOTER relay reply is too large".to_string());
    }
    let reply: Value =
        serde_json::from_slice(&bytes).map_err(|_| "SOTER relay reply is not JSON".to_string())?;
    if reply.get("error").is_some() {
        return Err("SOTER relay could not serve this operation".to_string());
    }
    decode_reply(tx, &reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    #[test]
    fn separate_config_is_off_by_default_and_uid_map_is_explicit() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
        let cfg = Config::parse(
            "enabled=true\nurl=https://example.test\ntoken=soter-only\ndevice_id=synthetic-b\nuid_map=10001=10002",
        )
        .unwrap();
        assert_eq!(cfg.device_id, "synthetic-b");
        assert_eq!(mapped_uid(10001, &cfg.uid_map), 10002);
        assert_eq!(mapped_uid(20001, &cfg.uid_map), 20001);
        let body = request_body(
            &cfg,
            dispatch::TX_INIT_SIGN,
            &Request::InitSign {
                uid: 10001,
                kname: "AuthKey".into(),
                challenge: "nonce".into(),
            },
        )
        .unwrap();
        assert_eq!(body["uid"], 10002);
        assert_eq!(body["device_id"], "synthetic-b");
        assert_eq!(body["challenge"], "nonce");
    }

    #[test]
    fn remote_reply_preserves_session_and_negative_vendor_status() {
        assert_eq!(
            decode_reply(
                dispatch::TX_INIT_SIGN,
                &json!({"error_code": 0, "session": 42}),
            )
            .unwrap(),
            Outcome::Init {
                code: 0,
                session: 42
            }
        );
        assert_eq!(
            decode_reply(
                dispatch::TX_FINISH_SIGN,
                &json!({"error_code": -26, "data": "", "length": 0}),
            )
            .unwrap(),
            Outcome::Buffer {
                code: -26,
                data: Vec::new(),
                field: 0,
            }
        );
        assert!(
            request_body(&Config::default(), dispatch::TX_EXPORT_ATTK, &Request::None).is_err()
        );
        let signed = decode_reply(
            dispatch::TX_INIT_SIGN,
            &json!({"error_code": 0, "session": -2}),
        )
        .unwrap();
        assert_eq!(
            signed,
            Outcome::Init {
                code: 0,
                session: (-2i64) as u64
            }
        );
    }

    #[test]
    fn forwards_to_the_soter_server_with_its_own_token_and_device() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let count = socket.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..count]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]);
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|v| v.parse::<usize>().ok())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let answer = br#"{"op":"init_sign","error_code":0,"session":42}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        answer.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            socket.write_all(answer).unwrap();
            String::from_utf8(request).unwrap()
        });
        let config = Config {
            enabled: true,
            url: format!("http://127.0.0.1:{port}"),
            token: "soter-token".into(),
            device_id: "soter-b".into(),
            uid_map: "10001=10002".into(),
            ..Config::default()
        };
        let outcome = forward(
            &config,
            dispatch::TX_INIT_SIGN,
            &Request::InitSign {
                uid: 10001,
                kname: "AuthKey".into(),
                challenge: "nonce".into(),
            },
        )
        .unwrap();
        assert_eq!(
            outcome,
            Outcome::Init {
                code: 0,
                session: 42
            }
        );
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /api/soter/ HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("x-relay-token: soter-token"));
        assert!(request.contains("\"device_id\":\"soter-b\""));
        assert!(request.contains("\"uid\":10002"));
    }
}
