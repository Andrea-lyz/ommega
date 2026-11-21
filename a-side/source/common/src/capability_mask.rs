//! Boot-time system feature declarations, independent of KeyMint routing.

use std::path::Path;
use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};

pub const CONFIG_PATH: &str = "/data/misc/keystore/ommega/data/system-capability-mask.conf";
pub const XML_PATH: &str = "system/etc/permissions/ommega-capability-mask.xml";
pub const MOUNTED_XML_PATH: &str = "/system/etc/permissions/ommega-capability-mask.xml";
pub const STRONGBOX_FEATURE: &str = "android.hardware.strongbox_keystore";
pub const DEVICE_ID_ATTESTATION_FEATURE: &str = "android.software.device_id_attestation";
pub const PACKAGE_MANAGER_NATIVE_SERVICE: &str = "package_native";
pub const PACKAGE_MANAGER_NATIVE_DESCRIPTOR: &str = "android.content.pm.IPackageManagerNative";
pub const MAX_CONFIG: usize = 4096;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Config {
    pub strongbox: bool,
    pub device_id_attestation: bool,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        if text.len() > MAX_CONFIG {
            return Err("System capability policy too large");
        }
        let (mut version, mut strongbox, mut device_id) = (None, None, None);
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or("Invalid system capability policy")?;
            match (key.trim(), value.trim()) {
                ("version", "1") if version.is_none() => version = Some(()),
                ("strongbox", "true" | "false") if strongbox.is_none() => {
                    strongbox = Some(value.trim() == "true");
                }
                ("device_id_attestation", "true" | "false") if device_id.is_none() => {
                    device_id = Some(value.trim() == "true");
                }
                _ => return Err("Unsupported or duplicate system capability setting"),
            }
        }
        match (version, strongbox, device_id) {
            (Some(()), Some(strongbox), Some(device_id_attestation)) => Ok(Self {
                strongbox,
                device_id_attestation,
            }),
            _ => Err("Incomplete system capability policy"),
        }
    }

    pub fn permissions_xml(self) -> String {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<permissions>\n");
        if self.strongbox {
            xml.push_str(&format!(
                "    <unavailable-feature name=\"{STRONGBOX_FEATURE}\" />\n"
            ));
        }
        if self.device_id_attestation {
            xml.push_str(&format!(
                "    <unavailable-feature name=\"{DEVICE_ID_ATTESTATION_FEATURE}\" />\n"
            ));
        }
        xml.push_str("</permissions>\n");
        xml
    }

    pub fn from_xml(xml: &str) -> Result<Self, &'static str> {
        let normalized = xml.replace("\r\n", "\n");
        for strongbox in [false, true] {
            for device_id_attestation in [false, true] {
                let config = Self {
                    strongbox,
                    device_id_attestation,
                };
                if normalized.trim() == config.permissions_xml().trim() {
                    return Ok(config);
                }
            }
        }
        Err("Unrecognized mounted system capability XML")
    }
}

pub fn mounted_config_from_path(path: &Path) -> Option<Config> {
    let xml = std::fs::read_to_string(path).ok()?;
    Config::from_xml(&xml).ok()
}

/// Reads the persisted capability config. Returns `None` when the file is
/// missing, unreadable, or fails `Config::parse`.
fn persistent_config_from_path(path: &Path) -> Option<Config> {
    let text = std::fs::read_to_string(path).ok()?;
    Config::parse(&text).ok()
}

fn strongbox_masked_from_paths(xml_path: &Path, conf_path: &Path) -> bool {
    match mounted_config_from_path(xml_path) {
        Some(config) => config.strongbox,
        None => {
            log::debug!(
                "capability mask XML unreadable at {}; using persisted config",
                xml_path.display()
            );
            persistent_config_from_path(conf_path).is_some_and(|config| config.strongbox)
        }
    }
}

/// Whether the StrongBox feature declaration is masked for this boot. The
/// mounted permissions XML is authoritative; when it cannot be read — for
/// example inside `keystore`, which may lack read access to the
/// module-mounted file — the persisted config that generated it is used
/// instead.
pub fn mounted_strongbox_masked() -> bool {
    strongbox_masked_from_paths(Path::new(MOUNTED_XML_PATH), Path::new(CONFIG_PATH))
}

