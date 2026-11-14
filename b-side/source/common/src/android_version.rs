use std::{
    string::{String, ToString},
    sync::OnceLock,
};

static ANDROID_MAJOR_VERSION: OnceLock<Option<i32>> = OnceLock::new();

pub fn android_major_version() -> Option<i32> {
    *ANDROID_MAJOR_VERSION.get_or_init(|| android_major_version_with(read_string_property))
}

pub fn android_major_version_with(read_property: impl Fn(&str) -> Option<String>) -> Option<i32> {
    read_property("ro.build.version.release_or_codename")
        .or_else(|| read_property("ro.build.version.release"))
        .and_then(|value| value.parse::<i32>().ok())
        .or_else(|| {
            read_property("ro.build.version.sdk")
                .and_then(|sdk| sdk.parse::<i32>().ok())
                .map(|sdk| match sdk {
                    31 | 32 => 12,
                    33 => 13,
                    34 => 14,
                    35 => 15,
                    36 => 16,
                    value if value >= 37 => 17,
                    _ => panic!("unrecognized or unsupported SDK version: {}", sdk),
                })
        })
}

/// Raw properties changed by the established B SPL flow. These values only
/// invalidate a HAL-sampled version cache; they are never version evidence.
pub fn version_property_snapshot() -> Result<[Option<String>; 5], &'static str> {
    version_property_snapshot_with(|name| rsproperties::get::<String>(name).ok())
}

fn version_property_snapshot_with(
    read_property: impl Fn(&str) -> Option<String>,
) -> Result<[Option<String>; 5], &'static str> {
    let values = [
        "ro.build.version.release",
        "ro.build.version.security_patch",
        "ro.vendor.build.security_patch",
        "ro.vendor.boot_security_patch",
        "ro.boot.image.build.security_patch",
    ]
    .map(read_property);
    let present = |index: usize| {
        values[index]
            .as_ref()
            .is_some_and(|value| !value.is_empty())
    };
    if !(present(0) && present(1) && present(2) && (present(3) || present(4))) {
        return Err("cannot observe B version properties for cache invalidation");
    }
    Ok(values)
}

fn read_string_property(name: &str) -> Option<String> {
    rsproperties::get::<String>(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
#[cfg(test)]
mod version_property_tests {
    use super::*;

    #[test]
    fn invalidation_snapshot_retains_raw_values_and_both_boot_aliases() {
        let values = version_property_snapshot_with(|name| Some(format!("raw:{name}"))).unwrap();
        assert_eq!(values[0].as_deref(), Some("raw:ro.build.version.release"));
        assert_eq!(
            values[3].as_deref(),
            Some("raw:ro.vendor.boot_security_patch")
        );
        assert_eq!(
            values[4].as_deref(),
            Some("raw:ro.boot.image.build.security_patch")
        );
    }

    #[test]
    fn missing_invalidation_evidence_fails_closed_but_one_boot_alias_is_enough() {
        for missing in [
            "ro.build.version.release",
            "ro.build.version.security_patch",
            "ro.vendor.build.security_patch",
        ] {
            assert!(version_property_snapshot_with(
                |name| (name != missing).then(|| "value".to_string())
            )
            .is_err());
        }
        assert!(version_property_snapshot_with(|_| None).is_err());
        assert!(version_property_snapshot_with(|_| Some(String::new())).is_err());
        for absent_boot in [
            "ro.vendor.boot_security_patch",
            "ro.boot.image.build.security_patch",
        ] {
            assert!(version_property_snapshot_with(
                |name| (name != absent_boot).then(|| "raw".to_string())
            )
            .is_ok());
        }
    }
}
