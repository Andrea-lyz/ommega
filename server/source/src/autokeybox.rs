//! Auto keybox refresh: periodically pull keybox material from configured
//! upstream URLs, parse it into PEM identities, and store it under a fixed
//! device_id (mirrors Django's `keybox_automation.py`).
//!
//! Supports:
//!   - http and https (reqwest + rustls, no system OpenSSL)
//!   - primary URL + mirror URL + GitHub mirror rewrites
//!   - hex- and base64-wrapped payloads
//!   - keybox XML, bare PEM bundles, or JSON/text wrappers
//!
//! All DB writes go through the parameterised `upsert_device_identity`, so no
//! string is ever interpolated into SQL (SQL-injection safe).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;

use crate::db::Db;
use crate::keybox::KeyboxData;

/// Global enable flag for the auto-refresh background loop.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Runtime-overridable device_id per source name (initialised from env, editable
/// via the admin API).
static DEVICE_IDS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn device_ids() -> &'static Mutex<HashMap<String, String>> {
    DEVICE_IDS.get_or_init(|| {
        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let mut m = HashMap::new();
        m.insert("yurikey".to_string(), env("KEYBOX_DEVICE_B1_ID", "device-b-1"));
        m.insert("kow".to_string(), env("KEYBOX_DEVICE_B2_ID", "device-b-2"));
        Mutex::new(m)
    })
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(v: bool) {
    ENABLED.store(v, Ordering::Relaxed);
}

/// Get the current device_id for a source name.
pub fn device_id_for(name: &str) -> String {
    crate::util::mu(&device_ids())
        .get(name)
        .cloned()
        .unwrap_or_default()
}

/// Set (override) the device_id for a source name.
pub fn set_device_id(name: &str, device_id: &str) {
    crate::util::mu(&device_ids())
        .insert(name.to_string(), device_id.to_string());
}

/// A single configured upstream keybox source.
#[derive(Debug, Clone)]
pub struct KeyboxSource {
    pub name: String,
    pub device_id: String,
    pub url_primary: String,
    pub url_mirror: String,
    pub decode_hex: bool,
    /// When true, `url_primary` is a JSON API returning a `keyboxes` list (each
    /// with `identity`/`status`); valid entries are downloaded individually and
    /// matched to `device_id` with `-1`, `-2`, ... suffixes.
    pub api_list: bool,
}

/// Build the configured sources from environment variables (URLs) and the
/// runtime-overridable device_id map.
pub fn configured_sources() -> Vec<KeyboxSource> {
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    vec![
        KeyboxSource {
            name: "yurikey".to_string(),
            device_id: device_id_for("yurikey"),
            url_primary: env(
                "KEYBOX_YURI_URL",
                "https://raw.githubusercontent.com/Yurii0307/yurikey/main/key",
            ),
            url_mirror: env(
                "KEYBOX_YURI_MIRROR_URL",
                "https://hub.gitmirror.com/raw.githubusercontent.com/Yurii0307/yurikey/main/key",
            ),
            decode_hex: false,
            api_list: false,
        },
        KeyboxSource {
            name: "kow".to_string(),
            device_id: device_id_for("kow"),
            url_primary: env(
                "KEYBOX_KOW_URL",
                "https://keybox.kowx712.cc/api/keyboxes",
            ),
            url_mirror: env(
                "KEYBOX_KOW_MIRROR_URL",
                "https://keybox.kowx712.cc/api/keyboxes",
            ),
            decode_hex: false,
            api_list: true,
        },
    ]
}

/// Fetch a URL over http/https, returning the body text.
fn http_get(url: &str, timeout: Duration) -> anyhow::Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent("Mozilla/5.0")
        .build()?;
    let resp = client.get(url).send()?;
    let bytes = resp.bytes()?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Fetch from primary, then mirror, then GitHub-mirror rewrites.
fn fetch_source(src: &KeyboxSource) -> anyhow::Result<String> {
    let mut candidates: Vec<String> = vec![src.url_primary.clone()];
    if !src.url_mirror.is_empty() {
        candidates.push(src.url_mirror.clone());
    }
    for u in [src.url_primary.clone(), src.url_mirror.clone()] {
        if u.contains("raw.githubusercontent.com") {
            candidates.push(
                u.replace(
                    "raw.githubusercontent.com",
                    "ghproxy.com/https://raw.githubusercontent.com",
                ),
            );
            candidates.push(
                u.replace(
                    "raw.githubusercontent.com",
                    "raw.gitmirror.com/raw.githubusercontent.com",
                ),
            );
        }
    }

    let mut last_err: Option<anyhow::Error> = None;
    for (i, url) in candidates.iter().enumerate() {
        if url.is_empty() {
            continue;
        }
        if i > 0 {
            std::thread::sleep(Duration::from_secs(2));
        }
        match http_get(url, Duration::from_secs(30)) {
            Ok(body) => return Ok(body),
            Err(e) => {
                tracing::warn!("autokeybox fetch failed url={url} err={e}");
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no candidate URL")))
}

/// Decode hex- or base64-wrapped payloads (mirrors `_maybe_decode_ns_payload`).
fn maybe_decode_ns_payload(raw: &str, decode_hex: bool) -> String {
    let mut text = raw.trim().to_string();
    if text.is_empty() {
        return text;
    }
    if decode_hex {
        let hex_only: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if !hex_only.is_empty() && hex_only.len() % 2 == 0 {
            if let Ok(decoded) = hex_decode(&hex_only) {
                if let Ok(s) = String::from_utf8(decoded) {
                    text = s;
                }
            }
        }
    }
    // Try base64 if the whole thing looks like a compact base64 blob.
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if !compact.is_empty() {
        if let Ok(decoded) = base64_decode(&compact) {
            let s = String::from_utf8_lossy(&decoded).into_owned();
            if s.contains('<') || s.contains("BEGIN ") || s.contains("AndroidAttestation") {
                text = s;
            }
        }
    }
    text
}

fn hex_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    let bytes = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(bytes)
}

fn base64_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.decode(s)?)
}

