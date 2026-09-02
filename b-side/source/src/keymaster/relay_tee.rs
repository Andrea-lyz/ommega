//! Relay support: a self-contained, minimal bridge to the *real* hardware TEE.
//!
//! This module gathers the few primitives that the relay / attest_proxy /
//! tee_ops forwarding path needs in order to talk to the real on-device
//! keymint HAL, **without** pulling in the rest of the (software) keystore
//! stack:
//!
//!   * `get_system_keymint` / `clear_system_keymint` — connect (and cache) the
//!     real TEE `IKeyMintDevice` binder proxy;
//!   * `key_params_to_aidl` — convert `kmr_wire::KeyParam` (the canonical,
//!     wire-format key parameters) into the AIDL `KeyParameter` list the real
//!     HAL understands;
//!   * the `KEY_MINT_V*` HAL version constants;
//!   * `get_interface_once` — the cached service lookup helper.
//!
//! Everything here depends only on AIDL-generated keymint types and `rsbinder`,
//! so this module can survive on its own if the software keystore modules are
//! removed.

use std::{
    cell::RefCell,
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    sync::Arc,
};

use anyhow::{Context, Result};
use kmr_wire::keymint::KeyParam;
use log::error;
use rsbinder::{hub, DeathRecipient, FromIBinder, StatusCode, Strong, WIBinder};

use crate::android::hardware::security::keymint::{
    Algorithm::Algorithm, EcCurve::EcCurve, HardwareAuthenticatorType::HardwareAuthenticatorType,
    IKeyMintDevice::IKeyMintDevice, KeyOrigin::KeyOrigin,
    KeyParameter::KeyParameter as KmKeyParameter, KeyParameterValue::KeyParameterValue,
    KeyPurpose::KeyPurpose, MlDsaVariant::MlDsaVariant as AidlMlDsaVariant, Tag::Tag,
};

// ---------------------------------------------------------------------------
// HAL version constants (previously defined on `KeyMintDevice`).
// ---------------------------------------------------------------------------

pub const KEY_MINT_V3: i32 = 300;
pub const KEY_MINT_V4: i32 = 400;
pub const KEY_MINT_V5: i32 = 500;

// ---------------------------------------------------------------------------
// Service lookup.
// ---------------------------------------------------------------------------

/// Performs a single `getService` lookup for the named binder service without
/// inheriting version-dependent wait behaviour.  Mirrors the implementation in
/// `keymaster/utils.rs` so the relay path does not depend on it.
pub(crate) fn get_interface_once<T: FromIBinder + ?Sized>(
    name: &str,
) -> Result<Strong<T>, StatusCode> {
    let binder = hub::default()?
        .try_get_service(name)
        .ok()
        .flatten()
        .ok_or(StatusCode::NameNotFound)?;
    FromIBinder::try_from(binder)
}

// ---------------------------------------------------------------------------
// Real TEE keymint proxy (cached).
// ---------------------------------------------------------------------------

/// Global generation counter for keymint cache invalidation.  Incremented
/// whenever a service death is detected, so every thread's thread-local cache
/// is invalidated on the next `get_system_keymint` call.
static KEYMINT_CACHE_GEN: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static SYSTEM_KEYMINT_CACHE: RefCell<Option<HashMap<&'static str, Strong<dyn IKeyMintDevice>>>> =
        const { RefCell::new(None) };
    static SYSTEM_KEYMINT_DEATH: RefCell<Option<HashMap<&'static str, Arc<dyn DeathRecipient>>>> =
        const { RefCell::new(None) };
    /// Thread-local snapshot of KEYMINT_CACHE_GEN at last cache build.
    static KEYMINT_CACHE_LOCAL_GEN: RefCell<u64> = const { RefCell::new(0) };
}

struct SystemKeymintDeath {
    service: &'static str,
}

impl DeathRecipient for SystemKeymintDeath {
    fn binder_died(&self, _who: &WIBinder) {
        clear_system_keymint(self.service);
        log::warn!(
            "system KeyMint verifier service {} died; cache cleared",
            self.service
        );
    }
}

/// Connects to the real hardware keymint HAL for the given service name
/// (e.g. `android.hardware.security.keymint.IKeyMintDevice/default`), caching
/// the proxy and watching for death.
///
/// The cache is thread-local but invalidated globally: when any thread detects
/// a binder death it bumps a global generation counter, and the next call on
/// *any* thread rebuilds its own cache from scratch.
pub fn get_system_keymint(service: &'static str) -> Result<Strong<dyn IKeyMintDevice>> {
    // Fast path: check global generation against thread-local snapshot.
    // If they differ, the cache is stale (some thread observed a death).
    let global_gen = KEYMINT_CACHE_GEN.load(Ordering::Acquire);
    let stale = KEYMINT_CACHE_LOCAL_GEN.with(|g| *g.borrow() != global_gen);
    if stale {
        clear_thread_local_keymint();
        KEYMINT_CACHE_LOCAL_GEN.with(|g| *g.borrow_mut() = global_gen);
    }

    SYSTEM_KEYMINT_CACHE.with(|cache| {
        if let Some(keymint) = cache
            .borrow()
            .as_ref()
            .and_then(|services| services.get(service).cloned())
        {
            return Ok(keymint);
        }

        let keymint: Strong<dyn IKeyMintDevice> =
            get_interface_once(service).with_context(|| format!("connect {service}"))?;
        let recipient: Arc<dyn DeathRecipient> = Arc::new(SystemKeymintDeath { service });
        keymint
            .as_binder()
            .link_to_death(Arc::downgrade(&recipient))
            .with_context(|| format!("watch {service} death"))?;
        SYSTEM_KEYMINT_DEATH.with(|death| {
            death
                .borrow_mut()
                .get_or_insert_with(HashMap::new)
                .insert(service, recipient);
        });
        cache
            .borrow_mut()
            .get_or_insert_with(HashMap::new)
            .insert(service, keymint.clone());
        Ok(keymint)
    })
}

