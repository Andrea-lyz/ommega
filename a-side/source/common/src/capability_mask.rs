//! Boot-time system feature declarations, independent of KeyMint routing.

use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};

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

pub fn mounted_strongbox_masked() -> bool {
    mounted_config_from_path(Path::new(MOUNTED_XML_PATH)).is_some_and(|config| config.strongbox)
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
const STATE_UNSUPPORTED: u8 = 3;

/// Per-process cache for whether `android.hardware.strongbox_keystore` is
/// globally hidden in the current boot.
///
/// Android's `SystemConfig` feature table is immutable once initialized, so a
/// definitive `package_native::hasSystemFeature` verdict is cached for the rest
/// of the process lifetime. While `package_native` is not yet registered during
/// early boot, this falls back to the mounted capability XML without caching.
pub struct StrongboxFeatureCache(AtomicU8);

impl Default for StrongboxFeatureCache {
    fn default() -> Self {
        Self::new()
    }
}

impl StrongboxFeatureCache {
    pub const fn new() -> Self {
        Self(AtomicU8::new(STATE_UNPROBED))
    }

    pub fn is_globally_hidden_with(
        &self,
        probe_pm: impl FnOnce() -> PmFeatureProbe,
        mounted_xml_masked: impl FnOnce() -> bool,
    ) -> bool {
        match self.0.load(Ordering::Acquire) {
            STATE_DECLARED => return false,
            STATE_HIDDEN => return true,
            STATE_UNSUPPORTED => return mounted_xml_masked(),
            _ => {}
        }

        match probe_pm() {
            PmFeatureProbe::Resolved(true) => {
                self.0.store(STATE_DECLARED, Ordering::Release);
                false
            }
            PmFeatureProbe::Resolved(false) => {
                self.0.store(STATE_HIDDEN, Ordering::Release);
                true
            }
            PmFeatureProbe::Failed => {
                self.0.store(STATE_UNSUPPORTED, Ordering::Release);
                mounted_xml_masked()
            }
            PmFeatureProbe::Unavailable => mounted_xml_masked(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

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
    fn feature_cache_prefers_live_pm_and_caches_definitive_verdicts() {
        let cache = StrongboxFeatureCache::new();
        let probes = AtomicUsize::new(0);
        // Service unavailable falls back to mounted XML without caching.
        assert!(cache.is_globally_hidden_with(
            || {
                probes.fetch_add(1, Ordering::Relaxed);
                PmFeatureProbe::Unavailable
            },
            || true
        ));
        assert!(!cache.is_globally_hidden_with(
            || {
                probes.fetch_add(1, Ordering::Relaxed);
                PmFeatureProbe::Unavailable
            },
            || false
        ));
        assert_eq!(probes.load(Ordering::Relaxed), 2);

        // Once PM resolves to false (hidden), the verdict is cached and PM is not called again.
        assert!(cache.is_globally_hidden_with(
            || {
                probes.fetch_add(1, Ordering::Relaxed);
                PmFeatureProbe::Resolved(false)
            },
            || false
        ));
        assert!(
            cache.is_globally_hidden_with(|| panic!("resolved verdict must be cached"), || false)
        );
        assert_eq!(probes.load(Ordering::Relaxed), 3);

        // Once PM resolves to true (declared), the verdict is cached even if XML claims masked.
        let declared_cache = StrongboxFeatureCache::new();
        assert!(!declared_cache.is_globally_hidden_with(|| PmFeatureProbe::Resolved(true), || true));
        assert!(!declared_cache
            .is_globally_hidden_with(|| panic!("declared verdict must be cached"), || true));

        // A persistent PM failure caches Unsupported and falls back to mounted XML.
        let failed_cache = StrongboxFeatureCache::new();
        assert!(failed_cache.is_globally_hidden_with(|| PmFeatureProbe::Failed, || true));
        assert!(!failed_cache
            .is_globally_hidden_with(|| panic!("failed PM probe must not repeat"), || false));
    }

    #[test]
    fn pm_has_system_feature_offsets_match_android_12_through_17() {
        for (version, offset) in [(12, 12), (13, 12), (14, 9), (15, 10), (16, 10), (17, 13)] {
            assert_eq!(pm_has_system_feature_offset(Some(version)), offset);
        }
        assert_eq!(pm_has_system_feature_offset(None), 10);
    }
}
