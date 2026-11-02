//! CDI seed, keybox import and RKP refresh policy for the automation page.
//!
//! The module ships no seed and no keybox: the user imports their own material.
//! The seed bundle is stored AES-256-GCM encrypted under data/rkp/, next to a
//! per-install key that only the keystore uid can read, so a copied file is not
//! a usable seed on its own.
//!
//! Commands (one JSON document on stdout, errors on stderr with exit code 1):
//!   status
//!   seed-import <bundle.json>
//!   seed-remove
//!   keybox-import <tee|strongbox> <keybox.xml>
//!   policy --auto <on|off> | --refresh-hours <n>
//!   due
//!
//! The network side of RKP lives in rkp_engine: it is only reached through
//! "issue", which stays disabled until an issuance has been validated.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use kmr_common::crypto::Rng as _;
use kmr_common::crypto::Sha256 as _;
use kmr_crypto_ommega::rng::OmmegaRng;
use kmr_crypto_ommega::sha256::OmmegaSha256;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const STATE_DIR: &str = "/data/misc/keystore/ommega/data/rkp";
const KEYBOX_TEE: &str = "/data/misc/keystore/ommega/keybox.xml";
const KEYBOX_STRONGBOX: &str = "/data/misc/keystore/ommega/keybox-strongbox.xml";
const MASTER_KEY: &str = "/data/misc/keystore/ommega/data/rkp/master.key";
const SEED_BLOB: &str = "/data/misc/keystore/ommega/data/rkp/seed.enc";
const SEED_META: &str = "/data/misc/keystore/ommega/data/rkp/seed.json";
const POLICY: &str = "/data/misc/keystore/ommega/data/rkp/policy.json";
const FEEDBACK: &str = "/data/misc/keystore/ommega/data/rkp/feedback.json";
const BUNDLE_LIMIT: u64 = 512 * 1024;
const KEYBOX_LIMIT: u64 = 256 * 1024;
const DEFAULT_REFRESH_HOURS: u64 = 72;