/// Transaction offset from `FIRST_CALL_TRANSACTION` for
/// `IPackageManagerNative::hasSystemFeature(String, int)` on Android 12-17.
pub fn pm_has_system_feature_offset(android_major: Option<i32>) -> u32 {
    match android_major {
        Some(..=13) => 12,
        Some(14) => 9,
        Some(15 | 16) | None => 10,
        Some(17..) => 13,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PmFeatureProbe {
    Resolved(bool),
    Unavailable,
    Failed,
}

const STATE_UNPROBED: u8 = 0;
const STATE_DECLARED: u8 = 1;
const STATE_HIDDEN: u8 = 2;

/// Minimum interval between `package_native` probes after a failed probe.
/// A failure (transport error, dead object, parse failure) may be transient,
/// so probing resumes after this delay instead of being cached forever.
pub const PM_PROBE_RETRY_MS: i64 = 30_000;

/// Monotonic time in milliseconds (CLOCK_BOOTTIME) for the retry interval.
#[allow(clippy::unnecessary_cast)]
pub fn monotonic_ms() -> i64 {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a valid out-pointer; clock_gettime writes it before returning.
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut now) };
    now.tv_sec as i64 * 1000 + (now.tv_nsec as i64 / 1_000_000)
}

/// Per-process cache for whether `android.hardware.strongbox_keystore` is
/// globally hidden in the current boot.
///
/// Android's `SystemConfig` feature table is immutable once initialized, so a
/// definitive `package_native::hasSystemFeature` verdict is cached for the rest
/// of the process lifetime. While `package_native` is not yet registered during
/// early boot, or a probe fails transiently, this falls back to the local mask
/// files without caching a verdict. After a failed probe, the next probe is
/// delayed by at least `PM_PROBE_RETRY_MS`.
pub struct StrongboxFeatureCache {
    state: AtomicU8,
    last_failed_ms: AtomicI64,
}

impl Default for StrongboxFeatureCache {
    fn default() -> Self {
        Self::new()
    }
}

impl StrongboxFeatureCache {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(STATE_UNPROBED),
            last_failed_ms: AtomicI64::new(0),
        }
    }

    pub fn is_globally_hidden_with(
        &self,
        probe_pm: impl FnOnce() -> PmFeatureProbe,
        masked_fallback: impl FnOnce() -> bool,
        now_ms: impl Fn() -> i64,
    ) -> bool {
        match self.state.load(Ordering::Acquire) {
            STATE_DECLARED => return false,
            STATE_HIDDEN => return true,
            _ => {}
        }

        let last_failed_ms = self.last_failed_ms.load(Ordering::Acquire);
        if last_failed_ms != 0 && now_ms().wrapping_sub(last_failed_ms) < PM_PROBE_RETRY_MS {
            return masked_fallback();
        }

        match probe_pm() {
            PmFeatureProbe::Resolved(true) => {
                self.state.store(STATE_DECLARED, Ordering::Release);
                false
            }
            PmFeatureProbe::Resolved(false) => {
                self.state.store(STATE_HIDDEN, Ordering::Release);
                true
            }
            PmFeatureProbe::Failed => {
                self.last_failed_ms.store(now_ms(), Ordering::Release);
                masked_fallback()
            }
            PmFeatureProbe::Unavailable => masked_fallback(),
        }
    }
}

