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
///
/// Absence is a legitimate observation, not an error: Boot SPL lives in the
/// vbmeta AVB property on many devices and has no `ro.*` property at all. A
/// missing value is recorded as `None`, so it still invalidates the cache when
/// it later appears or changes.
pub fn version_property_snapshot() -> [Option<String>; 5] {
    version_property_snapshot_with(|name| rsproperties::get::<String>(name).ok())
}

fn version_property_snapshot_with(
    read_property: impl Fn(&str) -> Option<String>,
) -> [Option<String>; 5] {
    [
        "ro.build.version.release",
        "ro.build.version.security_patch",
        "ro.vendor.build.security_patch",
        "ro.vendor.boot_security_patch",
        "ro.boot.image.build.security_patch",
    ]
    .map(|name| read_property(name).filter(|value| !value.trim().is_empty()))
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
        let values = version_property_snapshot_with(|name| Some(format!("raw:{name}")));
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
    fn absent_or_empty_properties_are_recorded_rather_than_failing_the_snapshot() {
        // A device may publish no Boot SPL property at all (the value then only
        // exists in the vbmeta AVB property), and an empty value is an
        // observation too. Neither may block the relay profile.
        let nothing = version_property_snapshot_with(|_| None);
        assert_eq!(nothing, [None, None, None, None, None]);
        let empty = version_property_snapshot_with(|_| Some(String::new()));
        assert_eq!(empty, [None, None, None, None, None]);

        let no_boot = version_property_snapshot_with(|name| {
            (name != "ro.vendor.boot_security_patch"
                && name != "ro.boot.image.build.security_patch")
                .then(|| "raw".to_string())
        });
        assert_eq!(no_boot[0].as_deref(), Some("raw"));
        assert_eq!(no_boot[1].as_deref(), Some("raw"));
        assert_eq!(no_boot[2].as_deref(), Some("raw"));
        assert_eq!(no_boot[3], None);
        assert_eq!(no_boot[4], None);

        // A later appearance still changes the snapshot, so the cache expires.
        let later = version_property_snapshot_with(|name| {
            (name == "ro.boot.image.build.security_patch").then(|| "2026-09-01".to_string())
        });
        assert_ne!(later, no_boot);
    }
}
