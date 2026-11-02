//! Pre-flight validation of the device profile before any RKP request is sent.

use anyhow::Result;

use crate::{error::RkpError, patch_level, profile::DeviceInfo};

pub fn validate_device_info_for_request(device: &DeviceInfo) -> Result<()> {
    for (field, value) in [
        ("brand", device.brand.as_str()),
        ("model", device.model.as_str()),
        ("device", device.device.as_str()),
        ("product", device.product.as_str()),
        ("manufacturer", device.manufacturer.as_str()),
        ("vb_state", device.vb_state.as_str()),
        ("security_level", device.security_level.as_str()),
        ("bootloader_state", device.bootloader_state.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(RkpError::MissingDeviceField(field).into());
        }
    }

    require_choice(
        "security_level",
        &device.security_level,
        &["tee", "strongbox"],
    )?;
    require_choice("vb_state", &device.vb_state, &["green", "yellow", "orange"])?;
    require_choice(
        "bootloader_state",
        &device.bootloader_state,
        &["locked", "unlocked"],
    )?;

    if device.os_version.trim().is_empty() && device.security_level != "strongbox" {
        return Err(RkpError::MissingDeviceField("os_version").into());
    }

    if device.fused > 1 {
        return Err(invalid("fused", "expected 0 or 1"));
    }

    for (field, value) in [
        ("boot_patch_level", device.boot_patch_level),
        ("system_patch_level", device.system_patch_level),
        ("vendor_patch_level", device.vendor_patch_level),
    ] {
        if !patch_level::is_valid(value) {
            return Err(invalid(
                field,
                "expected a valid patch level in YYYYMM or YYYYMMDD form; Android currently accepts both",
            ));
        }
    }

    if let Some(vbmeta_digest) = device.vbmeta_digest.as_deref() {
        let valid = hex::decode(vbmeta_digest).is_ok_and(|decoded| decoded.len() == 32);
        if !valid {
            return Err(invalid(
                "vbmeta_digest",
                "expected 32-byte hexadecimal data",
            ));
        }
    }

    Ok(())
}

fn require_choice(field: &'static str, value: &str, allowed: &[&str]) -> Result<()> {
    if allowed.contains(&value) {
        return Ok(());
    }
    Err(invalid(
        field,
        &format!("expected one of {}", allowed.join(", ")),
    ))
}

fn invalid(field: &'static str, reason: &str) -> anyhow::Error {
    RkpError::InvalidDeviceField {
        field,
        reason: reason.to_owned(),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::validate_device_info_for_request;
    use crate::profile::DeviceInfo;

    fn valid_device_info() -> DeviceInfo {
        DeviceInfo {
            brand: "google".into(),
            model: "Pixel".into(),
            device: "pixel".into(),
            product: "pixel".into(),
            manufacturer: "Google".into(),
            fused: 1,
            vb_state: "green".into(),
            os_version: "13".into(),
            security_level: "tee".into(),
            bootloader_state: "locked".into(),
            boot_patch_level: 20260101,
            system_patch_level: 202601,
            vendor_patch_level: 20260101,
            vbmeta_digest: Some("11".repeat(32)),
            dice_issuer: "CN=Android".into(),
            dice_subject: "CN=Android".into(),
        }
    }

    fn error_for(edit: impl FnOnce(&mut DeviceInfo)) -> String {
        let mut device = valid_device_info();
        edit(&mut device);
        validate_device_info_for_request(&device)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn accepts_valid_profile() {
        validate_device_info_for_request(&valid_device_info()).unwrap();
    }

    #[test]
    fn rejects_red_vb_state() {
        assert!(error_for(|device| device.vb_state = "red".into()).contains("vb_state"));
    }

    #[test]
    fn allows_missing_vbmeta_digest() {
        let mut device = valid_device_info();
        device.vbmeta_digest = None;
        validate_device_info_for_request(&device).unwrap();
    }

    #[test]
    fn requires_os_version_for_tee() {
        assert!(error_for(|device| device.os_version.clear()).contains("os_version"));
    }

    #[test]
    fn allows_blank_os_version_for_strongbox() {
        let mut device = valid_device_info();
        device.security_level = "strongbox".into();
        device.os_version.clear();
        validate_device_info_for_request(&device).unwrap();
    }

    #[test]
    fn accepts_six_digit_boot_patch_level() {
        let mut device = valid_device_info();
        device.boot_patch_level = 202601;
        validate_device_info_for_request(&device).unwrap();
    }

    #[test]
    fn rejects_invalid_patch_level_date() {
        assert!(
            error_for(|device| device.vendor_patch_level = 20261301).contains("vendor_patch_level")
        );
    }

    #[test]
    fn rejects_short_vbmeta_digest() {
        assert!(
            error_for(|device| device.vbmeta_digest = Some("ab".into())).contains("vbmeta_digest")
        );
    }
}