/// Normalise raw source text into a keybox-XML-compatible payload.
fn build_keybox_xml(source_text: &str, source_name: &str) -> String {
    let raw = source_text.trim();
    if raw.starts_with("<?xml") && raw.contains("<Key") {
        return raw.to_string();
    }
    // Extract PEM blocks from wrapped text and synthesise a minimal keybox XML.
    let pem_blocks = extract_pem_blocks(raw);
    if !pem_blocks.is_empty() {
        let mut keys = Vec::new();
        for block in pem_blocks {
            if block.contains("BEGIN CERTIFICATE") {
                continue;
            }
            keys.push(format!(
                "  <Key>\n    <PrivateKey>{block}</PrivateKey>\n    <CertificateChain>\n    </CertificateChain>\n  </Key>"
            ));
        }
        if !keys.is_empty() {
            return format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<AndroidAttestation source=\"{source_name}\">\n{}\n</AndroidAttestation>\n",
                keys.join("\n")
            );
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<AndroidAttestation source=\"{source_name}\">\n  <Raw><![CDATA[\n{raw}\n  ]]></Raw>\n</AndroidAttestation>\n"
    )
}

/// Extract `-----BEGIN ...----- ... -----END ...-----` blocks from text.
fn extract_pem_blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block = false;
    let mut cur = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("-----BEGIN ") {
            in_block = true;
            cur.clear();
            cur.push_str(line);
            cur.push('\n');
        } else if in_block {
            cur.push_str(line);
            cur.push('\n');
            if line.trim().starts_with("-----END ") {
                out.push(cur.clone());
                in_block = false;
                cur.clear();
            }
        }
    }
    out
}

/// Refresh a single source: fetch -> decode -> parse -> store.
pub fn refresh_one(src: &KeyboxSource, db: &Db) -> bool {
    if src.api_list {
        return refresh_api_list_source(src, db);
    }
    let body = match fetch_source(src) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("autokeybox fetch failed source={} err={e}", src.name);
            return false;
        }
    };
    let decoded = maybe_decode_ns_payload(&body, src.decode_hex);
    let xml = build_keybox_xml(&decoded, &src.name);
    let parsed = match crate::keybox::parse_keybox_xml_all(&xml) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("autokeybox parse failed source={} err={e}", src.name);
            return false;
        }
    };
    if parsed.is_empty() {
        tracing::warn!("autokeybox parse empty source={}", src.name);
        return false;
    }
    for kb in parsed {
        store_identity(db, src, &src.device_id, &kb);
    }
    tracing::info!(
        "autokeybox updated device_id={} source={}",
        src.device_id,
        src.name
    );
    true
}

