//! Configuration and B-side property snapshot for the exp6 application view.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CONFIG_PATH: &str = "/data/misc/keystore/ommega/data/cpu-mask.conf";
pub const PROFILE_PATH: &str = "/data/misc/keystore/ommega/data/cpu-mask-profile.conf";
pub const PROFILE_VERSION: u32 = 2;
pub const MAX_PROFILE_BYTES: usize = 16 * 1024;
pub const PROPERTIES: [&str; 95] = [
    "ro.product.brand",
    "ro.product.device",
    "ro.product.name",
    "ro.product.manufacturer",
    "ro.product.model",
    "ro.product.brand_for_attestation",
    "ro.product.device_for_attestation",
    "ro.product.name_for_attestation",
    "ro.product.manufacturer_for_attestation",
    "ro.product.model_for_attestation",
    "ro.soc.model",
    "ro.product.oplus.cpuinfo",
    "ro.build.device_family",
    "ro.build.fingerprint",
    "ro.build.id",
    "ro.build.display.id",
    "ro.build.version.incremental",
    "ro.build.type",
    "ro.build.tags",
    "ro.build.user",
    "ro.build.host",
    "ro.build.date.utc",
    "ro.build.flavor",
    "ro.build.product",
    "ro.build.description",
    "ro.product.board",
    "ro.board.platform",
    "ro.hardware",
    "ro.bootloader",
    "ro.soc.manufacturer",
    "ro.build.version.release",
    "ro.build.version.release_or_codename",
    "ro.build.version.sdk",
    "ro.build.version.security_patch",
    "ro.build.version.base_os",
    "ro.build.version.codename",
    "ro.build.version.preview_sdk",
    "ro.product.first_api_level",
    "ro.build.version.min_supported_target_sdk",
    "ro.product.cpu.abilist",
    "ro.product.cpu.abilist32",
    "ro.product.cpu.abilist64",
    "ro.system.build.fingerprint",
    "ro.vendor.build.fingerprint",
    "ro.odm.build.fingerprint",
    "ro.product.build.fingerprint",
    "ro.system_ext.build.fingerprint",
    "ro.bootimage.build.fingerprint",
    "ro.build.version.release_or_preview_display",
    "ro.product.cpu.abi",
    "ro.product.cpu.abi2",
    "ro.build.characteristics",
    "ro.build.version.sdk_full",
    "ro.build.version.all_codenames",
    "ro.build.version.preview_sdk_fingerprint",
    "ro.product.bootimage.brand",
    "ro.product.bootimage.device",
    "ro.product.bootimage.name",
    "ro.product.bootimage.manufacturer",
    "ro.product.bootimage.model",
    "ro.product.odm.brand",
    "ro.product.odm.device",
    "ro.product.odm.name",
    "ro.product.odm.manufacturer",
    "ro.product.odm.model",
    "ro.product.product.brand",
    "ro.product.product.device",
    "ro.product.product.name",
    "ro.product.product.manufacturer",
    "ro.product.product.model",
    "ro.product.system.brand",
    "ro.product.system.device",
    "ro.product.system.name",
    "ro.product.system.manufacturer",
    "ro.product.system.model",
    "ro.product.system_ext.brand",
    "ro.product.system_ext.device",
    "ro.product.system_ext.name",
    "ro.product.system_ext.manufacturer",
    "ro.product.system_ext.model",
    "ro.product.vendor.brand",
    "ro.product.vendor.device",
    "ro.product.vendor.name",
    "ro.product.vendor.manufacturer",
    "ro.product.vendor.model",
    "ro.product.odm_dlkm.brand",
    "ro.product.odm_dlkm.device",
    "ro.product.odm_dlkm.name",
    "ro.product.odm_dlkm.manufacturer",
    "ro.product.odm_dlkm.model",
    "ro.product.vendor_dlkm.brand",
    "ro.product.vendor_dlkm.device",
    "ro.product.vendor_dlkm.name",
    "ro.product.vendor_dlkm.manufacturer",
    "ro.product.vendor_dlkm.model",
];

pub fn valid_package(package: &str) -> bool {
    !package.is_empty()
        && package.len() <= 255
        && package.contains('.')
        && package.split('.').all(|part| {
            part.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        })
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub packages: BTreeSet<String>,
}

