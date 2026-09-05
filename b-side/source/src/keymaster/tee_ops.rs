//! Native relay operations on the selected B-side KeyMint HAL.
//!
//! Building on top of [`super::attest_proxy`], this module exposes the subset
//! of hardware operations used by the native relay protocol:
//!
//!   * generate an attestation/signing key (with an A-side supplied appid / 709)
//!   * derive a signing key attested by an attestation key
//!   * fetch the certificate chain / public key of a previously generated key
//!   * sign data / sign a to-be-signed (TBS) blob / sign a challenge
//!   * decrypt data / perform EC key agreement
//!
//! Key generation and private-key operations here use the selected real KeyMint
//! HAL. This does not move A-side storage, authorization or local software-key
//! operations to B. Generation translates some authorizations, including
//! ATTEST_KEY to SIGN and user-auth requirements to NO_AUTH_REQUIRED; it does
//! not provide hardware enforcement of the original A user's authentication.
//!
//! Sessions are cached by alias and persisted atomically. A hardware-requested
//! keyblob upgrade is saved before begin is retried; it never generates a new key.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use kmr_wire::{
    keymint::{
        Algorithm as KmAlgorithm, DateTime, Digest as KmDigest, EcCurve as KmEcCurve,
        ErrorCode as KmErrorCode, KeyParam, KeyPurpose as KmKeyPurpose, PaddingMode as KmPadding,
    },
    KeySizeInBits, RsaExponent,
};

/// Certificate validity bound (now). Real TEEs require NOT_BEFORE/NOT_AFTER
/// when minting an attestation key, otherwise generateKey fails with
/// MISSING_NOT_BEFORE (ErrorCode -80).
fn now_date_time() -> DateTime {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    DateTime {
        ms_since_epoch: now,
    }
}

/// Certificate validity bound (now + ~10 years).
fn after_date_time() -> DateTime {
    const TEN_YEARS_MS: i64 = 10 * 365 * 24 * 3600 * 1000;
    DateTime {
        ms_since_epoch: now_date_time().ms_since_epoch + TEN_YEARS_MS,
    }
}

use crate::android::hardware::security::keymint::KeyParameter::KeyParameter as KmKeyParameter;
use crate::android::hardware::security::keymint::KeyPurpose::KeyPurpose;
use crate::err as ks_err;
use crate::keymaster::relay_tee::{
    clear_system_keymint, extract_km_error_code, get_system_keymint, key_params_to_aidl,
    keymint_error, keymint_status_error, normalize_keymint_version, probe_keymint_version,
    KEY_MINT_V5,
};

use super::attest_proxy::{SYSTEM_KEYMINT_DEFAULT, SYSTEM_KEYMINT_STRONGBOX};

/// How a generated key is meant to be used.  Mirrors the `KeyPurpose`s the
/// legacy agent used. Only distinguishes EC vs RSA (used for begin()-parameter
/// construction); the actual size/curve is carried in [`KeySpec`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyAlgorithm {
    #[default]
    EcP256,
    Rsa2048,
}

/// Key parameters requested by the A-side app, forwarded from the A-side
/// KeyMint params. The real TEE mints a key matching these instead of a fixed
/// default. Absent fields fall back to the legacy defaults (EC P-256, SHA-256,
/// etc.).
#[derive(Clone, Debug, Default)]
pub struct KeySpec {
    /// EC vs RSA key family (also drives later begin() parameter construction).
    pub algorithm: KeyAlgorithm,
    /// EC curve; defaults to P256.
    pub ec_curve: Option<KmEcCurve>,
    /// Key size in bits; defaults to 256 (EC) / 2048 (RSA).
    pub key_size: Option<u32>,
    /// Business-key purposes are forwarded unchanged, including AgreeKey.
    /// ATTEST_KEY is translated to SIGN for the existing child-TBS signing path.
    pub purposes: Vec<KmKeyPurpose>,
    /// Requested digests. SHA-256 is added only for translated ATTEST_KEY keys.
    pub digests: Vec<KmDigest>,
    /// Requested MGF1 digest (RSA-OAEP), when the app specified one.
    pub mgf_digest: Option<KmDigest>,
    /// Requested paddings (RSA only), with signing padding for translated ATTEST_KEY.
    pub paddings: Vec<KmPadding>,
    /// RSA public exponent; defaults to 65537.
    pub rsa_public_exponent: Option<u64>,
    /// Certificate subject (DER-encoded X500Name); optional.
    pub cert_subject_der: Option<Vec<u8>>,
    /// Certificate validity bounds; default to now / now+10y.
    pub cert_not_before: Option<DateTime>,
    pub cert_not_after: Option<DateTime>,
    /// Certificate serial (A-side CERTIFICATE_SERIAL tag); optional.
    pub cert_serial: Option<Vec<u8>>,
}

/// A single generated key, held for the lifetime of the relay process.
#[derive(Clone, Debug)]
pub struct TeeSession {
    pub key_blob: Vec<u8>,
    pub cert_chain: Vec<Vec<u8>>,
    pub algorithm: KeyAlgorithm,
    /// Which KeyMint HAL service minted this key (`SYSTEM_KEYMINT_DEFAULT`
    /// for TEE, `SYSTEM_KEYMINT_STRONGBOX` for StrongBox).  Sign/decrypt
    /// operations must drive the *same* HAL or the key blob is rejected with
    /// `INVALID_KEY_BLOB` — StrongBox blobs are not usable in the TEE and
    /// vice versa.
    pub hal_service: &'static str,
    /// Canonical KeyMint version (100..500) used for parameter encoding.
    pub km_version: i32,
}

struct CachedSession {
    session: TeeSession,
    // Memory only. Do not discard a hardware-upgraded blob when a disk write
    // fails, but do not use it for an operation until persistence succeeds.
    needs_persistence: bool,
}

impl From<TeeSession> for CachedSession {
    fn from(session: TeeSession) -> Self {
        Self {
            session,
            needs_persistence: false,
        }
    }
}

/// Stable identity exposed by the real B-side KeyMint HAL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyMintIdentityProfile {
    pub interface_version: i32,
    pub interface_hash: String,
    pub profile_version: i32,
    pub hardware_version: i32,
    pub security_level: i32,
    pub keymint_name: String,
    pub keymint_author: String,
    pub has_strongbox: bool,
}

fn stable_aidl_version(
    keymint: &rsbinder::Strong<
        dyn crate::android::hardware::security::keymint::IKeyMintDevice::IKeyMintDevice,
    >,
) -> Result<i32> {
    let binder = keymint.as_binder();
    let proxy = binder
        .as_proxy()
        .ok_or_else(|| anyhow!("KeyMint service resolved to a local binder"))?;
    let data = proxy
        .prepare_transact(true)
        .context("prepare getInterfaceVersion transaction")?;
    let mut reply = proxy
        .submit_transact(
            rsbinder::FIRST_CALL_TRANSACTION + 16_777_214,
            &data,
            rsbinder::FLAG_PRIVATE_LOCAL | rsbinder::FLAG_CLEAR_BUF,
        )
        .context("submit getInterfaceVersion transaction")?
        .ok_or_else(|| anyhow!("getInterfaceVersion returned no parcel"))?;
    let status: rsbinder::Status = reply.read().context("read getInterfaceVersion status")?;
    if !status.is_ok() {
        return Err(anyhow!("getInterfaceVersion failed: {status}"));
    }
    reply.read().context("read getInterfaceVersion result")
}

fn stable_aidl_hash(
    keymint: &rsbinder::Strong<
        dyn crate::android::hardware::security::keymint::IKeyMintDevice::IKeyMintDevice,
    >,
) -> Result<String> {
    let binder = keymint.as_binder();
    let proxy = binder
        .as_proxy()
        .ok_or_else(|| anyhow!("KeyMint service resolved to a local binder"))?;
    let data = proxy
        .prepare_transact(true)
        .context("prepare getInterfaceHash transaction")?;
    let mut reply = proxy
        .submit_transact(
            rsbinder::FIRST_CALL_TRANSACTION + 16_777_213,
            &data,
            rsbinder::FLAG_PRIVATE_LOCAL | rsbinder::FLAG_CLEAR_BUF,
        )
        .context("submit getInterfaceHash transaction")?
        .ok_or_else(|| anyhow!("getInterfaceHash returned no parcel"))?;
    let status: rsbinder::Status = reply.read().context("read getInterfaceHash status")?;
    if !status.is_ok() {
        return Err(anyhow!("getInterfaceHash failed: {status}"));
    }
    reply.read().context("read getInterfaceHash result")
}

/// Reads the remote identity before A constructs its software KeyMint TA.
/// Values come from the same real HAL that later mints the attestation chain.
pub fn identity_profile() -> Result<KeyMintIdentityProfile> {
    static PROFILE: OnceLock<KeyMintIdentityProfile> = OnceLock::new();
    if let Some(profile) = PROFILE.get() {
        return Ok(profile.clone());
    }
    let keymint = get_system_keymint(SYSTEM_KEYMINT_DEFAULT)
        .context("connect default KeyMint for identity profile")?;
    let hardware = keymint
        .getHardwareInfo()
        .map_err(|status| anyhow!("default KeyMint getHardwareInfo failed: {status}"))?;
    let interface_version = stable_aidl_version(&keymint)?;
    let interface_hash = stable_aidl_hash(&keymint)?;
    let has_strongbox = get_system_keymint(SYSTEM_KEYMINT_STRONGBOX)
        .and_then(|strongbox| {
            strongbox
                .getHardwareInfo()
                .map_err(|status| anyhow!("StrongBox getHardwareInfo failed: {status}"))
        })
        .is_ok();

    let profile = KeyMintIdentityProfile {
        interface_version,
        interface_hash,
        profile_version: interface_version * 100,
        hardware_version: normalize_keymint_version(hardware.versionNumber),
        security_level: hardware.securityLevel.0,
        keymint_name: hardware.keyMintName,
        keymint_author: hardware.keyMintAuthorName,
        has_strongbox,
    };
    let _ = PROFILE.set(profile.clone());
    Ok(PROFILE.get().cloned().unwrap_or(profile))
}

