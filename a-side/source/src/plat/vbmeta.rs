use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{mpsc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context, Result};
use kmr_common::crypto::{Rng, Sha256};
use kmr_crypto_ommega::{rng::OmmegaRng, sha256::OmmegaSha256};
use kmr_ta::device::KeyMintVersionTags;
use rsbinder::Strong;
use serde::Serialize;

use crate::{
    android::{
        hardware::security::keymint::{
            Algorithm::Algorithm, Digest::Digest, EcCurve::EcCurve, KeyParameter::KeyParameter,
            KeyParameterValue::KeyParameterValue, KeyPurpose::KeyPurpose,
            SecurityLevel::SecurityLevel, Tag::Tag,
        },
        system::keystore2::{
            Domain::Domain, IKeystoreSecurityLevel::IKeystoreSecurityLevel,
            KeyDescriptor::KeyDescriptor, KeyMetadata::KeyMetadata,
        },
    },
    config::{validate_trust_config, ConfigFile, ResolvedTrust, TrustValueSource, TrustValueSpec},
    plat::{attestation, resetprop, utils::get_keystore_service},
};

const OS_VERSION_PROPS: &[&str] = &[
    "ro.build.version.release",
    "ro.system.build.version.release",
];
const SECURITY_PATCH_PROPS: &[&str] = &[
    "ro.build.version.security_patch",
    "ro.system.build.version.security_patch",
];
const VENDOR_PATCH_PROPS: &[&str] = &[
    "ro.vendor.build.security_patch",
    "ro.vendor.build.version.security_patch",
];
const VBMETA_KEY_PROP: &str = "ro.boot.vbmeta.public_key_digest";
const VBMETA_HASH_PROP: &str = "ro.boot.vbmeta.digest";
const ORIGINAL_HASH_TIMEOUT: Duration = Duration::from_secs(5);
const AVB_HEADER_SIZE: usize = 256;
const AVB_FOOTER_SIZE: usize = 64;
const AVB_MAX_VBMETA_SIZE: usize = 64 * 1024;
const AVB_VERSION_MAJOR: u32 = 1;
const AVB_VERSION_MINOR: u32 = 4;
const AVB_DESCRIPTOR_SIZE: usize = 16;
const AVB_PROPERTY_DESCRIPTOR_SIZE: usize = 32;
const AVB_PROPERTY_TAG: u64 = 0;
const BOOT_AVB_PROPERTY: &[u8] = b"com.android.build.boot.security_patch";
const BOOT_HEADER_PREFIX_SIZE: usize = 48;
const BOOT_HEADER_VERSION_OFFSET: usize = 40;
const BOOT_HEADER_V3_OS_VERSION_OFFSET: usize = 16;
const BOOT_HEADER_LEGACY_OS_VERSION_OFFSET: usize = 44;
// Never read a System version from Vendor/Product, or a Vendor SPL from System.
const SYSTEM_BUILD_PROP_PATHS: &[&str] = &["/system/build.prop", "/system/system/build.prop"];
const VENDOR_BUILD_PROP_PATHS: &[&str] = &["/vendor/build.prop", "/vendor/default.prop"];

static NATIVE_VERSION_SNAPSHOT: OnceLock<NativeVersionSnapshot> = OnceLock::new();

#[derive(Clone)]
struct ResolvedField {
    value: [u8; 32],
    source: TrustValueSource,
}

/// Firmware observations, not claims that a hardware TA accepted these versions.
/// Raw Boot values decoded from a real header retain their native precision.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct NativeVersionField {
    value: Option<u32>,
    raw: Option<String>,
    source: Option<String>,
    state: &'static str,
    reason: Option<String>,
}

impl NativeVersionField {
    fn known(value: u32, raw: String, source: String) -> Self {
        Self {
            value: Some(value),
            raw: Some(raw),
            source: Some(source),
            state: "known",
            reason: None,
        }
    }

    fn unknown(reason: String) -> Self {
        Self {
            value: None,
            raw: None,
            source: None,
            state: "unknown",
            reason: Some(reason),
        }
    }

    fn required(&self, field: &str) -> Result<u32> {
        self.value
            .filter(|_| self.source.is_some() && self.raw.is_some() && self.state == "known")
            .ok_or_else(|| {
                anyhow!(
                    "native {field} is unknown: {}; no property/default substitute is permitted",
                    self.reason
                        .as_deref()
                        .unwrap_or("no authoritative firmware source")
                )
            })
    }
}

#[derive(Debug, Clone, Serialize)]
struct NativeVersionSnapshot {
    system: NativeVersionField,
    boot: NativeVersionField,
    vendor: NativeVersionField,
    os_version: NativeVersionField,
}

impl NativeVersionSnapshot {
    fn target(&self) -> Result<KeyMintVersionTags> {
        Ok(KeyMintVersionTags {
            os_version: self.os_version.required("OS version")?,
            os_patchlevel: self.system.required("System SPL")?,
            vendor_patchlevel: Some(self.vendor.required("Vendor SPL")?),
            boot_patchlevel: Some(self.boot.required("Boot SPL")?),
        })
    }

    fn validate_trust(&self, trust: &ResolvedTrust) -> Result<KeyMintVersionTags> {
        let target = self.target()?;
        let boot =
            crate::keymaster::keymint_device::extract_boot_patchlevel(&trust.boot_patchlevel)
                .map_err(anyhow::Error::msg)
                .context("invalid internal native Boot value")?;
        if target.boot_patchlevel != Some(boot)
            || trust.os_version != (target.os_version / 10000) as i32
            || Some(trust.security_patch.as_str()) != self.system.raw.as_deref()
            || Some(trust.os_patchlevel.as_str()) != self.system.raw.as_deref()
            || Some(trust.vendor_patchlevel.as_str()) != self.vendor.raw.as_deref()
            || Some(trust.boot_patchlevel.as_str()) != self.boot.raw.as_deref()
        {
            bail!("resolved trust differs from the frozen native version snapshot; refusing a version override");
        }
        Ok(target)
    }
}

/// Read the already-frozen startup target. This never probes the device or
/// asserts that getprop equals TA-enforced authorization values.
pub(crate) fn native_version_target(trust: &ResolvedTrust) -> Result<KeyMintVersionTags> {
    native_version_snapshot()?.validate_trust(trust)
}

/// JSON firmware block for spl-status.json. Each partition/OS record contains
/// value (wire u32 or null), raw, source, state and reason. No disk write occurs.
/// A "known" record establishes only its named firmware source, not TA acceptance.
pub(crate) fn native_spl_snapshot_json() -> Result<serde_json::Value> {
    serde_json::to_value(native_version_snapshot()?)
        .context("failed to serialize native SPL snapshot")
}

fn native_version_snapshot() -> Result<&'static NativeVersionSnapshot> {
    NATIVE_VERSION_SNAPSHOT
        .get()
        .ok_or_else(|| anyhow!("native version snapshot has not been captured at startup"))
}