/// Clears the current thread's keymint cache and death recipients.
fn clear_thread_local_keymint() {
    SYSTEM_KEYMINT_CACHE.with(|cache| {
        *cache.borrow_mut() = None;
    });
    SYSTEM_KEYMINT_DEATH.with(|death| {
        *death.borrow_mut() = None;
    });
}

/// Drops the cached proxy (and death recipient) for `service` across all
/// threads by bumping the global generation counter.  Each thread will
/// rebuild its own cache lazily on the next `get_system_keymint` call.
pub fn clear_system_keymint(_service: &'static str) {
    // Bump the global generation so every thread invalidates its cache on
    // the next lookup.  We pass the service name for logging clarity but
    // invalidate all services — the cost of a full rebuild per thread is
    // negligible compared to a dead binder proxy, and keeping the logic
    // simple (single counter) avoids per-service atomic bookkeeping.
    KEYMINT_CACHE_GEN.fetch_add(1, Ordering::Release);
    log::warn!("system KeyMint cache invalidated (generation bumped); all threads will reconnect on next use");
}

// ---------------------------------------------------------------------------
// KeyMint error-code extraction & HAL version probing.
// ---------------------------------------------------------------------------

/// Extracts a KeyMint `ErrorCode` from a binder `Status` when it carries a
/// service-specific error (the convention AOSP `map_km_error` uses). Returns
/// `None` for non-service-specific failures so callers can tell e.g. -74
/// (ATTESTATION_KEYS_NOT_PROVISIONED) from a generic binder error.
pub fn extract_km_error_code(status: &rsbinder::Status) -> Option<i32> {
    if status.exception_code() == rsbinder::ExceptionCode::ServiceSpecific {
        let se = status.service_specific_error();
        if se < 0 {
            return Some(se);
        }
    }
    None
}

/// Probes the real KeyMint HAL for its implementation version via
/// `getHardwareInfo()` instead of assuming V5. A StrongBox HAL may only
/// implement KeyMint V2/V3; encoding parameters for a newer version can
/// cause version-mismatch errors that look like "not supported".
/// Falls back to `KEY_MINT_V5` if the probe itself fails.
pub fn probe_keymint_version(keymint: &Strong<dyn IKeyMintDevice>) -> i32 {
    match keymint.getHardwareInfo() {
        Ok(info) => {
            let normalized = normalize_keymint_version(info.versionNumber);
            log::info!(
                "KeyMint HAL: versionNumber={} normalized={} securityLevel={:?} name={}",
                info.versionNumber,
                normalized,
                info.securityLevel,
                info.keyMintName
            );
            normalized
        }
        Err(status) => {
            log::warn!("getHardwareInfo failed: {status:?}; falling back to KEY_MINT_V5");
            KEY_MINT_V5
        }
    }
}

