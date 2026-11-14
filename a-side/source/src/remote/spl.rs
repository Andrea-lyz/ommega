//! Native A version admission and value-free-of-secrets diagnostic publication.
//! No network, HAL calls or property writes are performed by this module.

use std::{path::Path, sync::Mutex};

use anyhow::{anyhow, Result};
use kmr_ta::device::KeyMintVersionTags;
use serde_json::{json, Map, Value};

use super::{RelayKeyMintError, RemoteIdentityProfile, REMOTE_IDENTITY_PROFILE};

pub(super) const POLICY: &str = "a-native-v1";
const STATUS_PATH: &str = "/data/misc/keystore/ommega/data/spl-status.json";
static STATUS_WRITE: Mutex<()> = Mutex::new(());

pub(super) fn tags_json(tags: KeyMintVersionTags) -> Value {
    json!({
        "os_version": tags.os_version,
        "os_patchlevel": tags.os_patchlevel,
        "vendor_patchlevel": tags.vendor_patchlevel,
        "boot_patchlevel": tags.boot_patchlevel,
    })
}

pub(super) fn match_profile(
    expected: KeyMintVersionTags,
    profile: &RemoteIdentityProfile,
) -> std::result::Result<(), String> {
    if expected.vendor_patchlevel.is_none() || expected.boot_patchlevel.is_none() {
        return Err("A native partition version source is unknown".into());
    }
    if profile.version_policy.as_deref() != Some(POLICY) {
        return Err(
            "B does not provide the a-native-v1 pre-HAL version guard; update B separately".into(),
        );
    }
    if !profile
        .version_revision
        .as_deref()
        .is_some_and(valid_revision)
    {
        return Err("B version snapshot has no valid revision".into());
    }
    let observed = profile
        .version_tags
        .ok_or_else(|| "B has not published hardware version authorizations".to_string())?;
    if observed != expected {
        return Err(format!(
            "B version snapshot does not match A native target: expected={expected:?} observed={observed:?}; A SPL is never changed"
        ));
    }
    Ok(())
}

pub(super) fn valid_revision(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 192
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
}

pub(super) fn admitted_target(profile: &RemoteIdentityProfile) -> Result<KeyMintVersionTags> {
    let expected = crate::config::config()
        .read()
        .map_err(|_| anyhow!("config lock poisoned while checking native SPL target"))
        .and_then(|config| crate::plat::vbmeta::native_version_target(&config.trust));
    let result = expected.and_then(|expected| {
        match_profile(expected, profile).map_err(anyhow::Error::msg)?;
        Ok(expected)
    });
    result.map_err(|error| {
        anyhow::Error::new(RelayKeyMintError {
            code: kmr_wire::keymint::ErrorCode::VerificationFailed,
            message: format!("native SPL admission failed: {error:#}"),
        })
    })
}

pub(super) fn add_context(
    context: &mut Map<String, Value>,
    expected: KeyMintVersionTags,
    revision: &str,
) {
    context.insert("version_policy".into(), json!(POLICY));
    context.insert("expected_version_tags".into(), tags_json(expected));
    context.insert("expected_version_revision".into(), json!(revision));
    // Retain the server-keybox context spelling, but never truncate partition
    // dates or borrow one partition's date for another.
    context.insert("os_version".into(), json!(expected.os_version));
    context.insert("os_patch_level".into(), json!(expected.os_patchlevel));
    context.insert(
        "vendor_patch_level".into(),
        json!(expected.vendor_patchlevel),
    );
    context.insert("boot_patch_level".into(), json!(expected.boot_patchlevel));
}

fn version_source(tags: KeyMintVersionTags, source: &str) -> Value {
    let value = |value: Option<u32>| json!({"value": value, "source": source});
    json!({
        "os_version": value(Some(tags.os_version)),
        "system": value(Some(tags.os_patchlevel)),
        "vendor": value(tags.vendor_patchlevel),
        "boot": value(tags.boot_patchlevel),
    })
}

fn status_document(
    firmware: Value,
    configured: Option<KeyMintVersionTags>,
    profile: Option<&RemoteIdentityProfile>,
    relay_enabled: bool,
) -> Value {
    let (status, reason) = if !relay_enabled {
        (
            "unknown",
            "Local mode: no B hardware snapshot has been checked".to_string(),
        )
    } else {
        match (configured, profile) {
            (Some(expected), Some(profile)) => match match_profile(expected, profile) {
                Ok(()) => ("matched", "Cached B snapshot matches A; each new certificate and B pre-HAL revision are checked separately".to_string()),
                Err(error) => ("mismatch", error),
            },
            _ => ("unknown", "Native target or current-route B snapshot is unavailable".to_string()),
        }
    };
    let property = |name| {
        json!({
            "value": crate::plat::resetprop::read_string_property(name),
            "source": "observed Android property; not firmware or hardware acceptance proof",
        })
    };
    json!({
        "version": 1, "policy": "a-native-read-only",
        "hardware_verified": false,
        "sources": {
            "firmware": firmware,
            "android_properties": {
                "system": property("ro.build.version.security_patch"),
                "vendor": property("ro.vendor.build.security_patch"),
                "boot": property("ro.boot.image.build.security_patch"),
                "os_version": property("ro.build.version.release"),
            },
            "hal_configured": configured.map(|tags| version_source(tags, "frozen A software-TA configuration, not physical HAL observation")),
            "ta_observed": null,
            "remote_profile": profile.and_then(|p| p.version_tags).map(|tags| version_source(tags, "cached B hardware profile; freshness enforced by revision on B")),
            "certificate": null,
        },
        "remote_revision": profile.and_then(|p| p.version_revision.as_deref()),
        "match": {"status": status, "reason": reason},
    })
}