pub fn bootstrap_vbmeta(config_file: &ConfigFile) -> Result<ResolvedTrust> {
    validate_trust_config(&config_file.trust)?;
    let native = NATIVE_VERSION_SNAPSHOT.get_or_init(capture_native_versions);
    // Startup needs only the firmware values that always exist in the system
    // partition. A partition-level Boot value that even the privileged helper
    // cannot read must not brick the local keystore: it is recorded as unknown,
    // keeps the previous local behaviour, and remote admission stays fail-closed
    // because native_version_target still requires every field.
    let os_version = native
        .os_version
        .required("OS version")
        .context("native-only startup requires the firmware OS version")?;
    if native.boot.value.is_none() {
        log::warn!(
            "native Boot SPL unavailable ({}); local tags keep the System level and remote native admission will be refused",
            native.boot.reason.as_deref().unwrap_or("no source")
        );
    }
    let os_version = (os_version / 10000) as i32;
    let slot_suffix = resetprop::read_string_property("ro.boot.slot_suffix").unwrap_or_default();

    let vb_key = resolve_vb_key(
        &config_file.trust.vb_key,
        config_file.trust.device_locked,
        &slot_suffix,
    );
    let vb_hash = resolve_vb_hash(&config_file.trust.vb_hash);
    let (verified_boot_state, device_locked) = honest_boot_state(
        config_file.trust.verified_boot_state,
        config_file.trust.device_locked,
        &vb_key,
        &vb_hash,
    );
    if verified_boot_state != config_file.trust.verified_boot_state
        || device_locked != config_file.trust.device_locked
    {
        log::error!(
            "root of trust is not fully resolvable (vb_key={}, vb_hash={}); reporting an unverified, unlocked record instead of the configured state={} locked={}",
            vb_key.source,
            vb_hash.source,
            config_file.trust.verified_boot_state,
            config_file.trust.device_locked
        );
    }

    sync_sysprops_if_needed(&vb_key, &vb_hash)?;

    let vb_key_hex = hex::encode(vb_key.value);
    let vb_hash_hex = hex::encode(vb_hash.value);
    log::info!(
        "Resolved vbmeta trust: vb_key={} source={} vb_hash={} source={} verified_boot_state={} device_locked={}",
        vb_key_hex,
        vb_key.source,
        vb_hash_hex,
        vb_hash.source,
        verified_boot_state,
        device_locked
    );

    Ok(ResolvedTrust {
        os_version,
        security_patch: native
            .system
            .raw
            .clone()
            .context("native System SPL is unknown")?,
        os_patchlevel: native
            .system
            .raw
            .clone()
            .context("native System SPL is unknown")?,
        vendor_patchlevel: native
            .vendor
            .raw
            .clone()
            .context("native Vendor SPL is unknown")?,
        boot_patchlevel: native
            .boot
            .raw
            .clone()
            .unwrap_or_else(|| native.system.raw.clone().unwrap_or_default()),
        vb_key: vb_key.value,
        vb_hash: vb_hash.value,
        vb_key_source: vb_key.source,
        vb_hash_source: vb_hash.source,
        verified_boot_state,
        device_locked,
    })
}

fn resolve_vb_key(spec: &TrustValueSpec, device_locked: bool, slot_suffix: &str) -> ResolvedField {
    match spec {
        TrustValueSpec::Hex(value) => ResolvedField {
            value: *value,
            source: TrustValueSource::ExplicitHex,
        },
        TrustValueSpec::Random => random_field(TrustValueSource::RandomExplicit),
        TrustValueSpec::Auto => {
            if let Some(value) = read_hex_property(VBMETA_KEY_PROP) {
                return ResolvedField {
                    value,
                    source: TrustValueSource::Property,
                };
            }

            match compute_vbmeta_public_key_digest(slot_suffix, device_locked) {
                Ok(value) => ResolvedField {
                    value,
                    source: TrustValueSource::Computed,
                },
                Err(error) => {
                    // Never invent a value here: a random digest would contradict the
                    // verified/locked claim, would change on every restart and would
                    // therefore invalidate every keyblob. Report the root of trust as
                    // unavailable and leave the properties untouched.
                    log::error!(
                        "vbmeta public key digest unavailable, reporting an unavailable root of trust: {error:#}"
                    );
                    unavailable_field()
                }
            }
        }
    }
}

fn resolve_vb_hash(spec: &TrustValueSpec) -> ResolvedField {
    match spec {
        TrustValueSpec::Hex(value) => ResolvedField {
            value: *value,
            source: TrustValueSource::ExplicitHex,
        },
        TrustValueSpec::Random => random_field(TrustValueSource::RandomExplicit),
        TrustValueSpec::Auto => {
            if let Some(value) = read_hex_property(VBMETA_HASH_PROP) {
                return ResolvedField {
                    value,
                    source: TrustValueSource::Property,
                };
            }

            match probe_original_verified_boot_hash_with_timeout(ORIGINAL_HASH_TIMEOUT) {
                Ok(value) => ResolvedField {
                    value,
                    source: TrustValueSource::Original,
                },
                Err(error) => {
                    // See `resolve_vb_key`: no fabricated hash and no property write.
                    log::error!(
                        "verified boot hash unavailable, reporting an unavailable root of trust: {error:#}"
                    );
                    unavailable_field()
                }
            }
        }
    }
}

fn random_field(source: TrustValueSource) -> ResolvedField {
    let mut rng = OmmegaRng {};
    let mut value = [0u8; 32];
    rng.fill_bytes(&mut value);
    ResolvedField { value, source }
}

/// Placeholder for a root-of-trust value that could not be resolved from the
/// device. It never backs a verified/locked claim: see [`honest_boot_state`].
fn unavailable_field() -> ResolvedField {
    ResolvedField {
        value: [0u8; 32],
        source: TrustValueSource::Unavailable,
    }
}

/// Boot state actually reported in the attestation record.
///
/// A root of trust that could not be resolved must not be reported as a verified,
/// locked device: `Verified` (green) implies a locked bootloader and a hash that
/// stays constant across boots, so placeholder values behind a green/locked claim
/// are externally detectable. A green claim without a locked bootloader is
/// rejected for the same reason.
fn honest_boot_state(
    configured_state: bool,
    configured_locked: bool,
    vb_key: &ResolvedField,
    vb_hash: &ResolvedField,
) -> (bool, bool) {
    let unavailable = matches!(vb_key.source, TrustValueSource::Unavailable)
        || matches!(vb_hash.source, TrustValueSource::Unavailable);
    if unavailable {
        return (false, false);
    }
    (configured_state && configured_locked, configured_locked)
}

fn capture_native_versions() -> NativeVersionSnapshot {
    use crate::keymaster::keymint_device::{
        extract_os_version, extract_patchlevel, extract_truncated_patchlevel,
    };
    NativeVersionSnapshot {
        os_version: read_native_build_field(
            SYSTEM_BUILD_PROP_PATHS,
            OS_VERSION_PROPS,
            extract_os_version,
        ),
        system: read_native_build_field(
            SYSTEM_BUILD_PROP_PATHS,
            SECURITY_PATCH_PROPS,
            extract_truncated_patchlevel,
        ),
        vendor: read_native_build_field(
            VENDOR_BUILD_PROP_PATHS,
            VENDOR_PATCH_PROPS,
            extract_patchlevel,
        ),
        boot: read_abl_boot_patchlevel().unwrap_or_else(|error| {
            NativeVersionField::unknown(format!("Boot AVB/header unavailable: {error:#}"))
        }),
    }
}