/// Session persistence directory.  Key blobs minted by the real TEE are
/// self-contained and remain usable after a relay restart (begin/finish works
/// on the persisted blob), so we persist every generated session here to keep
/// the A-side `isRemote` keys usable across relay restarts.
fn sessions_dir() -> PathBuf {
    PathBuf::from("/data/adb/ommega/sessions")
}

fn ensure_sessions_dir() -> Result<PathBuf> {
    let path = sessions_dir();
    std::fs::create_dir_all(&path)
        .with_context(|| format!("create session directory {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("chmod session directory {}", path.display()))?;
    Ok(path)
}

/// Alias -> safe file stem.  Aliases can contain arbitrary UTF-8, so we keep
/// the printable prefix and append a short hash to guarantee uniqueness.
fn session_stem(alias: &str) -> String {
    let mut hasher = DefaultHasher::new();
    alias.hash(&mut hasher);
    let digest = hasher.finish();
    let safe: String = alias
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    format!("{safe}_{digest:016x}")
}

fn session_path(alias: &str) -> PathBuf {
    sessions_dir().join(format!("{}.json", session_stem(alias)))
}

fn load_session_from_disk(alias: &str) -> Option<TeeSession> {
    load_session_from_path(&session_path(alias))
}

fn load_session_from_path(path: &Path) -> Option<TeeSession> {
    let data = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&data).ok()?;
    let key_blob_b64 = value.get("key_blob")?.as_str()?;
    let key_blob = B64.decode(key_blob_b64).ok()?;
    let cert_chain = value
        .get("cert_chain")?
        .as_array()?
        .iter()
        .map(|c| B64.decode(c.as_str()?).ok())
        .collect::<Option<Vec<Vec<u8>>>>()?;
    let algorithm = match value.get("algorithm")?.as_str()? {
        "EcP256" => KeyAlgorithm::EcP256,
        "Rsa2048" => KeyAlgorithm::Rsa2048,
        _ => return None,
    };
    // `hal_service` was added later; old sessions without this field default
    // to the TEE HAL (the only service that existed at the time).
    let hal_service = match value.get("hal_service").and_then(|v| v.as_str()) {
        Some("strongbox") => SYSTEM_KEYMINT_STRONGBOX,
        _ => SYSTEM_KEYMINT_DEFAULT,
    };
    let km_version = value
        .get("km_version")
        .and_then(|value| value.as_i64())
        .map(|value| normalize_keymint_version(value as i32))
        .unwrap_or(KEY_MINT_V5);
    Some(TeeSession {
        key_blob,
        cert_chain,
        algorithm,
        hal_service,
        km_version,
    })
}

fn save_session_to_disk(alias: &str, session: &TeeSession) -> Result<()> {
    let path = session_path(alias);
    ensure_sessions_dir()?;
    save_session_at(&path, alias, session)
}

fn save_session_at(path: &Path, alias: &str, session: &TeeSession) -> Result<()> {
    let algorithm = match session.algorithm {
        KeyAlgorithm::EcP256 => "EcP256",
        KeyAlgorithm::Rsa2048 => "Rsa2048",
    };
    let hal_service_label = if session.hal_service == SYSTEM_KEYMINT_STRONGBOX {
        "strongbox"
    } else {
        "tee"
    };
    let value = serde_json::json!({
        "alias": alias,
        "key_blob": B64.encode(&session.key_blob),
        "cert_chain": session.cert_chain.iter().map(|c| B64.encode(c)).collect::<Vec<_>>(),
        "algorithm": algorithm,
        "hal_service": hal_service_label,
        "km_version": session.km_version,
    });
    static SAVE_COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = SAVE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_extension(format!("tmp-{}-{counter}", std::process::id()));
    let encoded = serde_json::to_vec(&value).context("serialize TEE session")?;
    let write_result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .with_context(|| format!("create temporary session {}", temp.display()))?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod temporary session {}", temp.display()))?;
        file.write_all(&encoded)
            .with_context(|| format!("write temporary session {}", temp.display()))?;
        file.sync_all()
            .with_context(|| format!("sync temporary session {}", temp.display()))?;
        std::fs::rename(&temp, path)
            .with_context(|| format!("replace session {}", path.display()))?;
        // The temp file already has mode 0600. Sync the rename as well as its
        // contents so a successful save survives a relay/device restart.
        let directory = path.parent().context("session path has no parent")?;
        std::fs::File::open(directory)
            .and_then(|directory| directory.sync_all())
            .with_context(|| format!("sync session directory {}", directory.display()))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    write_result
}

fn sessions() -> &'static Mutex<HashMap<String, CachedSession>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, CachedSession>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn session_put(alias: &str, session: TeeSession) -> Result<()> {
    // Serialize disk publication with upgrades and cache-miss recovery. An
    // older operation must not overwrite a newly generated key at this alias.
    let mut sessions = sessions().lock().unwrap();
    save_session_to_disk(alias, &session)?;
    sessions.insert(alias.to_string(), session.into());
    Ok(())
}

fn session_get(alias: &str) -> Result<TeeSession> {
    let mut sessions = sessions().lock().unwrap();
    if !sessions.contains_key(alias) {
        let session = load_session_from_disk(alias)
            .ok_or_else(|| anyhow!("no key for alias '{alias}' (call attest first)"))?;
        sessions.insert(alias.to_string(), session.into());
        log::info!("recovered persisted session for alias '{alias}'");
    }
    let cached = sessions.get_mut(alias).expect("session was loaded");
    persist_pending_session(cached, |session| save_session_to_disk(alias, session))?;
    Ok(cached.session.clone())
}

fn persist_pending_session(
    cached: &mut CachedSession,
    persist: impl FnOnce(&TeeSession) -> Result<()>,
) -> Result<()> {
    if cached.needs_persistence {
        persist(&cached.session)?;
        cached.needs_persistence = false;
    }
    Ok(())
}

fn recover_upgraded_session(
    cached: &mut CachedSession,
    observed: &TeeSession,
    upgrade: impl FnOnce(&[u8]) -> Result<Vec<u8>>,
    persist: impl FnOnce(&TeeSession) -> Result<()>,
) -> Result<TeeSession> {
    if cached.session.cert_chain != observed.cert_chain
        || cached.session.algorithm != observed.algorithm
        || cached.session.hal_service != observed.hal_service
        || cached.session.km_version != observed.km_version
    {
        return Err(keymint_status_error(
            &rsbinder::Status::new_service_specific_error(KmErrorCode::InvalidKeyBlob as i32, None),
            "session was replaced during keyblob upgrade",
        ));
    }
    // Another worker may have upgraded this same key while begin was running.
    if cached.session.key_blob == observed.key_blob && !cached.needs_persistence {
        let upgraded = upgrade(&cached.session.key_blob)?;
        if upgraded.is_empty() {
            return Err(anyhow!("real KeyMint upgradeKey returned an empty keyblob"));
        }
        cached.session.key_blob = upgraded;
        cached.needs_persistence = true;
    }
    persist_pending_session(cached, persist)?;
    Ok(cached.session.clone())
}

/// Loads every persisted session into memory.  Called once at startup so that
/// an alias generated before a relay restart is immediately usable.
pub fn load_all_sessions() {
    let Ok(directory) = ensure_sessions_dir() else {
        return;
    };
    let Some(entries) = std::fs::read_dir(directory).ok() else {
        return;
    };
    let mut loaded = 0usize;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") {
            continue;
        }
        // Recover the alias by scanning the file's JSON (we cannot reverse the
        // filename hash); read each file and store by its alias key.
        let Ok(data) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) else {
            continue;
        };
        let Some(alias) = value.get("alias").and_then(|a| a.as_str()) else {
            continue;
        };
        let key_blob_b64 = value.get("key_blob").and_then(|v| v.as_str());
        let Some(key_blob) = key_blob_b64.and_then(|s| B64.decode(s).ok()) else {
            continue;
        };
        let cert_chain = value
            .get("cert_chain")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c.as_str().and_then(|s| B64.decode(s).ok()))
                    .collect::<Vec<Vec<u8>>>()
            })
            .unwrap_or_default();
        let algorithm = match value.get("algorithm").and_then(|v| v.as_str()) {
            Some("Rsa2048") => KeyAlgorithm::Rsa2048,
            _ => KeyAlgorithm::EcP256,
        };
        let hal_service = match value.get("hal_service").and_then(|v| v.as_str()) {
            Some("strongbox") => SYSTEM_KEYMINT_STRONGBOX,
            _ => SYSTEM_KEYMINT_DEFAULT,
        };
        let km_version = value
            .get("km_version")
            .and_then(|value| value.as_i64())
            .map(|value| normalize_keymint_version(value as i32))
            .unwrap_or(KEY_MINT_V5);
        let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o600));
        sessions().lock().unwrap().insert(
            alias.to_string(),
            TeeSession {
                key_blob,
                cert_chain,
                algorithm,
                hal_service,
                km_version,
            }
            .into(),
        );
        loaded += 1;
    }
    if loaded > 0 {
        log::info!("loaded {loaded} persisted TEE sessions");
    }
}

