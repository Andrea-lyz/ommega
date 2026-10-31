// SPDX-License-Identifier: Apache-2.0
//! Android keyblob version ordering. No properties, device calls or persistence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VersionError {
    RequiresUpgrade,
    Future,
}

pub(super) fn check(stored: u32, current: u32, os_version: bool) -> Result<(), VersionError> {
    // Android's documented OS-version-zero exception does not apply to SPLs.
    if os_version && current == 0 && stored != 0 {
        return Err(VersionError::RequiresUpgrade);
    }
    match stored.cmp(&current) {
        core::cmp::Ordering::Less => Err(VersionError::RequiresUpgrade),
        core::cmp::Ordering::Equal => Ok(()),
        core::cmp::Ordering::Greater => Err(VersionError::Future),
    }
}

pub(super) fn upgrade(
    stored: &mut u32,
    current: u32,
    os_version: bool,
) -> Result<bool, VersionError> {
    match check(*stored, current, os_version) {
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
        for is_os in [false, true] {
            assert_eq!(check(16, 17, is_os), Err(VersionError::RequiresUpgrade));
            assert_eq!(check(17, 17, is_os), Ok(()));
            assert_eq!(check(18, 17, is_os), Err(VersionError::Future));
        }
    }
    #[test]
    fn failed_downgrade_does_not_modify_original_version() {
        let mut stored = 20260901;
        assert_eq!(
            upgrade(&mut stored, 20260801, false),
            Err(VersionError::Future)
        );
        assert_eq!(stored, 20260901);
    }
    #[test]
    fn forward_upgrade_then_noop() {
        let mut stored = 202608;
        assert_eq!(upgrade(&mut stored, 202609, false), Ok(true));
        assert_eq!(stored, 202609);
        assert_eq!(upgrade(&mut stored, 202609, false), Ok(false));
    }
    #[test]
    fn os_zero_is_an_explicit_exception_only() {
        let mut os = 170000;
        assert_eq!(check(os, 0, true), Err(VersionError::RequiresUpgrade));
        assert_eq!(upgrade(&mut os, 0, true), Ok(true));
        assert_eq!(os, 0);
        assert_eq!(upgrade(&mut os, 0, true), Ok(false));
        let mut patch = 202609;
        assert_eq!(upgrade(&mut patch, 0, false), Err(VersionError::Future));
        assert_eq!(patch, 202609);
    }
    #[test]
    fn boundaries_do_not_wrap() {
        let mut value = 0;
        assert_eq!(upgrade(&mut value, u32::MAX, false), Ok(true));
        assert_eq!(value, u32::MAX);
        assert_eq!(
            upgrade(&mut value, u32::MAX - 1, false),
            Err(VersionError::Future)
        );
        assert_eq!(value, u32::MAX);
    }
}
