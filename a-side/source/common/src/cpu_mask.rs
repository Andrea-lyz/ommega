//! Configuration and B-side property snapshot for the exp6 application view.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CONFIG_PATH: &str = "/data/misc/keystore/ommega/data/cpu-mask.conf";
pub const PROFILE_PATH: &str = "/data/misc/keystore/ommega/data/cpu-mask-profile.conf";
pub const PROPERTIES: [&str; 13] = [
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
            version: 1,
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
        if self.version != 1 || self.properties.len() != PROPERTIES.len() {
            return Err("incomplete B property profile");
        }
        for (index, name) in PROPERTIES.iter().enumerate() {
            let value = self.properties.get(*name).ok_or("missing B property")?;
            if value.len() >= 92 || value.contains(['\0', '\r', '\n']) {
                return Err("B property exceeds the exp6 short-property layout");
            }
            if (index < 5 || index == 10) && value.is_empty() {
                return Err("missing B identity or CPU model");
            }
            // Retain exp6's verified Android KeyStore fallback contract.
            if (5..10).contains(&index) && !value.is_empty() {
                return Err("exp6 requires empty dedicated attestation properties on B");
            }
        }
        Ok(())
    }

    pub fn text(&self) -> Result<String, &'static str> {
        self.validate()?;
        let mut text = String::from("version=1\n");
        for name in PROPERTIES {
            text.push_str(&format!("{name}={}\n", self.properties[name]));
        }
        Ok(text)
    }

    pub fn parse(contents: &str) -> Result<Self, &'static str> {
        let mut properties = BTreeMap::new();
        let mut version = false;
        for line in contents.lines() {
            let (key, value) = line.split_once('=').ok_or("invalid B property profile")?;
            if key == "version" && value == "1" && !version {
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
            version: 1,
            properties,
        };
        profile.validate()?;
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            version: 1,
            properties,
        };
        assert_eq!(Profile::parse(&profile.text().unwrap()).unwrap(), profile);
        profile
            .properties
            .insert("ro.product.model".into(), "x".repeat(92));
        assert!(profile.validate().is_err());
        profile.properties.remove("ro.product.model");
        assert!(profile.validate().is_err());
    }
}