impl Config {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.packages.len() > 4096 || self.packages.iter().any(|p| !valid_package(p)) {
            return Err("invalid CPU mask scope");
        }
        Ok(())
    }

    pub fn parse(contents: &str) -> Result<Self, &'static str> {
        let mut config = Self::default();
        let (mut version, mut enabled) = (false, false);
        for line in contents.lines() {
            let (key, value) = line.split_once('=').ok_or("invalid CPU mask setting")?;
            match (key, value) {
                ("version", "1") if !version => version = true,
                ("enabled", "true" | "false") if !enabled => {
                    enabled = true;
                    config.enabled = value == "true";
                }
                ("package", value) if config.packages.len() < 4096 => {
                    if !config.packages.insert(value.to_string()) {
                        return Err("duplicate CPU mask package");
                    }
                }
                _ => return Err("unsupported CPU mask setting"),
            }
        }
        if !version || !enabled {
            return Err("incomplete CPU mask config");
        }
        config.validate()?;
        Ok(config)
    }

    pub fn text(&self) -> Result<String, &'static str> {
        self.validate()?;
        let mut text = format!("version=1\nenabled={}\n", self.enabled);
        for package in &self.packages {
            text.push_str(&format!("package={package}\n"));
        }
        Ok(text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub version: u32,
    pub properties: BTreeMap<String, String>,
}

impl Profile {
    /// Properties only. No key generation, Binder, HAL, or TEE operation.
    pub fn read_device() -> Self {
        Self {
            version: PROFILE_VERSION,
            properties: PROPERTIES
                .into_iter()
                .map(|name| {
                    let value = rsproperties::get::<String>(name).unwrap_or_default();
                    (name.to_string(), value)
                })
                .collect(),
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != PROFILE_VERSION || self.properties.len() != PROPERTIES.len() {
            return Err("incomplete B property profile");
        }
        for (index, name) in PROPERTIES.iter().enumerate() {
            let value = self.properties.get(*name).ok_or("missing B property")?;
            if value.len() >= 92 || value.contains(['\0', '\r', '\n']) {
                return Err("B property exceeds the exp6 short-property layout");
            }
            let required = index < 5
                || index == 10
                || matches!(
                    *name,
                    "ro.build.fingerprint"
                        | "ro.build.id"
                        | "ro.build.version.release"
                        | "ro.build.version.sdk"
                        | "ro.product.cpu.abilist"
                );
            if required && value.is_empty() {
                return Err("missing B identity or CPU model");
            }
            // Retain exp6's verified Android KeyStore fallback contract.
            if (5..10).contains(&index) && !value.is_empty() {
                return Err("exp6 requires empty dedicated attestation properties on B");
            }
        }
        for name in [
            "ro.build.version.sdk",
            "ro.build.version.preview_sdk",
            "ro.product.first_api_level",
            "ro.build.version.min_supported_target_sdk",
        ] {
            let value = &self.properties[name];
            if (!value.is_empty()
                && (!value.bytes().all(|c| c.is_ascii_digit()) || value.parse::<i32>().is_err()))
                || (name == "ro.build.version.sdk"
                    && value
                        .parse::<u32>()
                        .ok()
                        .is_none_or(|n| n == 0 || n > i32::MAX as u32))
            {
                return Err("invalid B API level");
            }
        }
        full_sdk_version(
            &self.properties["ro.build.version.sdk_full"],
            self.properties["ro.build.version.sdk"]
                .parse()
                .map_err(|_| "invalid B API level")?,
        )?;
        for name in [
            "ro.product.cpu.abilist",
            "ro.product.cpu.abilist32",
            "ro.product.cpu.abilist64",
        ] {
            let value = &self.properties[name];
            if !value.is_empty()
                && value.split(',').any(|abi| {
                    abi.is_empty()
                        || !abi
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                })
            {
                return Err("invalid B ABI list");
            }
        }
        if !self.properties["ro.build.date.utc"]
            .bytes()
            .all(|c| c.is_ascii_digit())
            || self.properties["ro.build.date.utc"]
                .parse::<u64>()
                .ok()
                .is_none_or(|seconds| seconds > i64::MAX as u64 / 1000)
        {
            return Err("invalid B build timestamp");
        }
        Ok(())
    }

    pub fn text(&self) -> Result<String, &'static str> {
        self.validate()?;
        let mut text = format!("version={PROFILE_VERSION}\n");
        for name in PROPERTIES {
            text.push_str(&format!("{name}={}\n", self.properties[name]));
        }
        Ok(text)
    }