// ---------------------------------------------------------------------------
// Key generation.
// ---------------------------------------------------------------------------

/// Generates a fresh key with the *real* TEE, embedding the A-side requested
/// `AttestationApplicationId` (tag 709) in the attestation extension of the
/// returned certificate chain. The key is minted to match the A-side requested
/// [`KeySpec`] (size/curve/purpose/digest/padding/subject/validity/serial).
pub fn generate_attest_key(
    alias: &str,
    challenge: &[u8],
    app_id_der: &[u8],
    spec: &KeySpec,
) -> Result<TeeSession> {
    generate_attest_key_on(SYSTEM_KEYMINT_DEFAULT, alias, challenge, app_id_der, spec)
}

/// Same as [`generate_attest_key`] but drives a caller-chosen real KeyMint HAL
/// service. Used to mint a StrongBox attestation via the B-side device's real
/// `/strongbox` HAL when the A-side requested `security_level=StrongBox`.
pub fn generate_attest_key_on(
    service: &'static str,
    alias: &str,
    challenge: &[u8],
    app_id_der: &[u8],
    spec: &KeySpec,
) -> Result<TeeSession> {
    let keymint = get_system_keymint(service)
        .with_context(|| ks_err!("real keymint {service} connect failed"))?;
    // Probe the actual HAL version instead of assuming V5. A StrongBox HAL
    // may only implement KeyMint V2/V3; sending V5-encoded parameters can
    // cause version-mismatch errors that look like "not supported".
    let km_version = probe_keymint_version(&keymint);
    let params = build_attestation_params(app_id_der, challenge, spec, km_version)?;

    let result = match keymint.generateKey(&params, None) {
        Ok(result) => result,
        Err(status) => {
            if is_dead_object_status(&status) {
                clear_system_keymint(service);
            }
            // Extract the KeyMint ErrorCode (e.g., -74, -68) so the caller
            // can distinguish "HAL not provisioned" from "parameter rejected".
            return Err(keymint_status_error(
                &status,
                &format!("real keymint {service} generateKey failed"),
            ));
        }
    };

    let cert_chain: Vec<Vec<u8>> = result
        .certificateChain
        .into_iter()
        .map(|cert| cert.encodedCertificate)
        .collect();

    let session = TeeSession {
        key_blob: result.keyBlob,
        cert_chain,
        algorithm: spec.algorithm,
        hal_service: service,
        km_version,
    };
    session_put(alias, session.clone())?;
    Ok(session)
}

fn build_attestation_params(
    app_id_der: &[u8],
    challenge: &[u8],
    spec: &KeySpec,
    km_version: i32,
) -> Result<Vec<KmKeyParameter>> {
    let (algo, default_size) = match spec.algorithm {
        KeyAlgorithm::EcP256 => (KmAlgorithm::Ec, 256u32),
        KeyAlgorithm::Rsa2048 => (KmAlgorithm::Rsa, 2048u32),
    };
    let key_size = spec.key_size.unwrap_or(default_size);
    let mut params = vec![
        KeyParam::Algorithm(algo),
        KeyParam::KeySize(KeySizeInBits(key_size)),
        KeyParam::AttestationChallenge(challenge.to_vec()),
        KeyParam::AttestationApplicationId(app_id_der.to_vec()),
    ];

    // Forward business-key purposes exactly. The only unavoidable translation
    // is ATTEST_KEY -> SIGN: this qti TEE rejects direct ATTEST_KEY generation,
    // while the A-side later asks this key to sign a child certificate TBS.
    let mut purposes = spec.purposes.clone();
    let is_attest_key = purposes.contains(&KmKeyPurpose::AttestKey);
    if is_attest_key {
        purposes.retain(|p| *p != KmKeyPurpose::AttestKey);
        if !purposes.contains(&KmKeyPurpose::Sign) {
            purposes.push(KmKeyPurpose::Sign);
        }
    }
    for p in purposes {
        params.push(KeyParam::Purpose(p));
    }

    // Forward business-key digests exactly. A translated ATTEST_KEY needs
    // SHA-256 because child certificates are signed with SHA256withRSA/ECDSA.
    let mut digests = spec.digests.clone();
    if is_attest_key && !digests.contains(&KmDigest::Sha256) {
        digests.push(KmDigest::Sha256);
    }
    for d in digests {
        params.push(KeyParam::Digest(d));
    }

    // Remote-proxy model: the B-side signs on behalf of the A-side, but the
    // A-side's auth token cannot reach the B-side real TEE.  Without
    // NO_AUTH_REQUIRED the TEE mints a user-auth-gated key and every relay
    // begin(SIGN) fails with KEY_USER_NOT_AUTHENTICATED (-26).  The relay
    // always signs without a token, so the key must be usable without auth.
    params.push(KeyParam::NoAuthRequired);

    match algo {
        KmAlgorithm::Rsa => {
            // Real keymint requires RSA_PUBLIC_EXPONENT on every RSA key;
            // without it generateKey fails with INVALID_ARGUMENT.
            params.push(KeyParam::RsaPublicExponent(RsaExponent(
                spec.rsa_public_exponent.unwrap_or(65537),
            )));
            // Padding set: only the app's requested paddings. We used to force
            // all four RSA paddings (PKCS1Sign/PSS/PKCS1Encrypt/OAEP) so the
            // relay's own operations could always begin(), but that also lets
            // *unauthorized* uses succeed, which a probe like DuckDetector's
            // "RSA-PSS key must reject PKCS1 sign" rejects. The relay's begin
            // padding always matches the app's (it derives from the A-side
            // algorithm string, which derives from the app's begin params), so
            // only the requested paddings are needed.
            for p in spec.paddings.iter() {
                params.push(KeyParam::Padding(*p));
            }
            // An attestation key signs the child-cert TBS with PKCS1v15
            // (SHA256withRSA); ensure that padding is authorized too, even when
            // the app only requested ATTEST_KEY use.
            if is_attest_key && !spec.paddings.contains(&KmPadding::RsaPkcs115Sign) {
                params.push(KeyParam::Padding(KmPadding::RsaPkcs115Sign));
            }
            // MGF1 digest for RSA-OAEP. Always push an explicit MGF1 digest so
            // the TEE's key authorization carries the tag. begin(DECRYPT)
            // always sends RsaOaepMgfDigest (see decrypt_begin_params; SHA1 is
            // the default when the app didn't set one). If the authorization
            // omits the tag, the real TEE rejects the operation with
            // INCOMPATIBLE_MGF_DIGEST (-78), whereas a local software keymint
            // would never check it.
            if spec.paddings.contains(&KmPadding::RsaOaep) {
                let mgf = spec.mgf_digest.unwrap_or(KmDigest::Sha1);
                params.push(KeyParam::RsaOaepMgfDigest(mgf));
            }
        }
        KmAlgorithm::Ec => {
            // The real TEE requires an explicit curve for EC keys; without it
            // generateKey fails with UNSUPPORTED_KEY_SIZE (ErrorCode -6).
            params.push(KeyParam::EcCurve(spec.ec_curve.unwrap_or(KmEcCurve::P256)));
        }
        _ => {}
    }

    // Certificate validity bounds (app-specified or default now / now+10y);
    // real TEEs require NOT_BEFORE/NOT_AFTER (else MISSING_NOT_BEFORE -80).
    params.push(KeyParam::CertificateNotBefore(
        spec.cert_not_before.unwrap_or_else(now_date_time),
    ));
    params.push(KeyParam::CertificateNotAfter(
        spec.cert_not_after.unwrap_or_else(after_date_time),
    ));

    // Certificate subject (DER X500Name), when the app requested one.
    if let Some(subject) = &spec.cert_subject_der {
        params.push(KeyParam::CertificateSubject(subject.clone()));
    }
    // A-side requested certificate serial (CERTIFICATE_SERIAL tag). When
    // absent the TEE mints a random 16-byte serial (looks like garbage to
    // the A-side); passing it through makes the leaf serial deterministic.
    if let Some(serial) = &spec.cert_serial {
        params.push(KeyParam::CertificateSerial(serial.clone()));
    }
    key_params_to_aidl(&params, km_version)
        .with_context(|| ks_err!("encode real TEE attestation parameters"))
}

// ---------------------------------------------------------------------------
// Read-only helpers.
// ---------------------------------------------------------------------------

/// Returns the certificate chain (DER) for `alias`, leaf first.
pub fn get_cert_chain(alias: &str) -> Result<Vec<Vec<u8>>> {
    Ok(session_get(alias)?.cert_chain)
}

/// Returns the SubjectPublicKeyInfo (SPKI, DER) of the leaf certificate.
pub fn get_public_key(alias: &str) -> Result<Vec<u8>> {
    let session = session_get(alias)?;
    let leaf = session
        .cert_chain
        .first()
        .ok_or_else(|| anyhow!("certificate chain empty for '{alias}'"))?;
    spki_from_cert_der(leaf)
}

