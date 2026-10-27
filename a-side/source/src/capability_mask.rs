//! Boot-time system feature declarations, independent of KeyMint routing.

pub const CONFIG_PATH: &str = "/data/misc/keystore/ommega/data/system-capability-mask.conf";
pub const XML_PATH: &str = "system/etc/permissions/ommega-capability-mask.xml";
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
            xml.push_str(
                "    <unavailable-feature name=\"android.hardware.strongbox_keystore\" />\n",
            );
        }
        if self.device_id_attestation {
            xml.push_str(
                "    <unavailable-feature name=\"android.software.device_id_attestation\" />\n",
            );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_mask_only_the_selected_features() {
        for strongbox in [false, true] {
            for device_id_attestation in [false, true] {
                let text = format!("version=1\nstrongbox={strongbox}\ndevice_id_attestation={device_id_attestation}\n");
                let config = Config::parse(&text).unwrap();
                let xml = config.permissions_xml();
                assert_eq!(
                    xml.contains("android.hardware.strongbox_keystore"),
                    strongbox
                );
                assert_eq!(
                    xml.contains("android.software.device_id_attestation"),
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
        let xml = include_str!("../template/system/etc/permissions/ommega-capability-mask.xml");
        assert_eq!(xml, Config::default().permissions_xml());
    }
}