/// `package_native::hasSystemFeature(STRONGBOX_FEATURE)` transaction shared by
/// the keymint daemon and the injector. The caller must satisfy its own binder
/// environment first: the daemon returns `Unavailable` until `ProcessState` is
/// initialized, and the injector wraps the call in its bypass guard.
#[cfg(feature = "binder")]
pub fn probe_strongbox_system_feature() -> PmFeatureProbe {
    use rsbinder::{hub, Status, StatusCode};

    let binder = match hub::try_get_service(PACKAGE_MANAGER_NATIVE_SERVICE) {
        Ok(Some(binder)) => binder,
        Ok(None)
        | Err(StatusCode::DeadObject | StatusCode::FailedTransaction | StatusCode::NameNotFound) => {
            return PmFeatureProbe::Unavailable;
        }
        Err(error) => {
            log::warn!(
                "failed to query {PACKAGE_MANAGER_NATIVE_SERVICE} for StrongBox feature: {error:?}"
            );
            return PmFeatureProbe::Failed;
        }
    };

    let descriptor = binder.descriptor();
    if descriptor != PACKAGE_MANAGER_NATIVE_DESCRIPTOR {
        log::warn!("unexpected {PACKAGE_MANAGER_NATIVE_SERVICE} descriptor: {descriptor}");
        return PmFeatureProbe::Failed;
    }
    let Some(proxy) = binder.as_proxy() else {
        log::warn!("{PACKAGE_MANAGER_NATIVE_SERVICE} binder was unexpectedly local");
        return PmFeatureProbe::Failed;
    };
    let mut data = match proxy.prepare_transact(true) {
        Ok(data) => data,
        Err(error) => {
            log::warn!("failed to prepare PackageManager hasSystemFeature transaction: {error:?}");
            return PmFeatureProbe::Failed;
        }
    };
    if let Err(error) = data
        .write(STRONGBOX_FEATURE)
        .and_then(|()| data.write(&0_i32))
    {
        log::warn!("failed to encode PackageManager hasSystemFeature arguments: {error:?}");
        return PmFeatureProbe::Failed;
    }

    let code = rsbinder::FIRST_CALL_TRANSACTION
        + pm_has_system_feature_offset(crate::android_version::android_major_version());
    let mut reply = match proxy.submit_transact(code, &data, rsbinder::FLAG_CLEAR_BUF) {
        Ok(Some(reply)) => reply,
        Ok(None) => {
            log::warn!("PackageManager hasSystemFeature returned no reply");
            return PmFeatureProbe::Failed;
        }
        Err(StatusCode::DeadObject | StatusCode::FailedTransaction) => {
            return PmFeatureProbe::Unavailable;
        }
        Err(error) => {
            log::warn!("PackageManager hasSystemFeature transact failed: {error:?}");
            return PmFeatureProbe::Failed;
        }
    };
    reply.set_data_position(0);

    let status: Status = match reply.read() {
        Ok(status) => status,
        Err(error) => {
            log::warn!("failed to decode PackageManager hasSystemFeature status: {error:?}");
            return PmFeatureProbe::Failed;
        }
    };
    if !status.is_ok() {
        log::warn!("PackageManager hasSystemFeature returned non-ok status: {status}");
        return PmFeatureProbe::Failed;
    }
    match reply.read::<bool>() {
        Ok(declared) => PmFeatureProbe::Resolved(declared),
        Err(error) => {
            log::warn!("failed to decode PackageManager hasSystemFeature result: {error:?}");
            PmFeatureProbe::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, AtomicUsize};

    #[test]
    fn switches_mask_only_the_selected_features() {
        for strongbox in [false, true] {
            for device_id_attestation in [false, true] {
                let text = format!("version=1\nstrongbox={strongbox}\ndevice_id_attestation={device_id_attestation}\n");
                let config = Config::parse(&text).unwrap();
                let xml = config.permissions_xml();
                assert_eq!(xml.contains(STRONGBOX_FEATURE), strongbox);
                assert_eq!(
                    xml.contains(DEVICE_ID_ATTESTATION_FEATURE),
                    device_id_attestation
                );
                assert!(!xml.contains("app_attest_key"));
                assert_eq!(
                    xml.matches("<unavailable-feature ").count(),
                    usize::from(strongbox) + usize::from(device_id_attestation)
                );
                assert_eq!(Config::from_xml(&xml).unwrap(), config);
            }
        }
    }

    #[test]
    fn malformed_or_ambiguous_policies_are_never_partially_applied() {
        let valid = "version=1\nstrongbox=false\ndevice_id_attestation=true\n";
        assert!(Config::parse(&valid.replace('\n', "\r\n")).is_ok());
        for invalid in [
            String::new(),
            valid.replace("version=1", "version=2"),
            valid.replace("strongbox=false", "strongbox=0"),
            valid.replace("strongbox=false", "strongbox=false\nstrongbox=true"),
            valid.replace("device_id_attestation=true", "other=true"),
            "# comment only\n".into(),
            "x".repeat(MAX_CONFIG + 1),
        ] {
            assert!(Config::parse(&invalid).is_err(), "accepted {invalid:?}");
        }
        assert!(Config::from_xml("<permissions><feature name=\"other\" /></permissions>").is_err());
    }

    #[test]
    fn packaged_xml_keeps_both_features_unmodified_by_default() {
        let xml = include_str!("../../template/system/etc/permissions/ommega-capability-mask.xml");
        assert_eq!(xml, Config::default().permissions_xml());
    }

    #[test]
    fn mounted_config_reader_handles_missing_valid_and_invalid_xml() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("ommega-cap-mask-test-{}.xml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert_eq!(mounted_config_from_path(&path), None);

        let masked = Config {
            strongbox: true,
            device_id_attestation: false,
        };
        std::fs::write(&path, masked.permissions_xml()).unwrap();
        assert_eq!(mounted_config_from_path(&path), Some(masked));

        std::fs::write(&path, "<permissions><feature name=\"bad\" /></permissions>").unwrap();
        assert_eq!(mounted_config_from_path(&path), None);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn strongbox_mask_falls_back_to_persisted_config_when_xml_unreadable() {
        let dir = std::env::temp_dir();
        let tag = format!("ommega-cap-mask-fallback-{}", std::process::id());
        let xml = dir.join(format!("{tag}.xml"));
        let conf = dir.join(format!("{tag}.conf"));
        let _ = std::fs::remove_file(&xml);
        let _ = std::fs::remove_file(&conf);

        // Neither source readable -> not masked.
        assert!(!strongbox_masked_from_paths(&xml, &conf));

        // A masked persisted config applies when the XML cannot be read.
        std::fs::write(
            &conf,
            "version=1\nstrongbox=true\ndevice_id_attestation=true\n",
        )
        .unwrap();
        assert!(strongbox_masked_from_paths(&xml, &conf));

        // An unmasked persisted config applies the same way.
        std::fs::write(
            &conf,
            "version=1\nstrongbox=false\ndevice_id_attestation=true\n",
        )
        .unwrap();
        assert!(!strongbox_masked_from_paths(&xml, &conf));

        // A readable XML is authoritative over the persisted config.
        std::fs::write(
            &conf,
            "version=1\nstrongbox=true\ndevice_id_attestation=true\n",
        )
        .unwrap();
        std::fs::write(
            &xml,
            Config {
                strongbox: false,
                device_id_attestation: true,
            }
            .permissions_xml(),
        )
        .unwrap();
        assert!(!strongbox_masked_from_paths(&xml, &conf));

        let _ = std::fs::remove_file(&xml);
        let _ = std::fs::remove_file(&conf);
    }

    #[test]
    fn feature_cache_prefers_live_pm_and_caches_definitive_verdicts() {
        let cache = StrongboxFeatureCache::new();
        let probes = AtomicUsize::new(0);
        // Service unavailable falls back to mounted XML without caching.
        let now = || 0i64;
        assert!(cache.is_globally_hidden_with(
            || {
                probes.fetch_add(1, Ordering::Relaxed);
                PmFeatureProbe::Unavailable
            },
            || true,
            now
        ));
        assert!(!cache.is_globally_hidden_with(
            || {
                probes.fetch_add(1, Ordering::Relaxed);
                PmFeatureProbe::Unavailable
            },
            || false,
            now
        ));
        assert_eq!(probes.load(Ordering::Relaxed), 2);

        // Once PM resolves to false (hidden), the verdict is cached and PM is not called again.
        assert!(cache.is_globally_hidden_with(
            || {
                probes.fetch_add(1, Ordering::Relaxed);
                PmFeatureProbe::Resolved(false)
            },
            || false,
            now
        ));
        assert!(cache.is_globally_hidden_with(
            || panic!("resolved verdict must be cached"),
            || false,
            now
        ));
        assert_eq!(probes.load(Ordering::Relaxed), 3);

        // Once PM resolves to true (declared), the verdict is cached even if XML claims masked.
        let declared_cache = StrongboxFeatureCache::new();
        assert!(!declared_cache.is_globally_hidden_with(
            || PmFeatureProbe::Resolved(true),
            || true,
            now
        ));
        assert!(!declared_cache.is_globally_hidden_with(
            || panic!("declared verdict must be cached"),
            || true,
            now
        ));

        // A failed PM probe falls back to mounted XML and stays retryable.
        let failed_cache = StrongboxFeatureCache::new();
        let clock = AtomicI64::new(10_000);
        let now = || clock.load(Ordering::Relaxed);
        assert!(failed_cache.is_globally_hidden_with(|| PmFeatureProbe::Failed, || true, now));
        // Inside the retry window the probe is rate-limited.
        assert!(!failed_cache.is_globally_hidden_with(
            || panic!("failed PM probe is rate-limited"),
            || false,
            now
        ));
        // After the window the probe runs again and a resolved verdict caches.
        clock.store(10_000 + PM_PROBE_RETRY_MS, Ordering::Relaxed);
        assert!(failed_cache.is_globally_hidden_with(
            || PmFeatureProbe::Resolved(false),
            || false,
            now
        ));
        assert!(failed_cache.is_globally_hidden_with(
            || panic!("resolved verdict must be cached"),
            || false,
            now
        ));
    }

    #[test]
    fn failed_pm_probe_retries_after_retry_interval() {
        let cache = StrongboxFeatureCache::new();
        let probes = AtomicUsize::new(0);
        let clock = AtomicI64::new(1_000);
        let now = || clock.load(Ordering::Relaxed);
        let probe = || {
            probes.fetch_add(1, Ordering::Relaxed);
            PmFeatureProbe::Failed
        };

        assert!(cache.is_globally_hidden_with(probe, || true, now));
        // Repeated calls inside the window skip PM and use the mounted XML.
        for t in [2_000, 30_999] {
            clock.store(t, Ordering::Relaxed);
            assert!(!cache.is_globally_hidden_with(
                || panic!("probe must be rate-limited"),
                || false,
                now
            ));
        }
        assert_eq!(probes.load(Ordering::Relaxed), 1);
        // Once the interval elapses, PM is probed again.
        clock.store(31_000, Ordering::Relaxed);
        assert!(cache.is_globally_hidden_with(probe, || true, now));
        assert_eq!(probes.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn pm_has_system_feature_offsets_match_android_12_through_17() {
        for (version, offset) in [(12, 12), (13, 12), (14, 9), (15, 10), (16, 10), (17, 13)] {
            assert_eq!(pm_has_system_feature_offset(Some(version)), offset);
        }
        assert_eq!(pm_has_system_feature_offset(None), 10);
    }
}