// ---------------------------------------------------------------------------
// Sign / decrypt / key-agreement operations (real TEE begin/update/finish).
// ---------------------------------------------------------------------------

/// Signs `data` with the TEE key for `alias`.
pub fn sign(alias: &str, data: &[u8], algorithm: &str) -> Result<Vec<u8>> {
    let session = session_get(alias)?;
    let op_params = sign_begin_params(algorithm, session.algorithm, session.km_version)
        .with_context(|| ks_err!("unsupported sign algorithm {algorithm}"))?;
    run_single_input_op(alias, &session, KeyPurpose::SIGN, &op_params, data)
}

/// Decrypts `data` with the TEE key for `alias`.
pub fn decrypt(alias: &str, data: &[u8], algorithm: &str) -> Result<Vec<u8>> {
    let session = session_get(alias)?;
    let op_params = decrypt_begin_params(algorithm, session.algorithm, session.km_version)
        .with_context(|| ks_err!("unsupported decrypt algorithm {algorithm}"))?;
    run_single_input_op(alias, &session, KeyPurpose::DECRYPT, &op_params, data)
}

/// Performs EC key agreement with the TEE key for `alias`. `peer_public_key`
/// must be a DER SubjectPublicKeyInfo, matching the KeyMint operation contract.
pub fn agree_key(alias: &str, peer_public_key: &[u8]) -> Result<Vec<u8>> {
    // Validate before session lookup (which can persist a pending upgrade),
    // and before opening a Binder operation.
    let peer_curve = agreement_peer_curve(peer_public_key)?;
    let session = session_get(alias)?;
    if session.algorithm != KeyAlgorithm::EcP256 {
        return Err(keymint_error(
            KmErrorCode::IncompatibleAlgorithm,
            "agreement requires an EC key",
        ));
    }
    let public_key = session
        .cert_chain
        .first()
        .ok_or_else(|| {
            keymint_error(
                KmErrorCode::InvalidKeyBlob,
                "agreement key has no certificate",
            )
        })
        .and_then(|leaf| {
            spki_from_cert_der(leaf).map_err(|_| {
                keymint_error(
                    KmErrorCode::InvalidKeyBlob,
                    "invalid agreement key certificate",
                )
            })
        })?;
    let local_curve = agreement_peer_curve(&public_key)
        .map_err(|_| keymint_error(KmErrorCode::InvalidKeyBlob, "invalid agreement key SPKI"))?;
    if peer_curve != local_curve {
        return Err(keymint_error(
            KmErrorCode::InvalidArgument,
            "agreement curve mismatch",
        ));
    }
    run_single_input_op(alias, &session, KeyPurpose::AGREE_KEY, &[], peer_public_key)
}

/// Check DER, algorithm, curve and point encoding only. The HAL still validates
/// the public point and the key's authorized purposes. No private key is read.
fn agreement_peer_curve(input: &[u8]) -> Result<KmEcCurve> {
    use der::Decode as _;
    use kmr_common::crypto::ec;
    if input.len() > 164 {
        return Err(keymint_error(
            KmErrorCode::InvalidInputLength,
            "agreement SPKI exceeds 164 bytes",
        ));
    }
    let invalid = || keymint_error(KmErrorCode::InvalidArgument, "invalid agreement peer SPKI");
    let spki = x509_cert::spki::SubjectPublicKeyInfoRef::from_der(input).map_err(|_| invalid())?;
    let point = spki.subject_public_key.as_bytes().ok_or_else(invalid)?;
    if spki.algorithm.oid == ec::X509_X25519_OID {
        if spki.algorithm.parameters.is_some() || point.len() != 32 {
            return Err(invalid());
        }
        return Ok(KmEcCurve::Curve25519);
    }
    if spki.algorithm.oid != ec::X509_NIST_OID {
        return Err(invalid());
    }
    let oid = spki
        .algorithm
        .parameters
        .ok_or_else(invalid)?
        .decode_as::<der::asn1::ObjectIdentifier>()
        .map_err(|_| invalid())?;
    let (curve, size) = match oid {
        ec::ALGO_PARAM_P224_OID => (KmEcCurve::P224, 28),
        ec::ALGO_PARAM_P256_OID => (KmEcCurve::P256, 32),
        ec::ALGO_PARAM_P384_OID => (KmEcCurve::P384, 48),
        ec::ALGO_PARAM_P521_OID => (KmEcCurve::P521, 66),
        _ => return Err(invalid()),
    };
    let valid = match point.first() {
        Some(4) => point.len() == 1 + 2 * size,
        Some(2 | 3) => point.len() == 1 + size,
        _ => false,
    };
    if !valid {
        return Err(invalid());
    }
    Ok(curve)
}

fn begin_with_upgrade_retry<T>(
    session: &TeeSession,
    mut begin: impl FnMut(&[u8]) -> rsbinder::status::Result<T>,
    recover: impl FnOnce() -> Result<TeeSession>,
) -> Result<T> {
    let result = match begin(&session.key_blob) {
        Err(status)
            if extract_km_error_code(&status) == Some(KmErrorCode::KeyRequiresUpgrade as i32) =>
        {
            let upgraded = recover()?;
            // Exactly one retry. No update/finish input has been submitted yet.
            begin(&upgraded.key_blob)
        }
        result => result,
    };
    result.map_err(|status| {
        if is_dead_object_status(&status) {
            clear_system_keymint(session.hal_service);
        }
        keymint_status_error(
            &status,
            &format!("real keymint {} begin failed", session.hal_service),
        )
    })
}

fn run_single_input_op(
    alias: &str,
    session: &TeeSession,
    purpose: KeyPurpose,
    op_params: &[KmKeyParameter],
    input: &[u8],
) -> Result<Vec<u8>> {
    let hal_service = session.hal_service;
    let keymint = get_system_keymint(hal_service)
        .with_context(|| ks_err!("real keymint {hal_service} connect failed"))?;

    let begin = begin_with_upgrade_retry(
        session,
        |blob| keymint.begin(purpose, blob, op_params, None),
        || {
            // Only recovery holds this lock over a HAL call. Ordinary crypto
            // operations still run concurrently without the session-table lock.
            let mut sessions = sessions().lock().unwrap();
            let cached = sessions
                .get_mut(alias)
                .context("upgrade session is missing")?;
            let upgraded = recover_upgraded_session(
                cached,
                session,
                |blob| {
                    // Relay-generated keys have no APPLICATION_ID/DATA binding.
                    // ATTESTATION_APPLICATION_ID is a separate, non-hidden tag.
                    keymint.upgradeKey(blob, &[]).map_err(|status| {
                        if is_dead_object_status(&status) {
                            clear_system_keymint(hal_service);
                        }
                        keymint_status_error(
                            &status,
                            &format!("real keymint {hal_service} upgradeKey failed"),
                        )
                    })
                },
                |session| save_session_to_disk(alias, session),
            )?;
            log::info!("event=keyblob_upgrade_ready alias={alias} hal={hal_service}");
            Ok(upgraded)
        },
    )?;

    let Some(operation) = begin.operation else {
        return Err(anyhow!(
            "real keymint {hal_service} begin returned no operation"
        ));
    };

    complete_single_input_op(
        hal_service,
        purpose,
        input,
        |data| operation.update(data, None, None),
        |data| operation.finish(data, None, None, None, None),
        || {
            let _ = operation.r#abort();
        },
    )
}

fn complete_single_input_op(
    hal_service: &str,
    purpose: KeyPurpose,
    input: &[u8],
    update: impl FnOnce(&[u8]) -> rsbinder::status::Result<Vec<u8>>,
    finish: impl FnOnce(Option<&[u8]>) -> rsbinder::status::Result<Vec<u8>>,
    abort: impl FnOnce(),
) -> Result<Vec<u8>> {
    // Agreement follows Android's one-shot finish(peer SPKI) path. It has no
    // streaming output. Sign/decrypt retain their existing update + finish path.
    // update() may
    // return output early (e.g. a single-block RSA decrypt can deliver the
    // plaintext from update); finish() then returns whatever is left, so both
    // outputs must be concatenated or the operation's result is silently lost.
    let result = (|| -> Result<Vec<u8>> {
        if purpose == KeyPurpose::AGREE_KEY {
            return finish(Some(input)).map_err(|status| {
                keymint_status_error(
                    &status,
                    &format!("real keymint {hal_service} finish failed"),
                )
            });
        }
        let mut out = update(input).map_err(|status| {
            keymint_status_error(
                &status,
                &format!("real keymint {hal_service} update failed"),
            )
        })?;
        out.extend_from_slice(&finish(None).map_err(|status| {
            keymint_status_error(
                &status,
                &format!("real keymint {hal_service} finish failed"),
            )
        })?);
        Ok(out)
    })();

    if result.is_err() {
        abort();
    }
    result
}

// ---------------------------------------------------------------------------
// Begin-parameter builders.
// ---------------------------------------------------------------------------

fn sign_begin_params(
    algorithm: &str,
    key_algorithm: KeyAlgorithm,
    km_version: i32,
) -> Result<Vec<KmKeyParameter>> {
    let params = sign_key_params(algorithm, key_algorithm)?;
    key_params_to_aidl(&params, km_version).with_context(|| ks_err!("encode sign begin parameters"))
}

