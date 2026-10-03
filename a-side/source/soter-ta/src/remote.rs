//! Optional SOTER relay behind the software TA's native HAL service.
//!
//! This configuration is deliberately separate from the KeyMint/Integrity
//! relay. A disabled relay never touches the network or the local SOTER ledger.
//! An enabled relay that fails must not fall back to the local ledger: that
//! would mix two different ASK/AuthKey identities in the same application slot.

use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};

use crate::dispatch::{self, Outcome, Request};

pub const CONFIG_PATH: &str = "/data/adb/ommega/soterta/remote.conf";
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
static CLIENT: OnceLock<Mutex<Option<(bool, reqwest::blocking::Client)>>> = OnceLock::new();

/// Field the relay server reads to classify the call: the comma-joined names of
/// the packages installed for the calling uid, when there are any.
pub const CALLER_PKG_FIELD: &str = "caller_pkg";
/// Field the relay server reads to mint its built-in fallback answers with this
/// device's identity instead of one derived from the moment it answered.
pub const CPU_ID_FIELD: &str = "cpu_id";

/// Android's uid -> package mapping. The SOTER host passes the calling
/// application's uid as a request parameter; this file turns it back into
/// package names.
const PACKAGES_LIST: &str = "/data/system/packages.list";
/// The mapping only moves when applications are installed or removed; a minute
/// follows that closely enough while keeping the relay path cheap.
const CALLER_CACHE_TTL: Duration = Duration::from_secs(60);
const CALLER_CACHE_MAX: usize = 256;
static CALLER_NAMES: Mutex<Vec<(u32, Instant, Option<String>)>> = Mutex::new(Vec::new());

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

/// Whether `text` is a well-formed SOTER device id: 32 hex characters, the
/// shape a real vendor TA reports and this ledger persists.
fn is_soter_cpu_id(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The uid the SOTER host passed for this call, when the transaction carries
/// one.
///
/// It is a request parameter, not the kernel's sender uid: this HAL service
/// sits below the SOTER host, which fills the parameter from the calling
/// application. The relay server therefore treats the resolved packages as a
/// label for who is calling - used to keep real SOTER applications on the real
/// device while probes and test traffic are answered from built-in material.
fn request_uid(request: &Request) -> Option<u32> {
    match request {
        Request::Uid(uid) => Some(*uid),
        Request::UidKey { uid, .. } => Some(*uid),
        Request::InitSign { uid, .. } => Some(*uid),
        _ => None,
    }
}

/// Package names installed for `uid`, in listing order.
fn packages_for_uid(listing: &str, uid: u32) -> Vec<String> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            let file_uid = fields.next()?.parse::<u32>().ok()?;
            (!name.is_empty() && file_uid == uid).then(|| name.to_string())
        })
        .collect()
}

/// Comma-joined package names for `uid`, cached for [`CALLER_CACHE_TTL`].
///
/// `None` when the uid owns no package (system uids) or the mapping cannot be
/// read; the relay request then carries no `caller_pkg` and the server falls
/// back to its own alias heuristics.
fn caller_package_for_uid(uid: u32) -> Option<String> {
    if let Ok(entries) = CALLER_NAMES.lock() {
        if let Some((_, at, hit)) = entries.iter().find(|(cached, _, _)| *cached == uid) {
            if at.elapsed() < CALLER_CACHE_TTL {
                return hit.clone();
            }
        }
    }
    let hit = match std::fs::read_to_string(PACKAGES_LIST) {
        Ok(listing) => {
            let joined = packages_for_uid(&listing, uid).join(",");
            (!joined.is_empty()).then_some(joined)
        }
        Err(error) => {
            eprintln!("soterta remote: cannot read {PACKAGES_LIST}: {error}");
            None
        }
    };
    if let Ok(mut entries) = CALLER_NAMES.lock() {
        if entries.len() >= CALLER_CACHE_MAX {
            entries.clear();
        }
        entries.retain(|(cached, _, _)| *cached != uid);
        entries.push((uid, Instant::now(), hit.clone()));
    }
    hit
}