/// Refresh an API-list source: fetch the keybox list, filter `valid` entries,
/// download each, and match them to `device_id` with `-1`/`-2`/... suffixes.
fn refresh_api_list_source(src: &KeyboxSource, db: &Db) -> bool {
    let list_body = match fetch_source(src) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("autokeybox list fetch failed source={} err={e}", src.name);
            return false;
        }
    };
    // Parse the JSON list: { "keyboxes": [ { "identity", "status", ... } ] }
    let identities: Vec<String> = match serde_json::from_str::<Value>(&list_body) {
        Ok(Value::Object(o)) => o
            .get("keyboxes")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter(|kb| kb.get("status").and_then(Value::as_str) == Some("valid"))
                    .filter_map(|kb| kb.get("identity").and_then(Value::as_str).map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    if identities.is_empty() {
        tracing::warn!("autokeybox list empty/parse failed source={}", src.name);
        return false;
    }
    tracing::info!(
        "autokeybox list source={} valid_count={}",
        src.name,
        identities.len()
    );

    let base = src.device_id.clone();
    for (i, identity) in identities.iter().enumerate() {
        // First valid keybox -> base device_id; subsequent -> base-N (base-1,
        // base-2, ...). The suffix is `i`, not `i + 1`, so the second identity
        // gets `base-1` as documented.
        let device_id = if i == 0 {
            base.clone()
        } else {
            format!("{base}-{i}")
        };
        match download_and_store(db, src, identity, &device_id) {
            Ok(true) => {
                tracing::info!("autokeybox stored {device_id} source={}", src.name);
            }
            Ok(false) => {
                tracing::warn!("autokeybox download rejected source={} identity={identity}", src.name);
            }
            Err(e) => {
                tracing::warn!(
                    "autokeybox download failed source={} identity={identity} err={e}",
                    src.name
                );
            }
        }
    }
    true
}

/// Download a single keybox by identity and store it under `device_id`.
/// Returns Ok(true) on success, Ok(false) if the server rejected (e.g. bot
/// challenge), Err on network/parse failure.
fn download_and_store(
    db: &Db,
    src: &KeyboxSource,
    identity: &str,
    device_id: &str,
) -> anyhow::Result<bool> {
    // Step 1: POST /api/keyboxes/:identity/download -> { token, url }
    let dl_endpoint = format!("{}/api/keyboxes/{identity}/download", base_url(src));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Mozilla/5.0")
        .build()?;
    let resp = client
        .post(&dl_endpoint)
        .header("Content-Type", "application/json")
        .send()?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        // Server-side rejection (bot challenge / not found / forbidden).
        tracing::warn!(
            "autokeybox download endpoint {dl_endpoint} -> {status}: {text}"
        );
        return Ok(false);
    }
    // Parse token/url.
    let (token, direct_url) = match serde_json::from_str::<Value>(&text) {
        Ok(v) => (
            v.get("token").and_then(Value::as_str).map(String::from),
            v.get("url").and_then(Value::as_str).map(String::from),
        ),
        Err(_) => (None, None),
    };

    // Step 2: fetch the actual keybox content.
    let content = if let Some(url) = direct_url {
        http_get(&url, Duration::from_secs(30))?
    } else if let Some(tok) = token {
        let dl_url = format!("{}/download/{tok}", base_url(src));
        http_get(&dl_url, Duration::from_secs(30))?
    } else {
        // The download response itself might be the keybox content.
        if text.contains("BEGIN ") || text.contains("<Keybox") || text.contains("<?xml") {
            text
        } else {
            tracing::warn!("autokeybox download response unrecognized: {text}");
            return Ok(false);
        }
    };

    // Parse and store (may contain RSA + EC).
    let decoded = maybe_decode_ns_payload(&content, false);
    let xml = build_keybox_xml(&decoded, &src.name);
    let parsed = crate::keybox::parse_keybox_xml_all(&xml)?;
    for kb in parsed {
        store_identity(db, src, device_id, &kb);
    }
    Ok(true)
}

fn base_url(src: &KeyboxSource) -> String {
    // Strip the trailing /api/keyboxes to get the site root.
    let u = src.url_primary.clone();
    let u = u.trim_end_matches('/');
    if let Some(idx) = u.find("/api/") {
        u[..idx].to_string()
    } else {
        u.to_string()
    }
}

fn store_identity(db: &Db, src: &KeyboxSource, device_id: &str, kb: &KeyboxData) {
    // Reject a mismatched key/chain before it can poison attestation: the
    // b_upload path validates the same way, and auto-refresh should not be
    // laxer (a broken identity would mint an unverifiable leaf chain).
    if let Some(err) = crate::cert::validate_identity_pem(
        &kb.private_key_pem,
        &kb.certificate_chain_pem,
    ) {
        tracing::warn!(
            "autokeybox validate failed device_id={device_id} source={} err={err}",
            src.name
        );
        return;
    }
    let identity = crate::db::DeviceIdentity {
        device_id: device_id.to_string(),
        algorithm: kb.algorithm.clone(),
        certificate_chain_pem: kb.certificate_chain_pem.clone(),
        private_key_pem_cipher: kb.private_key_pem.clone(),
        active: true,
        machine_id: format!("auto:{}", src.name),
        created_at: String::new(),
    };
    if let Err(e) = db.upsert_device_identity(&identity) {
        tracing::warn!("autokeybox store failed device_id={device_id} err={e}");
    }
}

/// Refresh all configured sources once (blocking).
pub fn refresh_all(db: &Db) {
    for src in configured_sources() {
        if let Err(e) = std::panic::catch_unwind(|| refresh_one(&src, db)) {
            tracing::warn!("autokeybox refresh panicked source={} err={:?}", src.name, e);
        }
    }
}

/// Start the background refresh loop in a dedicated thread. Runs an immediate
/// refresh, then repeats every `interval`. Stops when the flag is cleared.
pub fn start_background(db: Arc<Db>, interval: Duration) {
    let db = db.clone();
    std::thread::Builder::new()
        .name("autokeybox".to_string())
        .spawn(move || {
            tracing::info!("autokeybox loop started interval={:?}", interval);
            loop {
                if !is_enabled() {
                    break;
                }
                refresh_all(&db);
                // Sleep in small slices so a disable can be observed promptly.
                let mut waited = Duration::ZERO;
                while waited < interval {
                    if !is_enabled() {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                    waited += Duration::from_secs(1);
                }
            }
            tracing::info!("autokeybox loop stopped");
        })
        .ok();
}
