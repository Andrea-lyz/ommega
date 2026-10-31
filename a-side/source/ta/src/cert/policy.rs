// SPDX-License-Identifier: Apache-2.0
//! Checked signed-policy fields. No device IO; no rewriting of authorizations.
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub(super) struct Policy {
    pub numeric: BTreeMap<u32, i128>,
    pub ids: BTreeMap<u32, Vec<u8>>,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Mismatch(pub u32);

pub(super) fn validate(
    expected: &Policy,
    hardware: &Policy,
    software: &Policy,
) -> Result<(), Mismatch> {
    for tag in [2, 3, 10, 200] {
        if let Some(want) = expected.numeric.get(&tag) {
            if software.numeric.contains_key(&tag) || hardware.numeric.get(&tag) != Some(want) {
                return Err(Mismatch(tag));
            }
        }
    }
    for tag in [503, 504] {
        if software.numeric.contains_key(&tag)
            || expected.numeric.get(&tag) != hardware.numeric.get(&tag)
        {
            return Err(Mismatch(tag));
        }
    }
    // A timeout (including an explicit zero) and absence are different policies.
    // Do not invent a vendor exception that was not requested by the caller.
    if software.numeric.contains_key(&505)
        || expected.numeric.get(&505) != hardware.numeric.get(&505)
    {
        return Err(Mismatch(505));
    }
    // OEMs may sign these constraints in either enforcement list.
    for tag in [507, 508, 509] {
        let hw = hardware.numeric.get(&tag);
        let sw = software.numeric.get(&tag);
        if (hw.is_some() && sw.is_some()) || expected.numeric.get(&tag) != hw.or(sw) {
            return Err(Mismatch(tag));
        }
    }
    for (tag, want) in &expected.ids {
        if hardware.ids.get(tag) != Some(want) || software.ids.contains_key(tag) {
            return Err(Mismatch(*tag));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn no_auth() -> Policy {
        let mut policy = Policy::default();
        policy.numeric.extend([(2, 3), (3, 256), (10, 1), (503, 1)]);
        policy
    }
    fn authenticated() -> Policy {
        let mut policy = no_auth();
        policy.numeric.remove(&503);
        policy.numeric.extend([(504, 3), (505, 30)]);
        policy
    }
    #[test]
    fn matching_effective_policy_is_accepted() {
        let expected = no_auth();
        assert_eq!(validate(&expected, &expected, &Policy::default()), Ok(()));
    }
    #[test]
    fn signed_wrong_algorithm_size_or_curve_is_rejected() {
        let expected = no_auth();
        for tag in [2, 3, 10] {
            let mut signed = expected.clone();
            signed.numeric.insert(tag, 99);
            assert_eq!(
                validate(&expected, &signed, &Policy::default()),
                Err(Mismatch(tag))
            );
        }
    }
    #[test]
    fn rsa_public_exponent_is_compared_without_signed_truncation() {
        let mut expected = no_auth();
        expected.numeric.insert(200, u64::MAX.into());
        let mut signed = expected.clone();
        assert_eq!(validate(&expected, &signed, &Policy::default()), Ok(()));
        signed.numeric.insert(200, 65537);
        assert_eq!(
            validate(&expected, &signed, &Policy::default()),
            Err(Mismatch(200))
        );
    }
    #[test]
    fn no_auth_downgrade_and_unrequested_auth_are_rejected() {
        assert_eq!(
            validate(&authenticated(), &no_auth(), &Policy::default()),
            Err(Mismatch(503))
        );
        assert_eq!(
            validate(&no_auth(), &authenticated(), &Policy::default()),
            Err(Mismatch(503))
        );
    }
    #[test]
    fn hardware_auth_cannot_be_moved_to_software() {
        let expected = authenticated();
        for tag in [504, 505] {
            let mut hw = expected.clone();
            let mut sw = Policy::default();
            sw.numeric.insert(tag, hw.numeric.remove(&tag).unwrap());
            assert_eq!(validate(&expected, &hw, &sw), Err(Mismatch(tag)));
        }
    }
    #[test]
    fn missing_or_changed_timeout_window_is_rejected() {
        let expected = authenticated();
        let mut signed = expected.clone();
        signed.numeric.remove(&505);
        assert_eq!(
            validate(&expected, &signed, &Policy::default()),
            Err(Mismatch(505))
        );
        signed.numeric.insert(505, 300);
        assert_eq!(
            validate(&expected, &signed, &Policy::default()),
            Err(Mismatch(505))
        );
    }
    #[test]
    fn per_operation_auth_does_not_gain_an_unrequested_timeout() {
        let mut expected = authenticated();
        expected.numeric.remove(&505);
        let mut signed = expected.clone();
        signed.numeric.insert(505, 0);
        assert_eq!(
            validate(&expected, &signed, &Policy::default()),
            Err(Mismatch(505))
        );
        signed.numeric.insert(505, 30);
        assert_eq!(
            validate(&expected, &signed, &Policy::default()),
            Err(Mismatch(505))
        );
    }
    #[test]
    fn constraint_can_be_software_or_hardware_but_not_duplicated() {
        let mut expected = no_auth();
        expected.numeric.insert(509, 1);
        let mut hw = expected.clone();
        let mut sw = Policy::default();
        sw.numeric.insert(509, hw.numeric.remove(&509).unwrap());
        assert_eq!(validate(&expected, &hw, &sw), Ok(()));
        hw.numeric.insert(509, 1);
        assert_eq!(validate(&expected, &hw, &sw), Err(Mismatch(509)));
    }
    #[test]
    fn requested_device_id_must_match_hardware_bytes() {
        let mut expected = no_auth();
        expected.ids.insert(717, b"B-model".to_vec());
        let mut hw = expected.clone();
        hw.ids.insert(717, b"A-model".to_vec());
        assert_eq!(
            validate(&expected, &hw, &Policy::default()),
            Err(Mismatch(717))
        );
        let mut sw = Policy::default();
        sw.ids.insert(717, b"B-model".to_vec());
        hw.ids.remove(&717);
        assert_eq!(validate(&expected, &hw, &sw), Err(Mismatch(717)));
    }
    #[test]
    fn sid_and_unchecked_vendor_crypto_fields_are_not_invented_or_rejected() {
        let expected = no_auth();
        let mut hw = expected.clone();
        hw.numeric.extend([(502, 42), (203, 4)]);
        assert_eq!(validate(&expected, &hw, &Policy::default()), Ok(()));
    }
}