fn read_native_build_field(
    paths: &[&str],
    keys: &[&str],
    parse: fn(&str) -> std::result::Result<u32, String>,
) -> NativeVersionField {
    for path in paths {
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        for key in keys {
            if let Some(raw) = parse_build_prop_value(&contents, key) {
                let source = format!("firmware:{path}#{key}");
                return match parse(&raw) {
                    Ok(value) => NativeVersionField::known(value, raw, source),
                    Err(error) => NativeVersionField {
                        value: None,
                        raw: Some(raw),
                        source: Some(source),
                        state: "unknown",
                        reason: Some(error),
                    },
                };
            }
        }
    }
    NativeVersionField::unknown(format!(
        "no readable native firmware value for {} in {}",
        keys.join(" or "),
        paths.join(", ")
    ))
}

fn parse_build_prop_value(contents: &str, key: &str) -> Option<String> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .find_map(|line| {
            let (candidate_key, candidate_value) = line.split_once('=')?;
            (candidate_key.trim() == key).then(|| candidate_value.trim().to_string())
        })
        .filter(|value| !value.is_empty())
}

fn sync_sysprops_if_needed(vb_key: &ResolvedField, vb_hash: &ResolvedField) -> Result<()> {
    if vb_key.source.needs_sysprop_write() {
        let value = hex::encode(vb_key.value);
        resetprop::direct_write_and_verify_property(VBMETA_KEY_PROP, &value)?;
    }

    if vb_hash.source.needs_sysprop_write() {
        let value = hex::encode(vb_hash.value);
        resetprop::direct_write_and_verify_property(VBMETA_HASH_PROP, &value)?;
    }

    Ok(())
}

fn read_hex_property(name: &str) -> Option<[u8; 32]> {
    let value = resetprop::read_string_property(name)?;
    parse_hex_32(&value)
        .map_err(|error| {
            log::warn!("ignoring invalid property {name}={value}: {error:#}");
        })
        .ok()
}

fn parse_hex_32(value: &str) -> Result<[u8; 32]> {
    let decoded = hex::decode(value).with_context(|| format!("invalid hex value {value}"))?;
    decoded
        .try_into()
        .map_err(|decoded: Vec<u8>| anyhow!("hex value must be 32 bytes, got {}", decoded.len()))
}

impl TrustValueSource {
    fn needs_sysprop_write(self) -> bool {
        matches!(
            self,
            TrustValueSource::Computed
                | TrustValueSource::Original
                | TrustValueSource::RandomExplicit
        )
    }
}

fn read_abl_boot_patchlevel() -> Result<NativeVersionField> {
    let slot_suffix = resetprop::read_string_property("ro.boot.slot_suffix").unwrap_or_default();
    let boot_path = find_avb_partition_path(&slot_suffix, "boot")
        .context("failed to locate active boot partition")?;
    read_abl_boot_patchlevel_from_paths(find_top_level_vbmeta_path(&slot_suffix), &boot_path)
}

fn read_abl_boot_patchlevel_from_paths(
    top_level_path: Result<PathBuf>,
    boot_path: &Path,
) -> Result<NativeVersionField> {
    match top_level_path {
        Ok(path) => match load_vbmeta_blob(&path) {
            Ok(top_level) => {
                if let Some(value) = vbmeta_boot_patchlevel(
                    &top_level,
                    format!(
                        "avb:{}#com.android.build.boot.security_patch",
                        path.display()
                    ),
                )? {
                    return Ok(value);
                }
            }
            Err(error) => {
                log::debug!("top-level vbmeta unavailable; checking Boot image: {error:#}")
            }
        },
        Err(error) => log::debug!("top-level vbmeta unavailable; checking Boot image: {error:#}"),
    }

    match load_embedded_vbmeta_blob(boot_path) {
        Ok(embedded) => {
            if let Some(value) = vbmeta_boot_patchlevel(
                &embedded,
                format!(
                    "avb:{}:embedded#com.android.build.boot.security_patch",
                    boot_path.display()
                ),
            )? {
                return Ok(value);
            }
        }
        Err(error) => log::debug!(
            "embedded Boot AVB unavailable; checking native header {}: {error:#}",
            boot_path.display()
        ),
    }
    let value = read_boot_header_patchlevel(boot_path)?;
    Ok(NativeVersionField::known(
        value,
        value.to_string(),
        format!("boot-header:{}#os_version", boot_path.display()),
    ))
}

fn vbmeta_boot_patchlevel(vbmeta: &[u8], source: String) -> Result<Option<NativeVersionField>> {
    if let Some(raw) = avb_property_value(vbmeta, BOOT_AVB_PROPERTY)? {
        return Ok(Some(match parse_boot_patchlevel_property(raw) {
            Some(value) => {
                NativeVersionField::known(value, std::str::from_utf8(raw)?.to_string(), source)
            }
            None => NativeVersionField {
                value: None,
                raw: std::str::from_utf8(raw).ok().map(str::to_string),
                source: Some(source),
                state: "unknown",
                reason: Some(
                    "invalid com.android.build.boot.security_patch AVB property".to_string(),
                ),
            },
        }));
    }
    Ok(None)
}

fn avb_property_value<'a>(vbmeta: &'a [u8], key: &[u8]) -> Result<Option<&'a [u8]>> {
    let mut descriptors = avb_descriptors(vbmeta)?;
    while !descriptors.is_empty() {
        let descriptor = take_avb_descriptor(&mut descriptors)?;
        if be_u64(&descriptor[..8])? as u64 != AVB_PROPERTY_TAG
            || descriptor.len() < AVB_PROPERTY_DESCRIPTOR_SIZE
        {
            continue;
        }

        let key_len = be_u64(&descriptor[16..24])?;
        let value_len = be_u64(&descriptor[24..32])?;
        let Some(key_end) = AVB_PROPERTY_DESCRIPTOR_SIZE.checked_add(key_len) else {
            continue;
        };
        let Some(value_start) = key_end.checked_add(1) else {
            continue;
        };
        let Some(value_end) = value_start.checked_add(value_len) else {
            continue;
        };
        let Some(value_nul) = value_end.checked_add(1) else {
            continue;
        };
        if value_nul > descriptor.len()
            || descriptor.get(key_end) != Some(&0)
            || descriptor.get(value_end) != Some(&0)
        {
            continue;
        }
        if descriptor.get(AVB_PROPERTY_DESCRIPTOR_SIZE..key_end) == Some(key) {
            return Ok(descriptor.get(value_start..value_end));
        }
    }
    Ok(None)
}