pub fn normalize_keymint_version(version: i32) -> i32 {
    match version {
        1..=5 => version * 100,
        100 | 200 | KEY_MINT_V3 | KEY_MINT_V4 | KEY_MINT_V5 => version,
        _ => {
            log::warn!("unknown KeyMint version {version}; using v5 parameter encoding");
            KEY_MINT_V5
        }
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_keymint_version;

    #[test]
    fn keymint_version_is_normalized_to_canonical_units() {
        assert_eq!(normalize_keymint_version(1), 100);
        assert_eq!(normalize_keymint_version(4), 400);
        assert_eq!(normalize_keymint_version(400), 400);
        assert_eq!(normalize_keymint_version(999), 500);
    }
}

// ---------------------------------------------------------------------------
// Key parameter conversion (kmr_wire::KeyParam -> AIDL KeyParameter).
// ---------------------------------------------------------------------------

pub fn key_params_to_aidl(params: &[KeyParam], km_dev_version: i32) -> Result<Vec<KmKeyParameter>> {
    params
        .iter()
        .cloned()
        .map(|param| key_param_to_aidl(param, km_dev_version))
        .collect()
}

pub fn key_param_to_aidl(kp: KeyParam, km_dev_version: i32) -> Result<KmKeyParameter> {
    use kmr_wire::{keymint::KeyParam as KP, KeySizeInBits};
    let mut tag = Tag(kp.tag() as i32);
    let value = match kp {
        KP::Purpose(v) => KeyParameterValue::KeyPurpose(KeyPurpose(v as i32)),
        KP::Algorithm(v) => KeyParameterValue::Algorithm(Algorithm(v as i32)),
        KP::KeySize(KeySizeInBits(v)) => KeyParameterValue::Integer(v as i32),
        KP::BlockMode(v) => KeyParameterValue::BlockMode(
            crate::android::hardware::security::keymint::BlockMode::BlockMode(v as i32),
        ),
        KP::Digest(v) => KeyParameterValue::Digest(
            crate::android::hardware::security::keymint::Digest::Digest(v as i32),
        ),
        KP::Padding(v) => KeyParameterValue::PaddingMode(
            crate::android::hardware::security::keymint::PaddingMode::PaddingMode(v as i32),
        ),
        KP::CallerNonce => KeyParameterValue::BoolValue(true),
        KP::MinMacLength(v) => KeyParameterValue::Integer(v as i32),
        KP::EcCurve(v) => KeyParameterValue::EcCurve(EcCurve(v as i32)),
        KP::MlDsaVariant(v) if km_dev_version < KEY_MINT_V5 => {
            error!("TA emitted ML_DSA_VARIANT tag but HAL v5 is not supported");
            tag = Tag::INVALID;
            KeyParameterValue::Integer(v as i32)
        }
        KP::MlDsaVariant(v) => KeyParameterValue::MlDsaVariant(AidlMlDsaVariant(v as i32)),
        KP::RsaPublicExponent(kmr_wire::RsaExponent(v)) => KeyParameterValue::LongInteger(v as i64),
        KP::IncludeUniqueId => KeyParameterValue::BoolValue(true),
        KP::RsaOaepMgfDigest(v) => KeyParameterValue::Digest(
            crate::android::hardware::security::keymint::Digest::Digest(v as i32),
        ),
        KP::BootloaderOnly
        | KP::RollbackResistance
        | KP::EarlyBootOnly
        | KP::NoAuthRequired
        | KP::AllowWhileOnBody
        | KP::TrustedUserPresenceRequired
        | KP::TrustedConfirmationRequired
        | KP::UnlockedDeviceRequired
        | KP::DeviceUniqueAttestation
        | KP::StorageKey
        | KP::ResetSinceIdRotation => KeyParameterValue::BoolValue(true),
        KP::ActiveDatetime(v)
        | KP::OriginationExpireDatetime(v)
        | KP::UsageExpireDatetime(v)
        | KP::CreationDatetime(v)
        | KP::CertificateNotBefore(v)
        | KP::CertificateNotAfter(v) => KeyParameterValue::DateTime(v.ms_since_epoch),
        KP::MaxUsesPerBoot(v)
        | KP::UsageCountLimit(v)
        | KP::UserId(v)
        | KP::AuthTimeout(v)
        | KP::OsVersion(v)
        | KP::OsPatchlevel(v)
        | KP::VendorPatchlevel(v)
        | KP::BootPatchlevel(v)
        | KP::MacLength(v)
        | KP::MaxBootLevel(v) => KeyParameterValue::Integer(v as i32),
        KP::UserAuthType(v) => {
            KeyParameterValue::HardwareAuthenticatorType(HardwareAuthenticatorType(v as i32))
        }
        KP::UserSecureId(v) => KeyParameterValue::LongInteger(v as i64),
        KP::ApplicationId(v)
        | KP::ApplicationData(v)
        | KP::RootOfTrust(v)
        | KP::AttestationChallenge(v)
        | KP::AttestationApplicationId(v)
        | KP::AttestationIdBrand(v)
        | KP::AttestationIdDevice(v)
        | KP::AttestationIdProduct(v)
        | KP::AttestationIdSerial(v)
        | KP::AttestationIdImei(v)
        | KP::AttestationIdMeid(v)
        | KP::AttestationIdManufacturer(v)
        | KP::AttestationIdModel(v)
        | KP::Nonce(v)
        | KP::CertificateSerial(v)
        | KP::CertificateSubject(v) => KeyParameterValue::Blob(v),
        KP::AttestationIdSecondImei(v) if km_dev_version < KEY_MINT_V3 => {
            error!("TA emitted ATTESTATION_ID_SECOND_IMEI tag but HAL v3 is not supported");
            tag = Tag::INVALID;
            KeyParameterValue::Blob(v)
        }
        KP::AttestationIdSecondImei(v) => KeyParameterValue::Blob(v),
        KP::ModuleHash(v) if km_dev_version < KEY_MINT_V4 => {
            error!("TA emitted MODULE_HASH tag but HAL v4 is not supported");
            tag = Tag::INVALID;
            KeyParameterValue::Blob(v)
        }
        KP::ModuleHash(v) => KeyParameterValue::Blob(v),
        KP::Origin(v) => KeyParameterValue::Origin(KeyOrigin(v as i32)),
    };

    Ok(KmKeyParameter { tag, value })
}