fn request_body(
    config: &Config,
    tx: u32,
    request: &Request,
    cpu_id: &str,
    caller_pkg: Option<&str>,
) -> Result<Value, String> {
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
        // The ATTK trio is this device's own factory key pair and has no relay
        // operation: the QTI B-side HAL exposes no verified ATTK, and `ffi`
        // answers these three codes from the local ledger before it ever
        // consults the relay. Reaching this point means the caller skipped that
        // split, so refuse instead of inventing a remote ATTK.
        TX_EXPORT_ATTK | TX_GENERATE_ATTK | TX_VERIFY_ATTK => {
            return Err("ATTK transactions are answered locally, not relayed".to_string())
        }
        _ => return Err("unknown SOTER transaction".to_string()),
    };
    let mut body = json!({"op": op, "device_id": config.device_id});
    // The relay server mints its built-in fallback answers with `cpu_id` and
    // routes real SOTER applications by `caller_pkg` (see `forward`). Both are
    // read only by the server tier: it strips `cpu_id` before handing a request
    // to the real device, and the device ignores either field anyway.
    if is_soter_cpu_id(cpu_id) {
        body[CPU_ID_FIELD] = json!(cpu_id);
    }
    if let Some(pkg) = caller_pkg.map(str::trim).filter(|pkg| !pkg.is_empty()) {
        body[CALLER_PKG_FIELD] = json!(pkg);
    }
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
        Request::Magic(_) => {
            return Err("ATTK transactions are answered locally, not relayed".to_string())
        }
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
    if tx == dispatch::TX_GET_DEVICE_ID {
        let payload = match body.get("data").and_then(Value::as_str) {
            Some(encoded) => base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| "remote SOTER reply has invalid base64")?,
            None if code != 0 => Vec::new(),
            None => return Err("remote SOTER success reply is missing data".to_string()),
        };
        let data = device_id_bytes(&payload);
        return Ok(Outcome::Buffer {
            code,
            field: data.len() as i32,
            data,
        });
    }
    if matches!(
        tx,
        dispatch::TX_EXPORT_ASK | dispatch::TX_EXPORT_AUTH | dispatch::TX_FINISH_SIGN
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

/// The relay answers `get_device_id` the way the Java face spells it: 32 ASCII
/// hex characters with a trailing NUL (captured from the public relay,
/// 2026-09-28: 33 bytes for `090000005171734c42866bea148b21f5`). This TA replaces
/// the HAL underneath that Java layer, and a live HAL answers the transaction
/// with the 16 raw bytes whose hex is that string - the same bytes
/// [`crate::state::TaState::device_id`] hands out locally, and the same hex
/// every blob carries as `cpu_id`. Handing the spelling up instead would let one
/// device report two different ids.
fn device_id_bytes(payload: &[u8]) -> Vec<u8> {
    let kept = payload
        .iter()
        .rev()
        .skip_while(|byte| **byte == 0 || byte.is_ascii_whitespace())
        .count();
    let trimmed = &payload[..kept];
    match std::str::from_utf8(trimmed) {
        Ok(text) if text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit()) => {
            hex::decode(text).unwrap_or_else(|_| payload.to_vec())
        }
        _ => payload.to_vec(),
    }
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

pub fn forward(
    config: &Config,
    tx: u32,
    request: &Request,
    cpu_id: &str,
) -> Result<Outcome, String> {
    // The caller's package names let the relay server keep real SOTER
    // applications on the real device, and the device id lets it mint its
    // built-in answers with one stable identity. Without them an application
    // whose alias is not in the server's list is answered from built-in
    // material that carries a moment-derived id, and both show up as device
    // changes to whatever inspects the replies.
    let caller_pkg = request_uid(request).and_then(caller_package_for_uid);
    let body = request_body(config, tx, request, cpu_id, caller_pkg.as_deref())?;
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
            "",
            None,
        )
        .unwrap();
        assert_eq!(body["uid"], 10002);
        assert_eq!(body["device_id"], "synthetic-b");
        assert_eq!(body["challenge"], "nonce");
        assert!(body.get("cpu_id").is_none());
        assert!(body.get("caller_pkg").is_none());
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
        // The ATTK trio is answered from the local ledger (ffi never forwards
        // it), so the relay mapping must not invent a remote ATTK operation.
        assert!(request_body(
            &Config::default(),
            dispatch::TX_EXPORT_ATTK,
            &Request::None,
            "",
            None
        )
        .is_err());
        assert!(request_body(
            &Config::default(),
            dispatch::TX_VERIFY_ATTK,
            &Request::None,
            "",
            None
        )
        .is_err());
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
    fn the_relayed_device_id_is_handed_up_as_the_raw_hal_bytes() {
        // The relay spells the id the way the Java face does: 32 hex characters
        // plus a NUL, base64-wrapped (public relay capture, 2026-09-28).
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(b"090000005171734c42866bea148b21f5\0");
        assert_eq!(
            decode_reply(
                dispatch::TX_GET_DEVICE_ID,
                &json!({
                    "error_code": 0,
                    "data": encoded,
                    "length": 33,
                    "text": "090000005171734c42866bea148b21f5"
                }),
            )
            .unwrap(),
            Outcome::Buffer {
                code: 0,
                data: hex::decode("090000005171734c42866bea148b21f5").unwrap(),
                field: 16,
            }
        );
        // A reply that already carries raw bytes is passed through untouched.
        let raw = base64::engine::general_purpose::STANDARD.encode([0x0au8, 0xff, 0x00]);
        assert_eq!(
            decode_reply(
                dispatch::TX_GET_DEVICE_ID,
                &json!({"error_code": 0, "data": raw, "length": 3}),
            )
            .unwrap(),
            Outcome::Buffer {
                code: 0,
                data: vec![0x0a, 0xff, 0x00],
                field: 3,
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
            "09000000aabbccddeeff001122334455",
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
        assert!(request.contains("\"cpu_id\":\"09000000aabbccddeeff001122334455\""));
    }

    /// The identity fields only go out when they carry something the server can
    /// use: a well-formed device id, and at least one package name.
    #[test]
    fn identity_fields_are_only_sent_when_they_carry_a_value() {
        let cfg = Config::default();
        let request = Request::InitSign {
            uid: 10373,
            kname: "AuthKey".into(),
            challenge: "nonce".into(),
        };
        let body = request_body(
            &cfg,
            dispatch::TX_INIT_SIGN,
            &request,
            "09000000aabbccddeeff001122334455",
            Some("com.tencent.mm,com.tencent.mm:tools"),
        )
        .unwrap();
        assert_eq!(body["cpu_id"], "09000000aabbccddeeff001122334455");
        assert_eq!(body["caller_pkg"], "com.tencent.mm,com.tencent.mm:tools");

        for bad in [
            "",
            "ledger",
            "09000000aabbccddeeff0011223344",
            "zz000000aabbccddeeff001122334455",
        ] {
            let body = request_body(&cfg, dispatch::TX_INIT_SIGN, &request, bad, None).unwrap();
            assert!(body.get("cpu_id").is_none(), "{bad:?} must not be sent");
        }
        for blank in [None, Some(""), Some("   ")] {
            let body = request_body(
                &cfg,
                dispatch::TX_INIT_SIGN,
                &request,
                "09000000aabbccddeeff001122334455",
                blank,
            )
            .unwrap();
            assert!(
                body.get("caller_pkg").is_none(),
                "{blank:?} must not be sent"
            );
        }
    }

    /// The uid only labels transactions that actually carry one: `getDeviceId`
    /// and the session-based reads cannot be resolved this way.
    #[test]
    fn the_uid_only_labels_calls_that_carry_one() {
        assert_eq!(request_uid(&Request::Uid(7)), Some(7));
        assert_eq!(
            request_uid(&Request::UidKey {
                uid: 8,
                kname: "k".into()
            }),
            Some(8)
        );
        assert_eq!(
            request_uid(&Request::InitSign {
                uid: 9,
                kname: "k".into(),
                challenge: "c".into()
            }),
            Some(9)
        );
        assert_eq!(request_uid(&Request::None), None);
        assert_eq!(request_uid(&Request::Session(1)), None);
    }

    /// A uid can own several packages (shared uids); every name rides along so
    /// the server's whitelist sees the application, and a malformed line is
    /// skipped instead of failing the whole listing.
    #[test]
    fn packages_list_resolves_a_uid_to_its_package_names() {
        let listing = "\
com.tencent.mm 10373 1 /data/user/0/com.tencent.mm default:targetSdkVersion=34 3003,1015
com.eltavine.duckdetector 10401 0 /data/user/0/com.eltavine.duckdetector default:targetSdkVersion=35 3003
com.google.android.gms 10123 0 /data/user/0/com.google.android.gms default:targetSdkVersion=35 3003
com.google.android.gms.setup 10123 0 /data/user/0/com.google.android.gms default:targetSdkVersion=35 3003
";
        assert_eq!(packages_for_uid(listing, 10373), ["com.tencent.mm"]);
        assert_eq!(
            packages_for_uid(listing, 10123),
            ["com.google.android.gms", "com.google.android.gms.setup"]
        );
        assert!(packages_for_uid(listing, 1000).is_empty());
        assert!(packages_for_uid("", 1).is_empty());
        assert!(packages_for_uid("broken\nname not-a-uid /data/user/0/x default\n", 1).is_empty());
    }
}