fn avb_descriptors(vbmeta: &[u8]) -> Result<&[u8]> {
    let total_size = validated_vbmeta_size(vbmeta)?;
    if total_size > vbmeta.len() {
        bail!("vbmeta block sizes exceed the available data");
    }

    let auth_block_size = be_u64(&vbmeta[12..20])?;
    let aux_block_size = be_u64(&vbmeta[20..28])?;
    let descriptors_offset = be_u64(&vbmeta[96..104])?;
    let descriptors_size = be_u64(&vbmeta[104..112])?;
    let aux_start = AVB_HEADER_SIZE
        .checked_add(auth_block_size)
        .ok_or_else(|| anyhow!("vbmeta auxiliary block start overflow"))?;
    let aux_end = aux_start
        .checked_add(aux_block_size)
        .ok_or_else(|| anyhow!("vbmeta auxiliary block end overflow"))?;
    let descriptors_start = aux_start
        .checked_add(descriptors_offset)
        .ok_or_else(|| anyhow!("vbmeta descriptor start overflow"))?;
    let descriptors_end = descriptors_start
        .checked_add(descriptors_size)
        .ok_or_else(|| anyhow!("vbmeta descriptor end overflow"))?;
    if aux_end > total_size || descriptors_end > aux_end {
        bail!("vbmeta descriptor range is outside the auxiliary block");
    }
    Ok(&vbmeta[descriptors_start..descriptors_end])
}

fn take_avb_descriptor<'a>(descriptors: &mut &'a [u8]) -> Result<&'a [u8]> {
    if descriptors.len() < AVB_DESCRIPTOR_SIZE {
        bail!("truncated AVB descriptor header");
    }
    let bytes_following = be_u64(&descriptors[8..16])?;
    if bytes_following & 7 != 0 {
        bail!("AVB descriptor size is not divisible by 8");
    }
    let total_size = AVB_DESCRIPTOR_SIZE
        .checked_add(bytes_following)
        .ok_or_else(|| anyhow!("AVB descriptor size overflow"))?;
    if total_size > descriptors.len() {
        bail!("AVB descriptor exceeds the descriptor block");
    }
    let (descriptor, remaining) = descriptors.split_at(total_size);
    *descriptors = remaining;
    Ok(descriptor)
}

fn parse_boot_patchlevel_property(value: &[u8]) -> Option<u32> {
    crate::keymaster::keymint_device::extract_patchlevel(std::str::from_utf8(value).ok()?).ok()
}

fn read_boot_header_patchlevel(path: &Path) -> Result<u32> {
    let mut header = [0u8; BOOT_HEADER_PREFIX_SIZE];
    match File::open(path) {
        Ok(mut file) => {
            file.read_exact(&mut header).with_context(|| {
                format!("failed to read boot image header from {}", path.display())
            })?;
        }
        Err(direct_error) => {
            let bytes = read_native_image(path, BOOT_HEADER_PREFIX_SIZE).with_context(|| {
                format!(
                    "failed to read boot image header from {} directly ({direct_error}) or through the privileged helper",
                    path.display()
                )
            })?;
            if bytes.len() < BOOT_HEADER_PREFIX_SIZE {
                bail!(
                    "privileged helper returned only {} bytes of the Boot header",
                    bytes.len()
                );
            }
            header.copy_from_slice(&bytes[..BOOT_HEADER_PREFIX_SIZE]);
        }
    }
    boot_header_patchlevel(&header)
}

fn boot_header_patchlevel(header: &[u8]) -> Result<u32> {
    if header.len() < BOOT_HEADER_PREFIX_SIZE || &header[..8] != b"ANDROID!" {
        bail!("invalid boot image header");
    }

    let header_version =
        le_u32(&header[BOOT_HEADER_VERSION_OFFSET..BOOT_HEADER_VERSION_OFFSET + 4])?;
    let os_version_offset = if header_version >= 3 {
        if header_version > 4 {
            bail!("unsupported boot image header version {header_version}");
        }
        BOOT_HEADER_V3_OS_VERSION_OFFSET
    } else {
        BOOT_HEADER_LEGACY_OS_VERSION_OFFSET
    };
    let os_version = le_u32(&header[os_version_offset..os_version_offset + 4])?;
    let year = 2000 + ((os_version >> 4) & 0x7f);
    let month = os_version & 0xf;
    Ok(year * 10_000 + month * 100)
}

fn validated_vbmeta_size(header: &[u8]) -> Result<usize> {
    if header.len() < AVB_HEADER_SIZE || &header[..4] != b"AVB0" {
        bail!("invalid vbmeta header");
    }
    let required_major = be_u32(&header[4..8])?;
    let required_minor = be_u32(&header[8..12])?;
    if required_major != AVB_VERSION_MAJOR || required_minor > AVB_VERSION_MINOR {
        bail!("unsupported required libavb version {required_major}.{required_minor}");
    }
    if header[175] != 0 {
        bail!("vbmeta release string is not NUL-terminated");
    }

    let auth_block_size = be_u64(&header[12..20])?;
    let aux_block_size = be_u64(&header[20..28])?;
    if auth_block_size & 63 != 0 || aux_block_size & 63 != 0 {
        bail!("vbmeta inner block size is not divisible by 64");
    }
    let total_size = AVB_HEADER_SIZE
        .checked_add(auth_block_size)
        .and_then(|value| value.checked_add(aux_block_size))
        .ok_or_else(|| anyhow!("vbmeta size overflow"))?;
    if total_size > AVB_MAX_VBMETA_SIZE {
        bail!("vbmeta image exceeds the 64 KiB libavb limit");
    }
    Ok(total_size)
}

fn compute_vbmeta_public_key_digest(slot_suffix: &str, device_locked: bool) -> Result<[u8; 32]> {
    if !device_locked {
        return Ok([0u8; 32]);
    }

    let path = find_top_level_vbmeta_path(slot_suffix)?;
    let vbmeta_bytes = load_vbmeta_blob(&path)
        .with_context(|| format!("failed to read vbmeta image {}", path.display()))?;
    compute_vbmeta_public_key_digest_from_bytes(&vbmeta_bytes)
}

fn find_top_level_vbmeta_path(slot_suffix: &str) -> Result<PathBuf> {
    find_avb_partition_path(slot_suffix, "vbmeta")
}

fn find_avb_partition_path(slot_suffix: &str, partition_name: &str) -> Result<PathBuf> {
    let mut names = Vec::with_capacity(2);
    if !slot_suffix.is_empty() {
        names.push(format!("{partition_name}{slot_suffix}"));
    }
    names.push(partition_name.to_string());

    for candidate in names.into_iter().flat_map(|name| {
        [
            PathBuf::from(format!("/dev/block/by-name/{name}")),
            PathBuf::from(format!("/dev/block/bootdevice/by-name/{name}")),
        ]
    }) {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(anyhow!(
        "no {partition_name} partition found for slot suffix '{slot_suffix}'"
    ))
}

/// Read a native image the keystore uid cannot open by itself.
///
/// The daemon runs as uid 1017, which is denied on `/dev/block/by-name/*`. Boot
/// SPL is only published in the vbmeta AVB property, so ask the existing
/// privileged helper for a bounded prefix and keep AVB parsing here.
fn read_native_image(path: &Path, max_bytes: usize) -> Result<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(direct_error) => {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| anyhow!("native image path has no file name"))?;
            resetprop::read_native_partition(name, max_bytes).with_context(|| {
                format!(
                    "failed to read {} directly ({direct_error}) or through the privileged helper",
                    path.display()
                )
            })
        }
    }
}