#[derive(Debug, Serialize, Deserialize)]
struct SeedMeta {
    fingerprint: String,
    imported_at_ms: u64,
    has_chain: bool,
    source: String,
    #[serde(default)]
    algorithm: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Policy {
    auto: bool,
    #[serde(default = "default_refresh_hours")]
    refresh_hours: u64,
    #[serde(default)]
    last_issue_ms: u64,
    #[serde(default)]
    last_error: String,
}

fn default_refresh_hours() -> u64 {
    DEFAULT_REFRESH_HOURS
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            auto: false,
            refresh_hours: DEFAULT_REFRESH_HOURS,
            last_issue_ms: 0,
            last_error: String::new(),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn ensure_dir() -> Result<()> {
    let path = Path::new(STATE_DIR);
    fs::create_dir_all(path).context("create the RKP state directory")?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn write_private(path: &str, bytes: &[u8]) -> Result<()> {
    let temp = format!("{path}.tmp-{}", std::process::id());
    fs::write(&temp, bytes).with_context(|| format!("write {temp}"))?;
    fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&temp, path).with_context(|| format!("replace {path}"))?;
    Ok(())
}

fn read_limited(path: &str, limit: u64) -> Result<Vec<u8>> {
    let meta = fs::metadata(path).with_context(|| format!("stat {path}"))?;
    if meta.len() > limit {
        bail!("{path} is larger than {limit} bytes");
    }
    fs::read(path).with_context(|| format!("read {path}"))
}

/// Per-install key for the seed blob. Missing file means "no seed imported".
fn master_key(create: bool) -> Result<Option<[u8; 32]>> {
    match fs::read(MASTER_KEY) {
        Ok(bytes) if bytes.len() == 32 => Ok(Some(bytes.try_into().unwrap())),
        Ok(_) => bail!("master key has the wrong length"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if !create {
                return Ok(None);
            }
            ensure_dir()?;
            let mut key = [0u8; 32];
            OmmegaRng.fill_bytes(&mut key);
            write_private(MASTER_KEY, &key)?;
            Ok(Some(key))
        }
        Err(error) => Err(error).context("read master key"),
    }
}

/// The imported material bundle, using the Duck ToolBox version 1 contract.
///
/// It carries the device's real DICE chain, the key that signs the CSR challenge
/// and the identity the RKP server expects; a bare seed cannot be issued against
/// because the server validates the registered chain, not just a leaf key.
#[derive(Debug, Deserialize)]
struct DuckBundle {
    version: u32,
    dice_chain_hex: String,
    signing: DuckSigning,
    #[serde(default)]
    device: Value,
    #[serde(default)]
    issuance: Value,
    #[serde(default)]
    uds_certs_hex: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DuckSigning {
    kind: String,
    bytes_hex: String,
    #[serde(default)]
    derivation: Option<String>,
}

/// Validates the parts of the bundle this tool can check without the engine.
fn check_bundle(bundle: &DuckBundle) -> Result<(String, String)> {
    if bundle.version != 1 {
        bail!("unsupported bundle version {}", bundle.version);
    }
    let expected = match bundle.signing.kind.as_str() {
        "p256-scalar" | "p256-seed" | "ed25519-seed" => 32,
        "p384-scalar" => 48,
        other => bail!("unsupported signing kind {other}"),
    };
    if let Some(derivation) = bundle.signing.derivation.as_deref() {
        let known = ["direct", "none", "duck-rkp-legacy-mod-n-minus-1-plus-1"];
        if !known.contains(&derivation) {
            bail!("unsupported signing derivation {derivation}");
        }
    }
    let secret = hex::decode(bundle.signing.bytes_hex.trim())
        .context("signing.bytes_hex is not hexadecimal")?;
    if secret.len() != expected {
        bail!(
            "{0} expects {expected} bytes, got {1}",
            bundle.signing.kind,
            secret.len()
        );
    }
    let chain =
        hex::decode(bundle.dice_chain_hex.trim()).context("dice_chain_hex is not hexadecimal")?;
    if chain.len() < 16 {
        bail!("dice_chain_hex is too short to be a DICE chain");
    }
    if !bundle.device.is_null() && !bundle.device.is_object() {
        bail!("device must be a JSON object");
    }
    if let Some(uds) = bundle.uds_certs_hex.as_deref() {
        if !uds.trim().is_empty() {
            hex::decode(uds.trim()).context("uds_certs_hex is not hexadecimal")?;
        }
    }
    let fingerprint = bundle
        .issuance
        .get("fingerprint")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok((bundle.signing.kind.clone(), fingerprint))
}

fn encrypt_seed(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow!("invalid AES key"))?;
    let mut nonce = [0u8; 12];
    OmmegaRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|_| anyhow!("seed encryption failed"))?;
    let mut blob = Vec::with_capacity(12 + ciphertext.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

fn decrypt_seed(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < 13 {
        bail!("seed blob is truncated");
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow!("invalid AES key"))?;
    cipher
        .decrypt(Nonce::from_slice(&blob[..12]), &blob[12..])
        .map_err(|_| anyhow!("seed blob cannot be decrypted with this install"))
}

fn load_meta() -> Option<SeedMeta> {
    fs::read(SEED_META)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn load_policy() -> Policy {
    fs::read(POLICY)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_policy(policy: &Policy) -> Result<()> {
    ensure_dir()?;
    write_private(POLICY, &serde_json::to_vec_pretty(policy)?)
}

// ---------------------------------------------------------------- keybox DER

struct DerItem {
    tag: u8,
    start: usize,
    end: usize,
}

fn der_length(bytes: &[u8], offset: usize) -> Option<(usize, usize)> {
    let first = *bytes.get(offset)?;
    if first < 0x80 {
        return Some((first as usize, offset + 1));
    }
    let count = (first & 0x7f) as usize;
    if count == 0 || count > 4 || offset + count >= bytes.len() {
        return None;
    }
    let mut length = 0usize;
    for index in 0..count {
        length = length * 256 + bytes[offset + 1 + index] as usize;
    }
    Some((length, offset + 1 + count))
}

fn der_item(bytes: &[u8], offset: usize) -> Option<DerItem> {
    let tag = *bytes.get(offset)?;
    let (length, start) = der_length(bytes, offset + 1)?;
    let end = start.checked_add(length)?;
    if end > bytes.len() {
        return None;
    }
    Some(DerItem { tag, start, end })
}

fn der_children(bytes: &[u8], item: &DerItem) -> Vec<DerItem> {
    let mut children = Vec::new();
    let mut offset = item.start;
    while offset < item.end {
        match der_item(bytes, offset) {
            Some(child) if child.end > offset => {
                offset = child.end;
                children.push(child);
            }
            _ => break,
        }
    }
    children
}

fn der_text(bytes: &[u8], item: &DerItem) -> String {
    String::from_utf8_lossy(&bytes[item.start..item.end]).to_string()
}

fn der_time(bytes: &[u8], item: &DerItem) -> Option<i64> {
    let raw = der_text(bytes, item);
    // UTCTime (0x17) carries a two-digit year, GeneralizedTime (0x18) a
    // four-digit one. Keying off the digit count instead read every
    // "261112102405Z" as the year 2611.
    let text: String = raw
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect();
    let (year, rest) = if item.tag == 0x18 {
        if text.len() < 12 {
            return None;
        }
        (text[0..4].parse::<i64>().ok()?, &text[4..])
    } else {
        if text.len() < 10 {
            return None;
        }
        let short = text[0..2].parse::<i64>().ok()?;
        (
            if short < 50 {
                short + 2000
            } else {
                short + 1900
            },
            &text[2..],
        )
    };
    let month = rest.get(0..2)?.parse::<i64>().ok()?;
    let day = rest.get(2..4)?.parse::<i64>().ok()?;
    let hour = rest.get(4..6)?.parse::<i64>().ok()?;
    let minute = rest.get(6..8)?.parse::<i64>().ok()?;
    let second = rest
        .get(8..10)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    Some(
        days_from_civil(year, month, day) * 86_400_000
            + (hour * 3600 + minute * 60 + second) * 1000,
    )
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[derive(Debug, Default, Clone)]
struct CertInfo {
    subject_o: Option<String>,
    subject_cn: Option<String>,
    issuer_cn: Option<String>,
    not_after_ms: Option<i64>,
}

fn name_field(bytes: &[u8], name: &DerItem, wanted: &[u8]) -> Option<String> {
    for set in der_children(bytes, name) {
        for attribute in der_children(bytes, &set) {
            let parts = der_children(bytes, &attribute);
            if parts.len() < 2 {
                continue;
            }
            let oid = &bytes[parts[0].start..parts[0].end];
            if oid == wanted {
                return Some(der_text(bytes, &parts[1]));
            }
        }
    }
    None
}

fn certificate_info(der: &[u8]) -> Option<CertInfo> {
    let certificate = der_item(der, 0)?;
    if certificate.tag != 0x30 {
        return None;
    }
    let tbs = der_children(der, &certificate).into_iter().next()?;
    let fields = der_children(der, &tbs);
    let mut index = if fields.first().map(|field| field.tag) == Some(0xa0) {
        1
    } else {
        0
    };
    index += 3; // serial, signature, issuer
    let validity = fields.get(index)?;
    let subject = fields.get(index + 1)?;
    let times = der_children(der, validity);
    if times.len() < 2 {
        return None;
    }
    Some(CertInfo {
        subject_o: name_field(der, subject, &[0x55, 0x04, 0x0a]),
        subject_cn: name_field(der, subject, &[0x55, 0x04, 0x03]),
        issuer_cn: name_field(der, &fields[index - 1], &[0x55, 0x04, 0x03]),
        not_after_ms: der_time(der, &times[1]),
    })
}

/// Decode the first PEM/DER certificate of a keybox document.
fn keybox_leaf(xml: &str) -> Option<(CertInfo, usize)> {
    let first = xml.find("-----BEGIN CERTIFICATE-----")?;
    let body_start = first + "-----BEGIN CERTIFICATE-----".len();
    let end = xml[body_start..].find("-----END CERTIFICATE-----")? + body_start;
    let body: String = xml[body_start..end]
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/' || *c == '=')
        .collect();
    let der = BASE64.decode(body).ok()?;
    let count = xml.matches("-----BEGIN CERTIFICATE-----").count();
    Some((certificate_info(&der)?, count))
}

fn keybox_status(path: &str, expected: &str) -> Value {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return json!({"present": false}),
    };
    match keybox_leaf(&contents) {
        Some((info, count)) => {
            let level = info
                .subject_o
                .clone()
                .filter(|value| {
                    value.eq_ignore_ascii_case("TEE") || value.eq_ignore_ascii_case("StrongBox")
                })
                .unwrap_or_default();
            let rkp = info
                .issuer_cn
                .as_deref()
                .map(|value| value.contains("Droid CA") || value.contains("Key Attestation"))
                .unwrap_or(false);
            let days = info
                .not_after_ms
                .map(|at| (at - now_ms() as i64).div_euclid(86_400_000));
            json!({
                "present": true,
                "level": level,
                "subject": info.subject_cn,
                "expected": expected,
                "match": level.eq_ignore_ascii_case(expected),
                "rkp": rkp,
                "issuer": info.issuer_cn,
                "chain": count,
                "not_after_ms": info.not_after_ms,
                "days": days,
            })
        }
        None => json!({"present": true, "unreadable": true}),
    }
}

fn slot_for(level: &str) -> Result<(&'static str, &'static str)> {
    match level {
        "tee" => Ok((KEYBOX_TEE, "TEE")),
        "strongbox" | "sb" => Ok((KEYBOX_STRONGBOX, "StrongBox")),
        other => bail!("unknown level {other}; use tee or strongbox"),
    }
}

/// Validate a keybox and place it in its slot. Shared by the file import and by
/// the issuance path so an issued chain gets the same checks as a downloaded one.
fn install_keybox(level: &str, bytes: &[u8]) -> Result<Value> {
    let (target, expected) = slot_for(level)?;
    let text = std::str::from_utf8(bytes).context("keybox must be UTF-8 XML")?;
    let (info, _count) =
        keybox_leaf(text).ok_or_else(|| anyhow!("no readable certificate in the keybox"))?;
    if !text.contains("<PrivateKey") {
        bail!("the keybox has no <PrivateKey> entry");
    }
    let label = info.subject_o.clone().unwrap_or_default();
    let contradicts = (expected == "TEE" && label.eq_ignore_ascii_case("StrongBox"))
        || (expected == "StrongBox" && label.eq_ignore_ascii_case("TEE"));
    if !label.is_empty() && contradicts {
        bail!("this keybox carries a {label} chain but the {level} slot was selected");
    }
    ensure_dir()?;
    write_private(target, bytes)?;
    Ok(json!({
        "saved": true,
        "level": level,
        "path": target,
        "switch": keybox_status(target, expected),
    }))
}

fn import_keybox(args: &[String]) -> Result<Value> {
    let level = args
        .first()
        .ok_or_else(|| anyhow!("usage: keybox-import <tee|strongbox> <keybox.xml>"))?
        .to_ascii_lowercase();
    let source = args
        .get(1)
        .ok_or_else(|| anyhow!("usage: keybox-import <tee|strongbox> <keybox.xml>"))?;
    let bytes = read_limited(source, KEYBOX_LIMIT)?;
    install_keybox(&level, &bytes)
}

fn write_feedback(message: &str, error: bool) -> Result<()> {
    ensure_dir()?;
    let payload = json!({"message": message, "at_ms": now_ms(), "error": error});
    write_private(FEEDBACK, &serde_json::to_vec_pretty(&payload)?)
}

/// Ask the engine to obtain fresh certificates for the stored bundle and install
/// the result in the requested slot.
fn issue(args: &[String]) -> Result<Value> {
    let level = args
        .first()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "tee".to_string());
    let (_, expected) = slot_for(&level)?;
    let meta = load_meta().ok_or_else(|| anyhow!("尚未导入 CDI 种子包，请先导入"))?;
    let key = master_key(false)?.ok_or_else(|| anyhow!("种子主密钥丢失，请重新导入种子包"))?;
    let blob = fs::read(SEED_BLOB).context("读取已保存的种子包")?;
    let plain = decrypt_seed(&key, &blob)?;
    // The claimed level decides the level of the issued chain, so the stored
    // bundle is retargeted at the slot the user picked. Without this a bundle
    // captured for TEE would return a TEE chain on both buttons, and the second
    // one would be refused by the slot check after spending an issuance.
    let mut bundle: Value = serde_json::from_slice(&plain).context("种子包不是合法 JSON")?;
    let wanted = if expected == "StrongBox" { "strongbox" } else { "tee" };
    let device = bundle
        .get_mut("device")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("种子包缺少 device 段，无法确定签发级别"))?;
    device.insert(
        "security_level".to_string(),
        Value::String(wanted.to_string()),
    );
    let plain = serde_json::to_vec(&bundle)?;
    ensure_dir()?;
    let temp_dir = Path::new(STATE_DIR).join("tmp");
    fs::create_dir_all(&temp_dir)?;
    let _ = fs::set_permissions(&temp_dir, fs::Permissions::from_mode(0o700));
    let temporary = temp_dir.join(format!("issue-{}.json", std::process::id()));
    let temporary_text = temporary.display().to_string();
    write_private(&temporary_text, &plain)?;
    let paths =
        ommega_rkp_engine::paths::AppPaths::for_root(Path::new(STATE_DIR), Path::new(STATE_DIR));
    let outcome = ommega_rkp_engine::issue_from_bundle(&paths, Path::new(&temporary_text));
    let _ = fs::remove_file(&temporary_text);
    let produced = match outcome {
        Ok(value) => value,
        Err(error) => {
            let message = format!("{error:#}");
            let mut policy = load_policy();
            policy.last_error = message.clone();
            let _ = save_policy(&policy);
            let _ = write_feedback(&format!("签发失败：{message}"), true);
            return Err(error);
        }
    };
    let keybox_path = produced
        .get("keybox_path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("引擎没有返回 keybox 路径"))?
        .to_string();
    let bytes = fs::read(&keybox_path).with_context(|| format!("读取 {keybox_path}"))?;
    if bytes.len() as u64 > KEYBOX_LIMIT {
        bail!("签发结果超过 {KEYBOX_LIMIT} 字节");
    }
    let installed = install_keybox(&level, &bytes).map_err(|error| {
        let message = format!("{error:#}");
        let mut policy = load_policy();
        policy.last_error = message.clone();
        let _ = save_policy(&policy);
        let _ = write_feedback(&format!("签发失败：{message}"), true);
        error
    })?;
    let mut policy = load_policy();
    policy.last_issue_ms = now_ms();
    policy.last_error.clear();
    save_policy(&policy)?;
    let _ = write_feedback(
        &format!("{} 级 keybox 已签发（{}）", level, meta.fingerprint),
        false,
    );
    Ok(json!({
        "issued": true,
        "level": level,
        "fingerprint": meta.fingerprint,
        "keybox_path": keybox_path,
        "chain_summary": produced.get("chain_summary").cloned().unwrap_or(Value::Null),
        "installed": installed,
        "policy": due_state(&policy),
    }))
}

fn import_seed(source: &str) -> Result<Value> {
    let bytes = read_limited(source, BUNDLE_LIMIT)?;
    let bundle: DuckBundle = serde_json::from_slice(&bytes).context("parse the material bundle")?;
    let (algorithm, fingerprint) = check_bundle(&bundle)?;
    // The bundle is stored verbatim so the engine sees exactly the bytes the
    // user imported; the plaintext never touches the filesystem.
    let mut payload = bytes.clone();
    let key = master_key(true)?.ok_or_else(|| anyhow!("master key unavailable"))?;
    let blob = encrypt_seed(&key, &payload)?;
    payload.fill(0);
    ensure_dir()?;
    write_private(SEED_BLOB, &blob)?;
    let digest = OmmegaSha256 {}
        .hash(&bytes)
        .map_err(|error| anyhow!("hash the bundle: {error:?}"))?;
    let meta = SeedMeta {
        fingerprint: if fingerprint.is_empty() {
            hex::encode(&digest[..8])
        } else {
            fingerprint
        },
        imported_at_ms: now_ms(),
        has_chain: true,
        source: source.to_string(),
        algorithm,
    };
    write_private(SEED_META, &serde_json::to_vec_pretty(&meta)?)?;
    Ok(json!({
        "saved": true,
        "fingerprint": meta.fingerprint,
        "algorithm": meta.algorithm,
        "has_chain": meta.has_chain,
        "seed": secret_summary(),
    }))
}

fn secret_summary() -> Value {
    let Some(meta) = load_meta() else {
        return json!({"present": false});
    };
    // Prove the blob still belongs to this install instead of only trusting the
    // metadata file: a restored backup from another device must show up as
    // undecryptable instead of failing at issue time.
    let decryptable = match (master_key(false), fs::read(SEED_BLOB)) {
        (Ok(Some(key)), Ok(blob)) => decrypt_seed(&key, &blob).is_ok(),
        _ => false,
    };
    json!({
        "present": true,
        "fingerprint": meta.fingerprint,
        "imported_at_ms": meta.imported_at_ms,
        "has_chain": meta.has_chain,
        "algorithm": meta.algorithm,
        "source": meta.source,
        "decryptable": decryptable,
    })
}

fn remove_seed() -> Result<Value> {
    let _ = fs::remove_file(SEED_BLOB);
    let _ = fs::remove_file(SEED_META);
    Ok(json!({"removed": true, "seed": {"present": false}}))
}

fn due_state(policy: &Policy) -> Value {
    let tee = keybox_status(KEYBOX_TEE, "TEE");
    let sb = keybox_status(KEYBOX_STRONGBOX, "StrongBox");
    let soonest = [&tee, &sb]
        .iter()
        .filter_map(|value| value.get("not_after_ms").and_then(Value::as_i64))
        .min();
    let due_at = soonest.map(|at| at - (policy.refresh_hours as i64) * 3_600_000);
    let due = due_at.map(|at| (now_ms() as i64) >= at).unwrap_or(false);
    json!({
        "auto": policy.auto,
        "refresh_hours": policy.refresh_hours,
        "due_at_ms": due_at,
        "due": due,
        "last_issue_ms": policy.last_issue_ms,
        "last_error": policy.last_error,
    })
}

fn status() -> Result<Value> {
    let policy = load_policy();
    let feedback: Option<Value> = fs::read(FEEDBACK)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    Ok(json!({
        "seed": secret_summary(),
        "tee": keybox_status(KEYBOX_TEE, "TEE"),
        "strongbox": keybox_status(KEYBOX_STRONGBOX, "StrongBox"),
        "policy": due_state(&policy),
        "last_feedback": feedback,
        "paths": {"tee": KEYBOX_TEE, "strongbox": KEYBOX_STRONGBOX, "state": STATE_DIR},
    }))
}

fn policy_command(args: &[String]) -> Result<Value> {
    let mut policy = load_policy();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--auto" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("--auto needs on|off"))?;
                policy.auto = matches!(value.as_str(), "on" | "true" | "1" | "yes");
                index += 2;
            }
            "--refresh-hours" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("--refresh-hours needs a number"))?;
                policy.refresh_hours = value.parse().context("refresh hours must be a number")?;
                index += 2;
            }
            other => bail!("unknown policy option {other}"),
        }
    }
    save_policy(&policy)?;
    Ok(due_state(&policy))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("status") => status(),
        Some("seed-import") => args
            .get(1)
            .ok_or_else(|| anyhow!("usage: seed-import <bundle.json>"))
            .and_then(|path| import_seed(path)),
        Some("seed-remove") => remove_seed(),
        Some("keybox-import") => import_keybox(&args[1..]),
        Some("policy") => policy_command(&args[1..]),
        Some("due") => Ok(due_state(&load_policy())),
        Some("issue") => issue(&args[1..]),
        _ => Err(anyhow!(
            "usage: ommega-rkp status|due|seed-import <json>|seed-remove|keybox-import <tee|strongbox> <xml>|policy [--auto on|off] [--refresh-hours N]"
        )),
    };
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    }
}