/// Best effort, equality-suppressed publication. Reads existing state only;
/// merely opening a diagnostic page cannot acquire a profile or invoke a HAL.
pub(crate) fn publish() {
    if let Err(error) = publish_inner() {
        log::warn!("could not publish read-only SPL diagnostics: {error:#}");
    }
}

fn publish_inner() -> Result<()> {
    let _write = STATUS_WRITE
        .lock()
        .map_err(|_| anyhow!("SPL status lock poisoned"))?;
    let (configured, relay_enabled, route) = {
        let config = crate::config::config()
            .read()
            .map_err(|_| anyhow!("config lock poisoned"))?;
        (
            crate::plat::vbmeta::native_version_target(&config.trust).ok(),
            config.remote.enabled && !config.remote.url.is_empty(),
            super::RemoteRoute::from_config(&config.remote),
        )
    };
    let firmware = crate::plat::vbmeta::native_spl_snapshot_json()?;
    let profile = REMOTE_IDENTITY_PROFILE.read().ok().and_then(|guard| {
        guard
            .as_ref()
            .filter(|entry| entry.route == route)
            .map(|entry| entry.profile.clone())
    });
    let contents = serde_json::to_vec_pretty(&status_document(
        firmware,
        configured,
        profile.as_ref(),
        relay_enabled,
    ))?;
    let path = Path::new(STATUS_PATH);
    if std::fs::read(path).ok().as_deref() == Some(contents.as_slice()) {
        return Ok(());
    }
    kmr_common::runtime::fs::atomic_replace_preserving_metadata(
        path,
        &contents,
        0o600,
        kmr_common::consts::KEYSTORE_UID,
        kmr_common::consts::KEYSTORE_GID,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> KeyMintVersionTags {
        KeyMintVersionTags {
            os_version: 170000,
            os_patchlevel: 202609,
            vendor_patchlevel: Some(20260901),
            boot_patchlevel: Some(20260900),
        }
    }
    fn profile() -> RemoteIdentityProfile {
        RemoteIdentityProfile {
            interface_version: 3,
            interface_hash: "a".repeat(40),
            profile_version: 300,
            hardware_version: 300,
            security_level: 1,
            keymint_name: "test".into(),
            keymint_author: "test".into(),
            has_strongbox: false,
            max_operations: None,
            version_tags: Some(target()),
            version_policy: Some(POLICY.into()),
            version_revision: Some("snapshot-1:2".into()),
        }
    }
    #[test]
    fn only_equal_complete_native_snapshot_is_admitted() {
        assert!(match_profile(target(), &profile()).is_ok());
        for which in 0..4 {
            let mut p = profile();
            let tags = p.version_tags.as_mut().unwrap();
            match which {
                0 => tags.os_version = 160000,
                1 => tags.os_patchlevel = 202608,
                2 => tags.vendor_patchlevel = Some(20260905),
                _ => tags.boot_patchlevel = Some(20260901),
            }
            assert!(match_profile(target(), &p).is_err());
        }
    }
    #[test]
    fn missing_versions_policy_or_revision_never_succeeds() {
        let mut p = profile();
        p.version_tags = None;
        assert!(match_profile(target(), &p).is_err());
        p = profile();
        p.version_policy = None;
        assert!(match_profile(target(), &p).is_err());
        p = profile();
        p.version_revision = None;
        assert!(match_profile(target(), &p).is_err());
        let mut t = target();
        t.vendor_patchlevel = None;
        assert!(match_profile(t, &profile()).is_err());
        t = target();
        t.boot_patchlevel = None;
        assert!(match_profile(t, &profile()).is_err());
        for bad in ["", "revision\nforged", "bad/revision"] {
            assert!(!valid_revision(bad));
        }
    }
    #[test]
    fn admission_failures_are_business_errors_not_fallback_signals() {
        let error = anyhow::Error::new(RelayKeyMintError {
            code: kmr_wire::keymint::ErrorCode::VerificationFailed,
            message: "B snapshot does not match immutable A target".into(),
        });
        let error = super::super::remote_operation_error(error, "version admission");
        assert!(matches!(
            error.kind(),
            kmr_common::ErrorKind::Hal(kmr_wire::keymint::ErrorCode::VerificationFailed, _)
        ));
    }

    #[test]
    fn context_preserves_each_partition_raw_precision_and_observed_zero() {
        let mut context = Map::new();
        let mut tags = target();
        tags.os_version = 0;
        tags.boot_patchlevel = Some(0);
        add_context(&mut context, tags, "revision-1");
        assert_eq!(context["os_version"], json!(0));
        assert_eq!(context["vendor_patch_level"], json!(20260901));
        assert_eq!(context["boot_patch_level"], json!(0));
        assert_eq!(context["expected_version_tags"], tags_json(tags));
        assert_eq!(context["version_policy"], json!(POLICY));
    }
}