fn load_vbmeta_blob(path: &Path) -> Result<Vec<u8>> {
    if let Ok(mut file) = File::open(path) {
        return load_vbmeta_blob_at(&mut file, 0, None)
            .with_context(|| format!("failed to load vbmeta image {}", path.display()));
    }
    let bytes = read_native_image(path, AVB_MAX_VBMETA_SIZE + AVB_HEADER_SIZE)?;
    load_vbmeta_blob_at(&mut std::io::Cursor::new(bytes), 0, None)
        .with_context(|| format!("failed to load vbmeta image {}", path.display()))
}

fn load_embedded_vbmeta_blob(path: &Path) -> Result<Vec<u8>> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open partition {}", path.display()))?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .with_context(|| format!("failed to read partition magic from {}", path.display()))?;
    if &magic == b"AVB0" {
        return load_vbmeta_blob_at(&mut file, 0, None)
            .with_context(|| format!("failed to load standalone vbmeta from {}", path.display()));
    }

    let partition_size = file
        .seek(SeekFrom::End(0))
        .with_context(|| format!("failed to determine partition size for {}", path.display()))?;
    let footer_offset = partition_size
        .checked_sub(AVB_FOOTER_SIZE as u64)
        .ok_or_else(|| anyhow!("partition is smaller than an AVB footer"))?;
    file.seek(SeekFrom::Start(footer_offset))
        .context("failed to seek to AVB footer")?;
    let mut footer = [0u8; AVB_FOOTER_SIZE];
    file.read_exact(&mut footer)
        .context("failed to read AVB footer")?;
    if &footer[..4] != b"AVBf" {
        bail!("partition has neither AVB0 header nor AVBf footer");
    }
    if be_u32(&footer[4..8])? > 1 {
        bail!("unsupported AVB footer major version");
    }

    let vbmeta_offset = be_u64(&footer[20..28])?;
    let vbmeta_size = be_u64(&footer[28..36])?;
    if vbmeta_size < AVB_HEADER_SIZE {
        bail!("AVB footer declares a vbmeta block smaller than its header");
    }
    if vbmeta_size > AVB_MAX_VBMETA_SIZE {
        bail!("AVB footer vbmeta block exceeds the 64 KiB libavb limit");
    }
    let vbmeta_offset_u64 = u64::try_from(vbmeta_offset).context("vbmeta offset is too large")?;
    let vbmeta_size_u64 = u64::try_from(vbmeta_size).context("vbmeta size is too large")?;
    let vbmeta_end = vbmeta_offset_u64
        .checked_add(vbmeta_size_u64)
        .ok_or_else(|| anyhow!("embedded vbmeta range overflow"))?;
    if vbmeta_end > partition_size {
        bail!("embedded vbmeta range exceeds the partition");
    }
    load_vbmeta_blob_at(&mut file, vbmeta_offset_u64, Some(vbmeta_size))
        .with_context(|| format!("failed to load embedded vbmeta from {}", path.display()))
}

fn load_vbmeta_blob_at<R: Read + std::io::Seek>(
    file: &mut R,
    offset: u64,
    maximum_size: Option<usize>,
) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))
        .context("failed to seek to vbmeta header")?;
    let mut header = [0u8; AVB_HEADER_SIZE];
    file.read_exact(&mut header)
        .context("failed to read vbmeta header")?;

    let total_size = validated_vbmeta_size(&header)?;
    if let Some(maximum_size) = maximum_size {
        if total_size > maximum_size {
            bail!("vbmeta header exceeds its containing block");
        }
    }

    let mut blob = vec![0u8; total_size];
    blob[..AVB_HEADER_SIZE].copy_from_slice(&header);
    file.read_exact(&mut blob[AVB_HEADER_SIZE..])
        .with_context(|| {
            format!(
                "failed to read {} bytes of vbmeta data",
                total_size - AVB_HEADER_SIZE
            )
        })?;
    Ok(blob)
}

fn compute_vbmeta_public_key_digest_from_bytes(vbmeta_bytes: &[u8]) -> Result<[u8; 32]> {
    if vbmeta_bytes.len() < AVB_HEADER_SIZE {
        bail!("vbmeta blob too small");
    }
    if &vbmeta_bytes[..4] != b"AVB0" {
        bail!("vbmeta blob missing AVB0 magic");
    }

    let auth_block_size = be_u64(&vbmeta_bytes[12..20])?;
    let public_key_offset = be_u64(&vbmeta_bytes[64..72])?;
    let public_key_size = be_u64(&vbmeta_bytes[72..80])?;
    if public_key_size == 0 {
        bail!("vbmeta public key size is zero");
    }

    let key_start = AVB_HEADER_SIZE
        .checked_add(auth_block_size)
        .and_then(|value| value.checked_add(public_key_offset))
        .ok_or_else(|| anyhow!("vbmeta public key start overflow"))?;
    let key_end = key_start
        .checked_add(public_key_size)
        .ok_or_else(|| anyhow!("vbmeta public key end overflow"))?;
    if key_end > vbmeta_bytes.len() {
        bail!(
            "vbmeta public key range [{}..{}) exceeds blob length {}",
            key_start,
            key_end,
            vbmeta_bytes.len()
        );
    }

    OmmegaSha256 {}
        .hash(&vbmeta_bytes[key_start..key_end])
        .map_err(|error| anyhow!("failed to hash vbmeta public key: {error:?}"))
}

fn be_u64(bytes: &[u8]) -> Result<usize> {
    let array: [u8; 8] = bytes
        .try_into()
        .map_err(|_| anyhow!("expected 8 bytes, got {}", bytes.len()))?;
    usize::try_from(u64::from_be_bytes(array)).context("value does not fit in usize")
}

fn be_u32(bytes: &[u8]) -> Result<u32> {
    let array: [u8; 4] = bytes
        .try_into()
        .map_err(|_| anyhow!("expected 4 bytes, got {}", bytes.len()))?;
    Ok(u32::from_be_bytes(array))
}

fn le_u32(bytes: &[u8]) -> Result<u32> {
    let array: [u8; 4] = bytes
        .try_into()
        .map_err(|_| anyhow!("expected 4 bytes, got {}", bytes.len()))?;
    Ok(u32::from_le_bytes(array))
}

fn probe_original_verified_boot_hash_with_timeout(timeout: Duration) -> Result<[u8; 32]> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result =
            probe_original_verified_boot_hash_inner().map_err(|error| format!("{error:#}"));
        let _ = sender.send(result);
    });

    match receiver.recv_timeout(timeout) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(anyhow!(error)),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err(anyhow!("timed out while probing system verified boot hash"))
        }
        Err(error) => Err(anyhow!("system verified boot hash probe failed: {error}")),
    }
}