fn sign_key_params(algorithm: &str, key_algorithm: KeyAlgorithm) -> Result<Vec<KeyParam>> {
    let digest = digest_for_algorithm(algorithm)?;
    let normalized = algorithm.trim().to_ascii_uppercase();
    let scheme = normalized
        .split_once("WITH")
        .map(|(_, scheme)| scheme)
        .ok_or_else(|| {
            keymint_error(
                KmErrorCode::IncompatibleAlgorithm,
                format!("unsupported sign algorithm: {algorithm}"),
            )
        })?;
    Ok(match key_algorithm {
        KeyAlgorithm::EcP256 => {
            if scheme != "ECDSA" {
                return Err(keymint_error(
                    KmErrorCode::IncompatibleAlgorithm,
                    format!("EC key cannot use sign algorithm: {algorithm}"),
                ));
            }
            vec![KeyParam::Digest(digest)]
        }
        KeyAlgorithm::Rsa2048 => {
            let padding = if scheme == "RSA/NOPADDING" {
                if digest != KmDigest::None {
                    return Err(keymint_error(
                        KmErrorCode::IncompatibleDigest,
                        format!("RSA NoPadding requires Digest::NONE: {algorithm}"),
                    ));
                }
                KmPadding::None
            } else if scheme == "RSA/PSS" {
                if digest == KmDigest::None {
                    return Err(keymint_error(
                        KmErrorCode::UnsupportedDigest,
                        format!("RSA-PSS does not support Digest::NONE: {algorithm}"),
                    ));
                }
                KmPadding::RsaPss
            } else if scheme == "RSA" {
                KmPadding::RsaPkcs115Sign
            } else if scheme.starts_with("RSA/") {
                return Err(keymint_error(
                    KmErrorCode::UnsupportedPaddingMode,
                    format!("unsupported RSA sign algorithm: {algorithm}"),
                ));
            } else {
                return Err(keymint_error(
                    KmErrorCode::IncompatibleAlgorithm,
                    format!("RSA key cannot use sign algorithm: {algorithm}"),
                ));
            };
            let mut p = vec![KeyParam::Digest(digest), KeyParam::Padding(padding)];
            if padding == KmPadding::RsaPss {
                // Real TEEs require the MGF digest for PSS; without it
                // begin(SIGN) fails with INCOMPATIBLE_MGF_DIGEST. PSS uses the
                // message digest as its MGF digest.
                p.push(KeyParam::RsaOaepMgfDigest(digest));
            }
            p
        }
    })
}

fn decrypt_begin_params(
    algorithm: &str,
    key_algorithm: KeyAlgorithm,
    km_version: i32,
) -> Result<Vec<KmKeyParameter>> {
    let params = decrypt_key_params(algorithm, key_algorithm)?;
    key_params_to_aidl(&params, km_version)
        .with_context(|| ks_err!("encode decrypt begin parameters"))
}

fn decrypt_key_params(algorithm: &str, key_algorithm: KeyAlgorithm) -> Result<Vec<KeyParam>> {
    if key_algorithm == KeyAlgorithm::EcP256 {
        return Err(keymint_error(
            KmErrorCode::IncompatiblePurpose,
            "EC keys cannot be used for decrypt",
        ));
    }

    let normalized = algorithm.trim().to_ascii_uppercase();
    if normalized.starts_with("RSA/OAEP/") {
        let digest = digest_for_algorithm(algorithm)?;
        let mgf = mgf_digest_for_algorithm(algorithm)?;
        // OAEP requires both the digest and the MGF digest at begin(DECRYPT),
        // and the MGF digest must match the one the encryptor requested.
        Ok(vec![
            KeyParam::Padding(KmPadding::RsaOaep),
            KeyParam::Digest(digest),
            KeyParam::RsaOaepMgfDigest(mgf),
        ])
    } else if normalized == "RSA/ECB/NOPADDING" {
        Ok(vec![KeyParam::Padding(KmPadding::None)])
    } else if normalized == "RSA/ECB/PKCS1PADDING" {
        Ok(vec![KeyParam::Padding(KmPadding::RsaPkcs115Encrypt)])
    } else {
        Err(keymint_error(
            KmErrorCode::UnsupportedPaddingMode,
            format!("unsupported RSA decrypt algorithm: {algorithm}"),
        ))
    }
}

/// Parses the MGF1 digest from an OAEP algorithm string like
/// `RSA/OAEP/SHA-256/MGF1-SHA1`. Defaults to SHA1 (the standard OAEP default
/// when no MGF1 is specified).
fn mgf_digest_for_algorithm(algorithm: &str) -> Result<KmDigest> {
    let up = algorithm.to_uppercase();
    if let Some(pos) = up.find("/MGF1-") {
        let token = &up[pos + 6..];
        let digest = parse_digest_token(token).ok_or_else(|| {
            keymint_error(
                KmErrorCode::UnsupportedMgfDigest,
                format!("unsupported OAEP MGF digest in algorithm: {algorithm}"),
            )
        })?;
        if digest == KmDigest::None {
            return Err(keymint_error(
                KmErrorCode::UnsupportedMgfDigest,
                format!("OAEP MGF digest cannot be NONE: {algorithm}"),
            ));
        }
        return Ok(digest);
    }
    Ok(KmDigest::Sha1)
}

fn digest_for_algorithm(algorithm: &str) -> Result<KmDigest> {
    let up = algorithm.trim().to_ascii_uppercase();
    let token = if let Some(rest) = up.strip_prefix("RSA/OAEP/") {
        rest.split('/').next()
    } else {
        up.split_once("WITH").map(|(digest, _)| digest)
    };
    token.and_then(parse_digest_token).ok_or_else(|| {
        keymint_error(
            KmErrorCode::UnsupportedDigest,
            format!("unsupported digest algorithm: {algorithm}"),
        )
    })
}