    pub fn parse(contents: &str) -> Result<Self, &'static str> {
        if contents.len() > MAX_PROFILE_BYTES {
            return Err("B property profile too large");
        }
        let mut properties = BTreeMap::new();
        let mut version = false;
        for line in contents.lines() {
            let (key, value) = line.split_once('=').ok_or("invalid B property profile")?;
            if key == "version" && value == "2" && !version {
                version = true;
            } else if PROPERTIES.contains(&key) {
                if properties
                    .insert(key.to_string(), value.to_string())
                    .is_some()
                {
                    return Err("duplicate B property");
                }
            } else {
                return Err("unsupported B property profile");
            }
        }
        if !version {
            return Err("missing B profile version");
        }
        let profile = Self {
            version: PROFILE_VERSION,
            properties,
        };
        profile.validate()?;
        Ok(profile)
    }
}

fn full_sdk_version(value: &str, sdk: u32) -> Result<u32, &'static str> {
    if !value.bytes().all(|c| c.is_ascii_digit() || c == b'.') {
        return Err("invalid B full SDK version");
    }
    let (major, minor) = if value.is_empty() {
        (sdk, 0)
    } else {
        let (major, minor) = value.split_once('.').unwrap_or((value, "0"));
        (
            major
                .parse::<u32>()
                .map_err(|_| "invalid B full SDK version")?,
            minor
                .parse::<u32>()
                .map_err(|_| "invalid B full SDK version")?,
        )
    };
    if major >= 21_474 || minor >= 100_000 || major != sdk {
        return Err("inconsistent B full SDK version");
    }
    Ok(major * 100_000 + minor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_sdk_matches_android_major_minor_contract() {
        assert_eq!(full_sdk_version("36.1", 36), Ok(3_600_001));
        assert_eq!(full_sdk_version("", 31), Ok(3_100_000));
        for value in ["37.0", "36.100000", "-1", "36.x", "36.1.0", "21474"] {
            assert!(full_sdk_version(value, 36).is_err(), "{value}");
        }
    }

    #[test]
    fn scope_roundtrip_and_strict_validation() {
        let source =
            "version=1\nenabled=true\npackage=com.example.first\npackage=com.example.second\n";
        let config = Config::parse(source).unwrap();
        assert_eq!(config.text().unwrap(), source);
        for invalid in [
            "version=1\nenabled=true\npackage=../data\n",
            "version=1\nenabled=true\nenabled=false\n",
            "enabled=true\n",
            "version=1\nenabled=true\npackage=com.a\npackage=com.a\n",
        ] {
            assert!(Config::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn b_profile_is_complete_and_preserves_lengths() {
        let mut properties = BTreeMap::new();
        for (index, key) in PROPERTIES.iter().enumerate() {
            properties.insert(
                (*key).into(),
                if (5..10).contains(&index) {
                    String::new()
                } else {
                    format!("B-value-{index}")
                },
            );
        }
        let mut profile = Profile {
            version: PROFILE_VERSION,
            properties,
        };
        for (name, value) in [
            ("ro.build.version.sdk", "36"),
            ("ro.build.version.preview_sdk", "0"),
            ("ro.product.first_api_level", "31"),
            ("ro.build.version.min_supported_target_sdk", "26"),
            ("ro.build.date.utc", "1765979644"),
            ("ro.build.version.sdk_full", "36.1"),
        ] {
            profile.properties.insert(name.into(), value.into());
        }
        assert_eq!(Profile::parse(&profile.text().unwrap()).unwrap(), profile);
        let saved = profile.properties["ro.build.version.sdk"].clone();
        profile
            .properties
            .insert("ro.build.version.sdk".into(), "bogus".into());
        assert!(profile.validate().is_err());
        profile
            .properties
            .insert("ro.build.version.sdk".into(), saved);
        let mut stale = profile.clone();
        stale.version = 1;
        assert!(stale.validate().is_err());
        assert!(
            Profile::parse(&profile.text().unwrap().replace("version=2", "version=1")).is_err()
        );
        profile
            .properties
            .insert("ro.product.model".into(), "x".repeat(92));
        assert!(profile.validate().is_err());
        profile.properties.remove("ro.product.model");
        assert!(profile.validate().is_err());
    }
}