fn probe_original_verified_boot_hash_inner() -> Result<[u8; 32]> {
    let service = get_keystore_service().context("failed to connect to system keystore")?;
    let tee = service
        .getSecurityLevel(SecurityLevel::TRUSTED_ENVIRONMENT)
        .context("failed to get system TEE security level")?;

    if let Ok(metadata) = generate_blob_attested_key(&tee) {
        return extract_verified_boot_hash_from_metadata(&metadata);
    }

    let alias = format!(
        "ommega-vbhash-probe-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let descriptor = app_descriptor(&alias);
    let result = tee.generateKey(&descriptor, None, &attested_ec_params(), 0, &[]);
    let metadata = result.context("fallback APP attestation key generation failed")?;
    let extracted = extract_verified_boot_hash_from_metadata(&metadata);
    if let Err(error) = service.deleteKey(&descriptor) {
        log::warn!("failed to delete fallback APP vbhash probe key {alias}: {error:?}");
    }
    extracted
}

fn generate_blob_attested_key(
    security_level: &Strong<dyn IKeystoreSecurityLevel>,
) -> Result<KeyMetadata> {
    let descriptor = KeyDescriptor {
        domain: Domain::BLOB,
        nspace: 0,
        alias: None,
        blob: None,
    };
    security_level
        .generateKey(&descriptor, None, &attested_ec_params(), 0, &[])
        .context("BLOB attestation key generation failed")
}

fn app_descriptor(alias: &str) -> KeyDescriptor {
    KeyDescriptor {
        domain: Domain::APP,
        nspace: 0,
        alias: Some(alias.to_string()),
        blob: None,
    }
}

fn attested_ec_params() -> Vec<KeyParameter> {
    vec![
        kp(Tag::ALGORITHM, KeyParameterValue::Algorithm(Algorithm::EC)),
        kp(Tag::EC_CURVE, KeyParameterValue::EcCurve(EcCurve::P_256)),
        kp(Tag::KEY_SIZE, KeyParameterValue::Integer(256)),
        kp(Tag::DIGEST, KeyParameterValue::Digest(Digest::SHA_2_256)),
        kp(
            Tag::PURPOSE,
            KeyParameterValue::KeyPurpose(KeyPurpose::SIGN),
        ),
        kp(
            Tag::ATTESTATION_CHALLENGE,
            KeyParameterValue::Blob(b"ommega-vbmeta-probe".to_vec()),
        ),
    ]
}

fn kp(tag: Tag, value: KeyParameterValue) -> KeyParameter {
    KeyParameter { tag, value }
}

fn extract_verified_boot_hash_from_metadata(metadata: &KeyMetadata) -> Result<[u8; 32]> {
    let leaf = metadata
        .certificate
        .as_deref()
        .ok_or_else(|| anyhow!("attestation leaf certificate missing"))?;
    attestation::extract_verified_boot_hash_from_leaf_certificate(leaf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avb_public_key_digest_matches_embedded_blob_hash() {
        let public_key = b"test-public-key-material";
        let vbmeta = build_test_vbmeta(public_key, 64, 96);
        let digest = compute_vbmeta_public_key_digest_from_bytes(&vbmeta).unwrap();
        let expected = OmmegaSha256 {}.hash(public_key).unwrap();
        assert_eq!(digest, expected);
    }

    #[test]
    fn boot_patchlevel_uses_vbmeta_property_then_header() {
        let direct = build_test_vbmeta_with_descriptors(
            0,
            &build_test_property_descriptor(BOOT_AVB_PROPERTY, b"2026-02-01"),
        );
        let field = vbmeta_boot_patchlevel(&direct, "avb:test#boot".to_string())
            .unwrap()
            .unwrap();
        assert_eq!(field.value, Some(20260201));
        assert_eq!(field.raw.as_deref(), Some("2026-02-01"));
        assert_eq!(field.source.as_deref(), Some("avb:test#boot"));

        let mut duplicate = build_test_property_descriptor(BOOT_AVB_PROPERTY, b"2026-13-01");
        duplicate.extend(build_test_property_descriptor(
            BOOT_AVB_PROPERTY,
            b"2026-02-01",
        ));
        let first_invalid = build_test_vbmeta_with_descriptors(0, &duplicate);
        let invalid = vbmeta_boot_patchlevel(&first_invalid, "avb:test".to_string())
            .unwrap()
            .unwrap();
        assert!(invalid.value.is_none());
        assert_eq!(invalid.raw.as_deref(), Some("2026-13-01"));
        assert_eq!(invalid.source.as_deref(), Some("avb:test"));
        assert_eq!(invalid.state, "unknown");

        let no_property = build_test_vbmeta_with_descriptors(0, &[]);
        assert_eq!(
            vbmeta_boot_patchlevel(&no_property, "avb:test".to_string()).unwrap(),
            None
        );
    }

    #[test]
    fn boot_patchlevel_uses_embedded_vbmeta_before_header() {
        let dir = tempfile::tempdir().unwrap();
        let top_level_path = dir.path().join("vbmeta");
        let boot_path = dir.path().join("boot");
        std::fs::write(&top_level_path, build_test_vbmeta_with_descriptors(0, &[])).unwrap();

        let embedded = build_test_vbmeta_with_descriptors(
            0,
            &build_test_property_descriptor(BOOT_AVB_PROPERTY, b"2026-02-01"),
        );
        std::fs::write(&boot_path, build_test_boot_image(Some(&embedded))).unwrap();
        assert_eq!(
            read_abl_boot_patchlevel_from_paths(Ok(top_level_path.clone()), &boot_path)
                .unwrap()
                .value,
            Some(20260201)
        );

        let mut zero_header = build_test_boot_image(Some(&embedded));
        zero_header[BOOT_HEADER_V3_OS_VERSION_OFFSET..BOOT_HEADER_V3_OS_VERSION_OFFSET + 4].fill(0);
        std::fs::write(&boot_path, zero_header).unwrap();
        assert_eq!(
            read_abl_boot_patchlevel_from_paths(Ok(top_level_path.clone()), &boot_path)
                .unwrap()
                .value,
            Some(20260201)
        );
        assert_eq!(
            read_abl_boot_patchlevel_from_paths(
                Err(anyhow!("missing top-level vbmeta")),
                &boot_path,
            )
            .unwrap()
            .value,
            Some(20260201)
        );

        let embedded_without_property = build_test_vbmeta_with_descriptors(0, &[]);
        std::fs::write(
            &boot_path,
            build_test_boot_image(Some(&embedded_without_property)),
        )
        .unwrap();
        assert_eq!(
            read_abl_boot_patchlevel_from_paths(Ok(top_level_path.clone()), &boot_path)
                .unwrap()
                .value,
            Some(20250900)
        );

        std::fs::write(&boot_path, build_test_boot_image(None)).unwrap();
        assert_eq!(
            read_abl_boot_patchlevel_from_paths(Ok(top_level_path.clone()), &boot_path)
                .unwrap()
                .value,
            Some(20250900)
        );

        std::fs::write(
            &top_level_path,
            build_test_vbmeta_with_descriptors(
                0,
                &build_test_property_descriptor(BOOT_AVB_PROPERTY, b"2027-01-01"),
            ),
        )
        .unwrap();
        let mut zero_header = build_test_boot_image(Some(&embedded));
        zero_header[BOOT_HEADER_V3_OS_VERSION_OFFSET..BOOT_HEADER_V3_OS_VERSION_OFFSET + 4].fill(0);
        std::fs::write(&boot_path, zero_header).unwrap();
        assert_eq!(
            read_abl_boot_patchlevel_from_paths(Ok(top_level_path), &boot_path)
                .unwrap()
                .value,
            Some(20270101)
        );
    }

    #[test]
    fn boot_header_patchlevel_decodes_legacy_and_modern_headers() {
        for (header_version, offset) in [
            (0u32, BOOT_HEADER_LEGACY_OS_VERSION_OFFSET),
            (4u32, BOOT_HEADER_V3_OS_VERSION_OFFSET),
        ] {
            let mut header = [0u8; BOOT_HEADER_PREFIX_SIZE];
            header[..8].copy_from_slice(b"ANDROID!");
            header[BOOT_HEADER_VERSION_OFFSET..BOOT_HEADER_VERSION_OFFSET + 4]
                .copy_from_slice(&header_version.to_le_bytes());
            let os_version = (25u32 << 4) | 9;
            header[offset..offset + 4].copy_from_slice(&os_version.to_le_bytes());
            assert_eq!(boot_header_patchlevel(&header).unwrap(), 20250900);

            header[offset..offset + 4].fill(0);
            assert_eq!(boot_header_patchlevel(&header).unwrap(), 20000000);
        }
    }

    #[test]
    fn boot_patchlevel_property_uses_aosp_date_parser() {
        for (value, expected) in [
            (b"2025-09-05".as_slice(), Some(20250905)),
            (b"2025-9-5".as_slice(), None),
            (b"  +2025-02-31".as_slice(), None),
            (b"2025-13-01".as_slice(), None),
            (b"2025-12-01x".as_slice(), None),
        ] {
            assert_eq!(parse_boot_patchlevel_property(value), expected, "{value:?}");
        }

        let mut oversized = vec![0u8; AVB_HEADER_SIZE];
        oversized[..4].copy_from_slice(b"AVB0");
        oversized[4..8].copy_from_slice(&AVB_VERSION_MAJOR.to_be_bytes());
        oversized[20..28].copy_from_slice(&(AVB_MAX_VBMETA_SIZE as u64).to_be_bytes());
        assert!(validated_vbmeta_size(&oversized).is_err());

        let mut avb_1_4 = vec![0u8; AVB_HEADER_SIZE];
        avb_1_4[..4].copy_from_slice(b"AVB0");
        avb_1_4[4..8].copy_from_slice(&AVB_VERSION_MAJOR.to_be_bytes());
        avb_1_4[8..12].copy_from_slice(&AVB_VERSION_MINOR.to_be_bytes());
        assert_eq!(validated_vbmeta_size(&avb_1_4).unwrap(), AVB_HEADER_SIZE);
    }

    fn test_native_snapshot() -> NativeVersionSnapshot {
        NativeVersionSnapshot {
            os_version: NativeVersionField::known(
                120103,
                "12.1.3".to_string(),
                "firmware:system#release".to_string(),
            ),
            system: NativeVersionField::known(
                202609,
                "2026-09-01".to_string(),
                "firmware:system#spl".to_string(),
            ),
            vendor: NativeVersionField::known(
                20260805,
                "2026-08-05".to_string(),
                "firmware:vendor#spl".to_string(),
            ),
            boot: NativeVersionField::known(
                20250900,
                "20250900".to_string(),
                "boot-header:boot#os_version".to_string(),
            ),
        }
    }

    fn test_native_trust(snapshot: &NativeVersionSnapshot) -> ResolvedTrust {
        ResolvedTrust {
            os_version: 12,
            security_patch: snapshot.system.raw.clone().unwrap(),
            os_patchlevel: snapshot.system.raw.clone().unwrap(),
            vendor_patchlevel: snapshot.vendor.raw.clone().unwrap(),
            boot_patchlevel: snapshot.boot.raw.clone().unwrap(),
            vb_key: [1; 32],
            vb_hash: [2; 32],
            vb_key_source: TrustValueSource::Computed,
            vb_hash_source: TrustValueSource::Original,
            verified_boot_state: true,
            device_locked: true,
        }
    }

    #[test]
    fn native_target_keeps_full_os_version_and_raw_boot_precision() {
        let snapshot = test_native_snapshot();
        let trust = test_native_trust(&snapshot);
        let target = snapshot.validate_trust(&trust).unwrap();
        assert_eq!(target.os_version, 120103);
        assert_eq!(target.os_patchlevel, 202609);
        assert_eq!(target.vendor_patchlevel, Some(20260805));
        assert_eq!(target.boot_patchlevel, Some(20250900));
        let mut changed = trust;
        changed.vendor_patchlevel = "2026-09-01".to_string();
        assert!(snapshot.validate_trust(&changed).is_err());
    }

    #[test]
    fn observed_zero_boot_is_distinct_from_an_unknown_source() {
        let mut snapshot = test_native_snapshot();
        snapshot.boot = NativeVersionField::known(
            0,
            "0".to_string(),
            "boot-header:test#native-raw".to_string(),
        );
        let trust = test_native_trust(&snapshot);
        assert_eq!(
            snapshot.validate_trust(&trust).unwrap().boot_patchlevel,
            Some(0)
        );
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["boot"]["value"], 0);
        assert_eq!(json["boot"]["raw"], "0");
        assert_eq!(json["boot"]["state"], "known");
    }

    #[test]
    fn missing_native_sources_are_unknown_not_system_or_default_substitutes() {
        for field in ["system", "vendor", "boot", "os_version"] {
            let mut snapshot = test_native_snapshot();
            let unknown = NativeVersionField::unknown("missing native source".to_string());
            match field {
                "system" => snapshot.system = unknown,
                "vendor" => snapshot.vendor = unknown,
                "boot" => snapshot.boot = unknown,
                _ => snapshot.os_version = unknown,
            }
            assert!(snapshot.target().is_err());
            let json = serde_json::to_value(&snapshot).unwrap();
            assert!(json[field]["value"].is_null());
            assert!(json[field]["raw"].is_null());
            assert!(json[field]["source"].is_null());
            assert_eq!(json[field]["state"], "unknown");
            assert_eq!(json[field]["reason"], "missing native source");
        }
    }

    #[test]
    fn firmware_fields_use_exact_partition_sources_and_report_invalid_raw() {
        use crate::keymaster::keymint_device::{extract_patchlevel, extract_truncated_patchlevel};
        let dir = tempfile::tempdir().unwrap();
        let system = dir.path().join("system-build.prop");
        let vendor = dir.path().join("vendor-build.prop");
        std::fs::write(&system, "ro.build.version.security_patch=2026-09-01\n").unwrap();
        std::fs::write(&vendor, "ro.vendor.build.security_patch=invalid\n").unwrap();
        let system_paths = [system.to_str().unwrap()];
        let vendor_paths = [vendor.to_str().unwrap()];
        let system_field = read_native_build_field(
            &system_paths,
            SECURITY_PATCH_PROPS,
            extract_truncated_patchlevel,
        );
        assert_eq!(system_field.value, Some(202609));
        assert!(system_field
            .source
            .as_deref()
            .unwrap()
            .contains("system-build.prop#ro.build.version.security_patch"));
        let invalid =
            read_native_build_field(&vendor_paths, VENDOR_PATCH_PROPS, extract_patchlevel);
        assert!(invalid.value.is_none());
        assert_eq!(invalid.raw.as_deref(), Some("invalid"));
        assert_eq!(invalid.state, "unknown");
        std::fs::write(&vendor, "# no Vendor SPL\n").unwrap();
        let missing =
            read_native_build_field(&vendor_paths, VENDOR_PATCH_PROPS, extract_patchlevel);
        assert!(missing.value.is_none());
        assert!(missing.raw.is_none());
    }

    #[test]
    fn startup_snapshot_is_not_recaptured_after_firmware_changes() {
        use crate::keymaster::keymint_device::extract_truncated_patchlevel;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("build.prop");
        std::fs::write(&path, "ro.build.version.security_patch=2026-09-01\n").unwrap();
        let paths = [path.to_str().unwrap()];
        let snapshot = OnceLock::new();
        let first = snapshot
            .get_or_init(|| {
                read_native_build_field(&paths, SECURITY_PATCH_PROPS, extract_truncated_patchlevel)
            })
            .clone();
        std::fs::write(&path, "ro.build.version.security_patch=2099-01-01\n").unwrap();
        assert_eq!(
            snapshot.get_or_init(|| panic!("native snapshot must never be recaptured")),
            &first
        );
        assert_eq!(first.value, Some(202609));
    }

    #[test]
    fn build_prop_parser_uses_exact_keys_without_property_fallback() {
        let contents = "# comment\nro.vendor.build.security_patch.extra=ignored\nro.vendor.build.security_patch=2026-08-05\n";
        assert_eq!(
            parse_build_prop_value(contents, VENDOR_PATCH_PROPS[0]).as_deref(),
            Some("2026-08-05")
        );
        assert!(parse_build_prop_value(contents, SECURITY_PATCH_PROPS[0]).is_none());
    }

    #[test]
    fn only_derived_sources_require_sysprop_writeback() {
        assert!(TrustValueSource::RandomExplicit.needs_sysprop_write());
        assert!(TrustValueSource::Computed.needs_sysprop_write());
        assert!(TrustValueSource::Original.needs_sysprop_write());
        assert!(!TrustValueSource::ExplicitHex.needs_sysprop_write());
        assert!(!TrustValueSource::Property.needs_sysprop_write());
        assert!(!TrustValueSource::Unavailable.needs_sysprop_write());
    }

    #[test]
    fn unresolved_trust_never_reports_a_verified_locked_device() {
        let available = ResolvedField {
            value: [0x11; 32],
            source: TrustValueSource::Computed,
        };
        let unavailable = unavailable_field();
        assert_eq!(
            honest_boot_state(true, true, &available, &available),
            (true, true)
        );
        assert_eq!(
            honest_boot_state(true, true, &unavailable, &available),
            (false, false)
        );
        assert_eq!(
            honest_boot_state(true, true, &available, &unavailable),
            (false, false)
        );
        assert_eq!(
            honest_boot_state(true, false, &available, &available),
            (false, false)
        );
    }

    fn build_test_vbmeta(
        public_key: &[u8],
        auth_block_size: usize,
        aux_block_size: usize,
    ) -> Vec<u8> {
        let total_size = AVB_HEADER_SIZE + auth_block_size + aux_block_size;
        let mut blob = vec![0u8; total_size];
        blob[..4].copy_from_slice(b"AVB0");
        blob[12..20].copy_from_slice(&(auth_block_size as u64).to_be_bytes());
        blob[20..28].copy_from_slice(&(aux_block_size as u64).to_be_bytes());
        blob[64..72].copy_from_slice(&(0u64).to_be_bytes());
        blob[72..80].copy_from_slice(&(public_key.len() as u64).to_be_bytes());
        let start = AVB_HEADER_SIZE + auth_block_size;
        blob[start..start + public_key.len()].copy_from_slice(public_key);
        blob
    }

    fn build_test_vbmeta_with_descriptors(flags: u32, descriptors: &[u8]) -> Vec<u8> {
        let aux_block_size = (descriptors.len() + 63) & !63;
        let mut blob = vec![0u8; AVB_HEADER_SIZE + aux_block_size];
        blob[..4].copy_from_slice(b"AVB0");
        blob[4..8].copy_from_slice(&AVB_VERSION_MAJOR.to_be_bytes());
        blob[20..28].copy_from_slice(&(aux_block_size as u64).to_be_bytes());
        blob[104..112].copy_from_slice(&(descriptors.len() as u64).to_be_bytes());
        blob[120..124].copy_from_slice(&flags.to_be_bytes());
        blob[AVB_HEADER_SIZE..AVB_HEADER_SIZE + descriptors.len()].copy_from_slice(descriptors);
        blob
    }

    fn build_test_property_descriptor(key: &[u8], value: &[u8]) -> Vec<u8> {
        let unpadded_size = AVB_PROPERTY_DESCRIPTOR_SIZE + key.len() + 1 + value.len() + 1;
        let total_size = (unpadded_size + 7) & !7;
        let mut descriptor = vec![0u8; total_size];
        descriptor[..8].copy_from_slice(&AVB_PROPERTY_TAG.to_be_bytes());
        descriptor[8..16]
            .copy_from_slice(&((total_size - AVB_DESCRIPTOR_SIZE) as u64).to_be_bytes());
        descriptor[16..24].copy_from_slice(&(key.len() as u64).to_be_bytes());
        descriptor[24..32].copy_from_slice(&(value.len() as u64).to_be_bytes());
        descriptor[32..32 + key.len()].copy_from_slice(key);
        let value_start = 32 + key.len() + 1;
        descriptor[value_start..value_start + value.len()].copy_from_slice(value);
        descriptor
    }

    fn build_test_boot_image(embedded_vbmeta: Option<&[u8]>) -> Vec<u8> {
        let mut image = vec![0u8; BOOT_HEADER_PREFIX_SIZE];
        image[..8].copy_from_slice(b"ANDROID!");
        image[BOOT_HEADER_VERSION_OFFSET..BOOT_HEADER_VERSION_OFFSET + 4]
            .copy_from_slice(&4u32.to_le_bytes());
        image[BOOT_HEADER_V3_OS_VERSION_OFFSET..BOOT_HEADER_V3_OS_VERSION_OFFSET + 4]
            .copy_from_slice(&((25u32 << 4) | 9).to_le_bytes());

        if let Some(vbmeta) = embedded_vbmeta {
            let vbmeta_offset = image.len();
            image.extend_from_slice(vbmeta);
            let mut footer = [0u8; AVB_FOOTER_SIZE];
            footer[..4].copy_from_slice(b"AVBf");
            footer[4..8].copy_from_slice(&1u32.to_be_bytes());
            footer[20..28].copy_from_slice(&(vbmeta_offset as u64).to_be_bytes());
            footer[28..36].copy_from_slice(&(vbmeta.len() as u64).to_be_bytes());
            image.extend_from_slice(&footer);
        }
        image
    }
}