fn parse_digest_token(token: &str) -> Option<KmDigest> {
    let normalized = token.trim().replace('-', "");
    match normalized.as_str() {
        "NONE" => Some(KmDigest::None),
        "MD5" => Some(KmDigest::Md5),
        "SHA1" => Some(KmDigest::Sha1),
        "SHA224" => Some(KmDigest::Sha224),
        "SHA256" => Some(KmDigest::Sha256),
        "SHA384" => Some(KmDigest::Sha384),
        "SHA512" => Some(KmDigest::Sha512),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// X.509 helpers.
// ---------------------------------------------------------------------------

fn spki_from_cert_der(der: &[u8]) -> Result<Vec<u8>> {
    use x509_cert::{der::Decode as _, der::Encode as _, Certificate};
    let cert = Certificate::from_der(der).with_context(|| ks_err!("parse leaf certificate"))?;
    cert.tbs_certificate()
        .subject_public_key_info()
        .to_der()
        .with_context(|| ks_err!("encode subject public key info"))
}

fn is_dead_object_status(status: &rsbinder::Status) -> bool {
    status.exception_code() == rsbinder::ExceptionCode::TransactionFailed
        && status.transaction_error() == rsbinder::StatusCode::DeadObject
}

#[cfg(test)]
mod identity_profile_tests {
    use super::*;

    fn generate_ephemeral_session(mut params: Vec<KeyParam>) -> TeeSession {
        let algorithm = if params.contains(&KeyParam::Algorithm(KmAlgorithm::Rsa)) {
            KeyAlgorithm::Rsa2048
        } else {
            KeyAlgorithm::EcP256
        };
        let keymint = get_system_keymint(SYSTEM_KEYMINT_DEFAULT).unwrap();
        let km_version = probe_keymint_version(&keymint);
        params.extend([
            KeyParam::NoAuthRequired,
            KeyParam::CertificateNotBefore(now_date_time()),
            KeyParam::CertificateNotAfter(after_date_time()),
        ]);
        let params = key_params_to_aidl(&params, km_version).unwrap();
        let result = keymint.generateKey(&params, None).unwrap_or_else(|status| {
            panic!(
                "{}",
                keymint_status_error(&status, "ephemeral capability generateKey failed")
            )
        });
        TeeSession {
            key_blob: result.keyBlob,
            cert_chain: result
                .certificateChain
                .into_iter()
                .map(|cert| cert.encodedCertificate)
                .collect(),
            algorithm,
            hal_service: SYSTEM_KEYMINT_DEFAULT,
            km_version,
        }
    }

    fn generate_ephemeral_key(params: Vec<KeyParam>) -> (Vec<u8>, i32) {
        let session = generate_ephemeral_session(params);
        (session.key_blob, session.km_version)
    }

    fn begin_ephemeral_key(
        key_blob: &[u8],
        km_version: i32,
        purpose: KeyPurpose,
        params: Vec<KeyParam>,
    ) -> rsbinder::Strong<
        dyn crate::android::hardware::security::keymint::IKeyMintOperation::IKeyMintOperation,
    > {
        let keymint = get_system_keymint(SYSTEM_KEYMINT_DEFAULT).unwrap();
        let params = key_params_to_aidl(&params, km_version).unwrap();
        let result = keymint
            .begin(purpose, key_blob, &params, None)
            .unwrap_or_else(|status| {
                panic!(
                    "{}",
                    keymint_status_error(&status, "ephemeral capability begin failed")
                )
            });
        result
            .operation
            .expect("capability begin returned no operation")
    }

    fn run_ephemeral_key(
        key_blob: &[u8],
        km_version: i32,
        purpose: KeyPurpose,
        params: Vec<KeyParam>,
        input: &[u8],
    ) -> Vec<u8> {
        let operation = begin_ephemeral_key(key_blob, km_version, purpose, params);
        let mut output = operation
            .update(input, None, None)
            .unwrap_or_else(|status| {
                panic!(
                    "{}",
                    keymint_status_error(&status, "ephemeral capability update failed")
                )
            });
        output.extend_from_slice(
            &operation
                .finish(None, None, None, None, None)
                .unwrap_or_else(|status| {
                    panic!(
                        "{}",
                        keymint_status_error(&status, "ephemeral capability finish failed")
                    )
                }),
        );
        output
    }

    #[test]
    #[ignore = "requires a connected Android device with a real KeyMint HAL"]
    fn live_default_keymint_profile_is_self_consistent() {
        rsbinder::ProcessState::init_default().expect("initialize Binder process state");
        let profile = identity_profile().expect("read real KeyMint identity profile");
        println!("profile={profile:?}");
        assert_eq!(profile.security_level, 1);
        assert_eq!(profile.profile_version, profile.interface_version * 100);
        assert!(matches!(
            profile.hardware_version,
            100 | 200 | 300 | 400 | 500
        ));
        assert_eq!(profile.interface_hash.len(), 40);
        assert!(profile
            .interface_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
        assert!(!profile.keymint_name.trim().is_empty());
    }

    #[test]
    #[ignore = "requires a connected Android device with a real KeyMint HAL"]
    fn live_default_keymint_algorithm_capabilities() {
        rsbinder::ProcessState::init_default().expect("initialize Binder process state");

        let (raw_rsa, km_version) = generate_ephemeral_key(vec![
            KeyParam::Algorithm(KmAlgorithm::Rsa),
            KeyParam::KeySize(KeySizeInBits(2048)),
            KeyParam::RsaPublicExponent(RsaExponent(65537)),
            KeyParam::Purpose(KmKeyPurpose::Sign),
            KeyParam::Purpose(KmKeyPurpose::Decrypt),
            KeyParam::Digest(KmDigest::None),
            KeyParam::Padding(KmPadding::None),
        ]);
        let raw_signature = run_ephemeral_key(
            &raw_rsa,
            km_version,
            KeyPurpose::SIGN,
            sign_key_params("NONEwithRSA/NoPadding", KeyAlgorithm::Rsa2048).unwrap(),
            b"raw-rsa",
        );
        assert_eq!(raw_signature.len(), 256);
        let raw_plaintext = run_ephemeral_key(
            &raw_rsa,
            km_version,
            KeyPurpose::DECRYPT,
            decrypt_key_params("RSA/ECB/NoPadding", KeyAlgorithm::Rsa2048).unwrap(),
            &[0; 256],
        );
        assert!(!raw_plaintext.is_empty());

        let (sha224_rsa, km_version) = generate_ephemeral_key(vec![
            KeyParam::Algorithm(KmAlgorithm::Rsa),
            KeyParam::KeySize(KeySizeInBits(2048)),
            KeyParam::RsaPublicExponent(RsaExponent(65537)),
            KeyParam::Purpose(KmKeyPurpose::Sign),
            KeyParam::Digest(KmDigest::Sha224),
            KeyParam::Padding(KmPadding::RsaPkcs115Sign),
            KeyParam::Padding(KmPadding::RsaPss),
        ]);
        for algorithm in ["SHA224withRSA", "SHA224withRSA/PSS"] {
            let signature = run_ephemeral_key(
                &sha224_rsa,
                km_version,
                KeyPurpose::SIGN,
                sign_key_params(algorithm, KeyAlgorithm::Rsa2048).unwrap(),
                b"sha224-rsa",
            );
            assert_eq!(signature.len(), 256, "algorithm={algorithm}");
        }

        let (md5_rsa, km_version) = generate_ephemeral_key(vec![
            KeyParam::Algorithm(KmAlgorithm::Rsa),
            KeyParam::KeySize(KeySizeInBits(2048)),
            KeyParam::RsaPublicExponent(RsaExponent(65537)),
            KeyParam::Purpose(KmKeyPurpose::Sign),
            KeyParam::Digest(KmDigest::Md5),
            KeyParam::Padding(KmPadding::RsaPkcs115Sign),
            KeyParam::Padding(KmPadding::RsaPss),
        ]);
        for algorithm in ["MD5withRSA", "MD5withRSA/PSS"] {
            let signature = run_ephemeral_key(
                &md5_rsa,
                km_version,
                KeyPurpose::SIGN,
                sign_key_params(algorithm, KeyAlgorithm::Rsa2048).unwrap(),
                b"md5-rsa",
            );
            assert_eq!(signature.len(), 256, "algorithm={algorithm}");
        }

        let (sha224_oaep, km_version) = generate_ephemeral_key(vec![
            KeyParam::Algorithm(KmAlgorithm::Rsa),
            KeyParam::KeySize(KeySizeInBits(2048)),
            KeyParam::RsaPublicExponent(RsaExponent(65537)),
            KeyParam::Purpose(KmKeyPurpose::Decrypt),
            KeyParam::Digest(KmDigest::Sha224),
            KeyParam::Padding(KmPadding::RsaOaep),
            KeyParam::RsaOaepMgfDigest(KmDigest::Sha224),
        ]);
        let operation = begin_ephemeral_key(
            &sha224_oaep,
            km_version,
            KeyPurpose::DECRYPT,
            decrypt_key_params("RSA/OAEP/SHA-224/MGF1-SHA224", KeyAlgorithm::Rsa2048).unwrap(),
        );
        operation.r#abort().unwrap();

        let (md5_oaep, km_version) = generate_ephemeral_key(vec![
            KeyParam::Algorithm(KmAlgorithm::Rsa),
            KeyParam::KeySize(KeySizeInBits(2048)),
            KeyParam::RsaPublicExponent(RsaExponent(65537)),
            KeyParam::Purpose(KmKeyPurpose::Decrypt),
            KeyParam::Digest(KmDigest::Md5),
            KeyParam::Padding(KmPadding::RsaOaep),
            KeyParam::RsaOaepMgfDigest(KmDigest::Md5),
        ]);
        let operation = begin_ephemeral_key(
            &md5_oaep,
            km_version,
            KeyPurpose::DECRYPT,
            decrypt_key_params("RSA/OAEP/MD5/MGF1-MD5", KeyAlgorithm::Rsa2048).unwrap(),
        );
        operation.r#abort().unwrap();

        let (ec, km_version) = generate_ephemeral_key(vec![
            KeyParam::Algorithm(KmAlgorithm::Ec),
            KeyParam::KeySize(KeySizeInBits(256)),
            KeyParam::EcCurve(KmEcCurve::P256),
            KeyParam::Purpose(KmKeyPurpose::Sign),
            KeyParam::Digest(KmDigest::None),
            KeyParam::Digest(KmDigest::Sha224),
        ]);
        for algorithm in ["NONEwithECDSA", "SHA224withECDSA"] {
            let signature = run_ephemeral_key(
                &ec,
                km_version,
                KeyPurpose::SIGN,
                sign_key_params(algorithm, KeyAlgorithm::EcP256).unwrap(),
                b"ec-sign",
            );
            assert!(!signature.is_empty(), "algorithm={algorithm}");
        }
    }
}

#[cfg(test)]
mod agreement_tests {
    use super::*;
    use crate::keymaster::relay_tee::relay_error_result;
    use der::{
        asn1::{BitStringRef, ObjectIdentifier},
        AnyRef, Encode,
    };
    use kmr_common::crypto::ec;
    use std::cell::Cell;

    fn spki(oid: ObjectIdentifier, curve: Option<ObjectIdentifier>, point: &[u8]) -> Vec<u8> {
        x509_cert::spki::SubjectPublicKeyInfoRef {
            algorithm: x509_cert::spki::AlgorithmIdentifierRef {
                oid,
                parameters: curve.as_ref().map(AnyRef::from),
            },
            subject_public_key: BitStringRef::from_bytes(point).unwrap(),
        }
        .to_der()
        .unwrap()
    }

    #[test]
    fn peer_spki_checks_encoding_without_hal() {
        for (oid, curve, size) in [
            (ec::ALGO_PARAM_P224_OID, KmEcCurve::P224, 28),
            (ec::ALGO_PARAM_P256_OID, KmEcCurve::P256, 32),
            (ec::ALGO_PARAM_P384_OID, KmEcCurve::P384, 48),
            (ec::ALGO_PARAM_P521_OID, KmEcCurve::P521, 66),
        ] {
            // Synthetic points exercise encoding only, never sent to a HAL.
            let mut point = vec![1; 1 + 2 * size];
            point[0] = 4;
            let encoded = spki(ec::X509_NIST_OID, Some(oid), &point);
            assert_eq!(agreement_peer_curve(&encoded).unwrap(), curve);
            let mut trailing = encoded.clone();
            trailing.push(0);
            assert!(agreement_peer_curve(&trailing).is_err());
            assert!(agreement_peer_curve(&encoded[..encoded.len() - 1]).is_err());
        }
        let xdh = spki(ec::X509_X25519_OID, None, &[1; 32]);
        assert_eq!(agreement_peer_curve(&xdh).unwrap(), KmEcCurve::Curve25519);
        for invalid in [
            vec![],
            vec![0],
            spki(ec::X509_ED25519_OID, None, &[1; 32]),
            spki(ec::X509_X25519_OID, None, &[1; 31]),
            spki(ec::X509_X25519_OID, Some(ec::ALGO_PARAM_P256_OID), &[1; 32]),
            spki(ec::X509_NIST_OID, None, &[1; 65]),
            spki(ec::X509_NIST_OID, Some(ec::ALGO_PARAM_P256_OID), &[1; 65]),
        ] {
            let error = agreement_peer_curve(&invalid).unwrap_err();
            assert_eq!(relay_error_result(&error)["keymint_error_code"], -38);
        }
        assert_eq!(
            relay_error_result(&agreement_peer_curve(&[0; 165]).unwrap_err())["keymint_error_code"],
            -21
        );
    }

    #[test]
    fn agreement_uses_only_finish_and_keeps_raw_output() {
        let expected = vec![0; 32];
        let output = complete_single_input_op(
            "mock",
            KeyPurpose::AGREE_KEY,
            b"peer-spki",
            |_| panic!("agreement must not call update"),
            |input| {
                assert_eq!(input, Some(b"peer-spki".as_slice()));
                Ok(expected.clone())
            },
            || panic!("successful finish must not abort"),
        )
        .unwrap();
        assert_eq!(output, expected);
    }

    #[test]
    fn agreement_error_is_preserved_without_repeating_finish() {
        let aborted = Cell::new(0);
        let error = complete_single_input_op(
            "mock",
            KeyPurpose::AGREE_KEY,
            b"peer-spki",
            |_| panic!("agreement must not call update"),
            |_| Err(rsbinder::Status::new_service_specific_error(-38, None)),
            || aborted.set(aborted.get() + 1),
        )
        .unwrap_err();
        assert_eq!(relay_error_result(&error)["keymint_error_code"], -38);
        assert_eq!(aborted.get(), 1);
    }

    #[test]
    fn sign_and_decrypt_still_concatenate_update_and_finish_output() {
        for purpose in [KeyPurpose::SIGN, KeyPurpose::DECRYPT] {
            let output = complete_single_input_op(
                "mock",
                purpose,
                b"input",
                |input| {
                    assert_eq!(input, b"input");
                    Ok(vec![1, 2])
                },
                |input| {
                    assert!(input.is_none());
                    Ok(vec![3])
                },
                || panic!("successful operation must not abort"),
            )
            .unwrap();
            assert_eq!(output, [1, 2, 3]);
        }
    }

    #[test]
    fn agreement_generation_does_not_add_sign_digest_or_padding() {
        use crate::android::hardware::security::keymint::Tag::Tag;
        for curve in [
            KmEcCurve::P224,
            KmEcCurve::P256,
            KmEcCurve::P384,
            KmEcCurve::P521,
            KmEcCurve::Curve25519,
        ] {
            let spec = KeySpec {
                ec_curve: Some(curve),
                key_size: Some(ec::curve_to_key_size(curve).0),
                purposes: vec![KmKeyPurpose::AgreeKey],
                ..KeySpec::default()
            };
            for version in [100, 200, 300, 400, 500] {
                let params =
                    build_attestation_params(b"synthetic-appid", b"challenge", &spec, version)
                        .unwrap();
                let purposes: Vec<_> = params.iter().filter(|p| p.tag == Tag::PURPOSE).collect();
                let expected =
                    key_params_to_aidl(&[KeyParam::Purpose(KmKeyPurpose::AgreeKey)], version)
                        .unwrap();
                assert_eq!(purposes, expected.iter().collect::<Vec<_>>());
                assert!(params.iter().all(|p| p.tag != Tag::DIGEST
                    && p.tag != Tag::PADDING
                    && p.tag != Tag::RSA_OAEP_MGF_DIGEST));
            }
        }
    }
}

#[cfg(test)]
mod algorithm_mapping_tests {
    use super::*;
    use crate::keymaster::relay_tee::relay_error_result;

    fn assert_keymint_error(error: anyhow::Error, expected: KmErrorCode) {
        assert_eq!(
            relay_error_result(&error)["keymint_error_code"],
            expected as i32
        );
    }

    #[test]
    fn existing_sha256_pss_oaep_and_pkcs1_params_are_unchanged() {
        assert_eq!(
            sign_key_params("SHA256withRSA/PSS", KeyAlgorithm::Rsa2048).unwrap(),
            vec![
                KeyParam::Digest(KmDigest::Sha256),
                KeyParam::Padding(KmPadding::RsaPss),
                KeyParam::RsaOaepMgfDigest(KmDigest::Sha256),
            ]
        );
        assert_eq!(
            decrypt_key_params("RSA/OAEP/SHA-256/MGF1-SHA1", KeyAlgorithm::Rsa2048,).unwrap(),
            vec![
                KeyParam::Padding(KmPadding::RsaOaep),
                KeyParam::Digest(KmDigest::Sha256),
                KeyParam::RsaOaepMgfDigest(KmDigest::Sha1),
            ]
        );
        assert_eq!(
            decrypt_key_params("RSA/OAEP/SHA-256", KeyAlgorithm::Rsa2048).unwrap(),
            vec![
                KeyParam::Padding(KmPadding::RsaOaep),
                KeyParam::Digest(KmDigest::Sha256),
                KeyParam::RsaOaepMgfDigest(KmDigest::Sha1),
            ]
        );
        assert_eq!(
            decrypt_key_params("RSA/ECB/PKCS1Padding", KeyAlgorithm::Rsa2048).unwrap(),
            vec![KeyParam::Padding(KmPadding::RsaPkcs115Encrypt)]
        );
    }

    #[test]
    fn no_padding_sha224_and_none_map_exactly() {
        assert_eq!(
            sign_key_params("NONEwithRSA/NoPadding", KeyAlgorithm::Rsa2048).unwrap(),
            vec![
                KeyParam::Digest(KmDigest::None),
                KeyParam::Padding(KmPadding::None),
            ]
        );
        assert_eq!(
            sign_key_params("NONEwithRSA", KeyAlgorithm::Rsa2048).unwrap(),
            vec![
                KeyParam::Digest(KmDigest::None),
                KeyParam::Padding(KmPadding::RsaPkcs115Sign),
            ]
        );
        assert_eq!(
            decrypt_key_params("RSA/ECB/NoPadding", KeyAlgorithm::Rsa2048).unwrap(),
            vec![KeyParam::Padding(KmPadding::None)]
        );
        assert_eq!(
            sign_key_params("SHA224withECDSA", KeyAlgorithm::EcP256).unwrap(),
            vec![KeyParam::Digest(KmDigest::Sha224)]
        );
        assert_eq!(
            sign_key_params("NONEwithECDSA", KeyAlgorithm::EcP256).unwrap(),
            vec![KeyParam::Digest(KmDigest::None)]
        );
        assert_eq!(
            decrypt_key_params("RSA/OAEP/SHA-224/MGF1-SHA224", KeyAlgorithm::Rsa2048,).unwrap(),
            vec![
                KeyParam::Padding(KmPadding::RsaOaep),
                KeyParam::Digest(KmDigest::Sha224),
                KeyParam::RsaOaepMgfDigest(KmDigest::Sha224),
            ]
        );
        assert_eq!(
            sign_key_params("MD5withRSA/PSS", KeyAlgorithm::Rsa2048).unwrap(),
            vec![
                KeyParam::Digest(KmDigest::Md5),
                KeyParam::Padding(KmPadding::RsaPss),
                KeyParam::RsaOaepMgfDigest(KmDigest::Md5),
            ]
        );
        assert_eq!(
            decrypt_key_params("RSA/OAEP/MD5/MGF1-MD5", KeyAlgorithm::Rsa2048).unwrap(),
            vec![
                KeyParam::Padding(KmPadding::RsaOaep),
                KeyParam::Digest(KmDigest::Md5),
                KeyParam::RsaOaepMgfDigest(KmDigest::Md5),
            ]
        );
    }

    #[test]
    fn malformed_algorithms_keep_specific_keymint_error_codes() {
        assert_keymint_error(
            decrypt_key_params("RSA/OAEP/SHA-256/MGF1-SHA999", KeyAlgorithm::Rsa2048).unwrap_err(),
            KmErrorCode::UnsupportedMgfDigest,
        );
        assert_keymint_error(
            decrypt_key_params("RSA/OAEP/SHA999/MGF1-SHA256", KeyAlgorithm::Rsa2048).unwrap_err(),
            KmErrorCode::UnsupportedDigest,
        );
        assert_keymint_error(
            decrypt_key_params("RSA/ECB/UnknownPadding", KeyAlgorithm::Rsa2048).unwrap_err(),
            KmErrorCode::UnsupportedPaddingMode,
        );
        assert_keymint_error(
            sign_key_params("SHA256withRSA/NoPadding", KeyAlgorithm::Rsa2048).unwrap_err(),
            KmErrorCode::IncompatibleDigest,
        );
        assert_keymint_error(
            sign_key_params("SHA256withRSA/UnknownPadding", KeyAlgorithm::Rsa2048).unwrap_err(),
            KmErrorCode::UnsupportedPaddingMode,
        );
        assert_keymint_error(
            sign_key_params("SHA256withRSA", KeyAlgorithm::EcP256).unwrap_err(),
            KmErrorCode::IncompatibleAlgorithm,
        );
    }
}

#[cfg(test)]
mod upgrade_tests {
    use super::*;
    use crate::keymaster::relay_tee::relay_error_result;
    use std::cell::{Cell, RefCell};
    use std::sync::{Arc, Barrier};

    fn session() -> TeeSession {
        TeeSession {
            key_blob: vec![1],
            cert_chain: vec![vec![3, 4], vec![5, 6]],
            algorithm: KeyAlgorithm::EcP256,
            hal_service: SYSTEM_KEYMINT_DEFAULT,
            km_version: 400,
        }
    }

    fn status(code: KmErrorCode) -> rsbinder::Status {
        rsbinder::Status::new_service_specific_error(code as i32, None)
    }

    #[test]
    fn normal_begin_never_upgrades_or_persists() {
        let result = begin_with_upgrade_retry(
            &session(),
            |blob| Ok(blob.to_vec()),
            || panic!("successful begin must not recover"),
        )
        .unwrap();
        assert_eq!(result, [1]);
    }

    #[test]
    fn other_hal_and_transport_errors_do_not_upgrade() {
        for code in [
            KmErrorCode::KeyUserNotAuthenticated,
            KmErrorCode::InvalidKeyBlob,
            KmErrorCode::VerificationFailed,
            KmErrorCode::HardwareTypeUnavailable,
        ] {
            let error = begin_with_upgrade_retry::<()>(
                &session(),
                |_| Err(status(code)),
                || panic!("non-upgrade HAL failure must not recover"),
            )
            .unwrap_err();
            assert_eq!(
                relay_error_result(&error)["keymint_error_code"],
                code as i32
            );
        }
        let error = begin_with_upgrade_retry::<()>(
            &session(),
            |_| Err(rsbinder::Status::from(rsbinder::StatusCode::DeadObject)),
            || panic!("transport failure must not upgrade"),
        )
        .unwrap_err();
        assert!(relay_error_result(&error)
            .get("keymint_error_code")
            .is_none());
    }

    #[test]
    fn upgrade_is_saved_before_retry_and_preserves_identity() {
        let observed = session();
        let mut cached = CachedSession::from(observed.clone());
        let events = RefCell::new(Vec::new());
        let result = begin_with_upgrade_retry(
            &observed,
            |blob| {
                if blob == [1] {
                    events.borrow_mut().push("begin old");
                    Err(status(KmErrorCode::KeyRequiresUpgrade))
                } else {
                    assert_eq!(blob, [2]);
                    events.borrow_mut().push("begin upgraded");
                    Ok(42)
                }
            },
            || {
                recover_upgraded_session(
                    &mut cached,
                    &observed,
                    |blob| {
                        assert_eq!(blob, [1]);
                        events.borrow_mut().push("upgrade");
                        Ok(vec![2])
                    },
                    |updated| {
                        assert_eq!(updated.key_blob, [2]);
                        assert_eq!(updated.cert_chain, observed.cert_chain);
                        assert_eq!(updated.algorithm, observed.algorithm);
                        assert_eq!(updated.hal_service, observed.hal_service);
                        assert_eq!(updated.km_version, observed.km_version);
                        events.borrow_mut().push("persist");
                        Ok(())
                    },
                )
            },
        )
        .unwrap();
        assert_eq!(result, 42);
        assert!(!cached.needs_persistence);
        assert_eq!(
            *events.borrow(),
            ["begin old", "upgrade", "persist", "begin upgraded"]
        );
    }

    #[test]
    fn upgrade_required_on_retry_is_returned_without_a_loop() {
        let attempts = Cell::new(0);
        let recoveries = Cell::new(0);
        let error = begin_with_upgrade_retry::<()>(
            &session(),
            |_| {
                attempts.set(attempts.get() + 1);
                Err(status(KmErrorCode::KeyRequiresUpgrade))
            },
            || {
                recoveries.set(recoveries.get() + 1);
                Ok(TeeSession {
                    key_blob: vec![2],
                    ..session()
                })
            },
        )
        .unwrap_err();
        assert_eq!(attempts.get(), 2);
        assert_eq!(recoveries.get(), 1);
        assert_eq!(relay_error_result(&error)["keymint_error_code"], -62);
    }

    #[test]
    fn failed_upgrade_keeps_old_blob_and_preserves_error() {
        let observed = session();
        let mut cached = CachedSession::from(observed.clone());
        let error = recover_upgraded_session(
            &mut cached,
            &observed,
            |_| {
                Err(keymint_status_error(
                    &status(KmErrorCode::InvalidArgument),
                    "upgradeKey",
                ))
            },
            |_| panic!("failed upgrade must not persist"),
        )
        .unwrap_err();
        assert_eq!(relay_error_result(&error)["keymint_error_code"], -38);
        assert_eq!(cached.session.key_blob, observed.key_blob);
        assert!(!cached.needs_persistence);
    }

    #[test]
    fn empty_upgrade_result_is_rejected_without_overwriting() {
        let observed = session();
        let mut cached = CachedSession::from(observed.clone());
        assert!(recover_upgraded_session(
            &mut cached,
            &observed,
            |_| Ok(Vec::new()),
            |_| panic!("empty blob must not be saved"),
        )
        .is_err());
        assert_eq!(cached.session.key_blob, observed.key_blob);
        assert!(!cached.needs_persistence);
    }

    #[test]
    fn failed_save_stops_retry_and_pending_blob_is_not_lost() {
        let observed = session();
        let mut cached = CachedSession::from(observed.clone());
        let attempts = Cell::new(0);
        let error = begin_with_upgrade_retry::<()>(
            &observed,
            |_| {
                attempts.set(attempts.get() + 1);
                Err(status(KmErrorCode::KeyRequiresUpgrade))
            },
            || {
                recover_upgraded_session(
                    &mut cached,
                    &observed,
                    |_| Ok(vec![2]),
                    |_| Err(anyhow!("simulated disk full")),
                )
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("disk full"));
        assert_eq!(attempts.get(), 1);
        assert_eq!(cached.session.key_blob, [2]);
        assert!(cached.needs_persistence);
        assert!(persist_pending_session(&mut cached, |_| Err(anyhow!("still full"))).is_err());
        assert!(cached.needs_persistence);
        let fresh = recover_upgraded_session(
            &mut cached,
            &observed,
            |_| panic!("pending blob must not be upgraded again"),
            |updated| {
                assert_eq!(updated.key_blob, [2]);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(fresh.key_blob, [2]);
        assert!(!cached.needs_persistence);
    }

    #[test]
    fn replaced_alias_is_not_overwritten_by_stale_upgrade() {
        let observed = session();
        for replacement in [
            TeeSession {
                cert_chain: vec![vec![9]],
                ..session()
            },
            TeeSession {
                hal_service: SYSTEM_KEYMINT_STRONGBOX,
                ..session()
            },
            TeeSession {
                algorithm: KeyAlgorithm::Rsa2048,
                ..session()
            },
        ] {
            let mut cached = CachedSession::from(replacement);
            let error = recover_upgraded_session(
                &mut cached,
                &observed,
                |_| panic!("different key identity must not upgrade"),
                |_| panic!("different key identity must not overwrite the alias"),
            )
            .unwrap_err();
            assert_eq!(relay_error_result(&error)["keymint_error_code"], -33);
        }
    }

    #[test]
    fn persisted_upgrade_survives_cache_reconstruction() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.json");
        let observed = session();
        save_session_at(&path, "synthetic-key", &observed).unwrap();
        let mut cached = CachedSession::from(load_session_from_path(&path).unwrap());
        recover_upgraded_session(
            &mut cached,
            &observed,
            |_| Ok(vec![2, 3, 4]),
            |session| save_session_at(&path, "synthetic-key", session),
        )
        .unwrap();
        drop(cached);
        let restored = load_session_from_path(&path).unwrap();
        assert_eq!(restored.key_blob, [2, 3, 4]);
        assert_eq!(restored.cert_chain, observed.cert_chain);
        assert_eq!(restored.algorithm, observed.algorithm);
        assert_eq!(restored.hal_service, observed.hal_service);
        assert_eq!(restored.km_version, observed.km_version);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn concurrent_stale_snapshots_share_one_updated_blob() {
        let cached = Arc::new(Mutex::new(CachedSession::from(session())));
        let barrier = Arc::new(Barrier::new(2));
        let upgrades = std::sync::atomic::AtomicUsize::new(0);
        let saves = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let cached = cached.clone();
                let barrier = barrier.clone();
                let upgrades = &upgrades;
                let saves = &saves;
                scope.spawn(move || {
                    let observed = session();
                    let mut first = true;
                    let result = begin_with_upgrade_retry(
                        &observed,
                        |blob| {
                            if first {
                                first = false;
                                barrier.wait();
                                Err(status(KmErrorCode::KeyRequiresUpgrade))
                            } else {
                                assert_eq!(blob, [2]);
                                assert_eq!(saves.load(Ordering::SeqCst), 1);
                                Ok(())
                            }
                        },
                        || {
                            recover_upgraded_session(
                                &mut cached.lock().unwrap(),
                                &observed,
                                |_| {
                                    upgrades.fetch_add(1, Ordering::SeqCst);
                                    Ok(vec![2])
                                },
                                |_| {
                                    saves.fetch_add(1, Ordering::SeqCst);
                                    Ok(())
                                },
                            )
                        },
                    );
                    result.unwrap();
                });
            }
        });
        assert_eq!(upgrades.load(Ordering::SeqCst), 1);
        assert_eq!(saves.load(Ordering::SeqCst), 1);
    }
}
