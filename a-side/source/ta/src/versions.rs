// SPDX-License-Identifier: Apache-2.0
//! Android keyblob version ordering. No properties, device calls or persistence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VersionError {
    RequiresUpgrade,
    Future,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VersionField {
    OsVersion,
    PatchLevel,
    BootPatchLevel,
}

pub(super) fn check(stored: u32, current: u32, field: VersionField) -> Result<(), VersionError> {
    // Android's documented OS-version-zero exception does not apply to SPLs.
    if field == VersionField::OsVersion && current == 0 && stored != 0 {
        return Err(VersionError::RequiresUpgrade);
    }
    match stored.cmp(&current) {
        core::cmp::Ordering::Less => Err(VersionError::RequiresUpgrade),
        core::cmp::Ordering::Equal => Ok(()),
        core::cmp::Ordering::Greater if field == VersionField::BootPatchLevel => Ok(()),
        core::cmp::Ordering::Greater => Err(VersionError::Future),
    }
}

pub(super) fn upgrade(
    stored: &mut u32,
    current: u32,
    field: VersionField,
) -> Result<bool, VersionError> {
    match check(*stored, current, field) {
        Ok(()) => Ok(false),
        Err(VersionError::RequiresUpgrade) => {
            *stored = current;
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_equal_and_future_match_android() {
        for field in [VersionField::PatchLevel, VersionField::OsVersion] {
            assert_eq!(check(16, 17, field), Err(VersionError::RequiresUpgrade));
            assert_eq!(check(17, 17, field), Ok(()));
            assert_eq!(check(18, 17, field), Err(VersionError::Future));
        }
        assert_eq!(
            check(16, 17, VersionField::BootPatchLevel),
            Err(VersionError::RequiresUpgrade)
        );
        assert_eq!(check(17, 17, VersionField::BootPatchLevel), Ok(()));
        assert_eq!(check(18, 17, VersionField::BootPatchLevel), Ok(()));
    }
    #[test]
    fn failed_downgrade_does_not_modify_original_version() {
        let mut stored = 20260901;
        assert_eq!(
            upgrade(&mut stored, 20260801, VersionField::PatchLevel),
            Err(VersionError::Future)
        );
        assert_eq!(stored, 20260901);
    }
    #[test]
    fn forward_upgrade_then_noop() {
        let mut stored = 202608;
        assert_eq!(
            upgrade(&mut stored, 202609, VersionField::PatchLevel),
            Ok(true)
        );
        assert_eq!(stored, 202609);
        assert_eq!(
            upgrade(&mut stored, 202609, VersionField::PatchLevel),
            Ok(false)
        );
    }
    #[test]
    fn os_zero_is_an_explicit_exception_only() {
        let mut os = 170000;
        assert_eq!(
            check(os, 0, VersionField::OsVersion),
            Err(VersionError::RequiresUpgrade)
        );
        assert_eq!(upgrade(&mut os, 0, VersionField::OsVersion), Ok(true));
        assert_eq!(os, 0);
        assert_eq!(upgrade(&mut os, 0, VersionField::OsVersion), Ok(false));
        let mut patch = 202609;
        assert_eq!(
            upgrade(&mut patch, 0, VersionField::PatchLevel),
            Err(VersionError::Future)
        );
        assert_eq!(patch, 202609);
    }
    #[test]
    fn boundaries_do_not_wrap() {
        let mut value = 0;
        assert_eq!(
            upgrade(&mut value, u32::MAX, VersionField::PatchLevel),
            Ok(true)
        );
        assert_eq!(value, u32::MAX);
        assert_eq!(
            upgrade(&mut value, u32::MAX - 1, VersionField::PatchLevel),
            Err(VersionError::Future)
        );
        assert_eq!(value, u32::MAX);
    }
    #[test]
    fn future_boot_patchlevel_is_retained_without_modification() {
        let mut stored = 20260901;
        assert_eq!(
            upgrade(&mut stored, 20260801, VersionField::BootPatchLevel),
            Ok(false)
        );
        assert_eq!(stored, 20260901);
    }
}
