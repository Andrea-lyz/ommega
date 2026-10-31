// SPDX-License-Identifier: Apache-2.0
//! The original authentication proof must succeed before local re-MACing.
//! Generic over the token/error only so this ordering can run without a HAL.
pub(super) fn verified_localization<T, U, E>(
    token: &T,
    validate: impl FnOnce(&T) -> Result<(), E>,
    verify: impl FnOnce(&T) -> Result<(), E>,
    localize: impl FnOnce(&T) -> Result<U, E>,
) -> Result<U, E> {
    validate(token)?;
    verify(token)?;
    localize(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn invalid_shape_never_calls_verification_or_remac() {
        let result: Result<(), &str> = verified_localization(
            &[0u8; 31],
            |_| Err("shape"),
            |_| panic!("invalid shape reached verifier"),
            |_| panic!("invalid shape reached localizer"),
        );
        assert_eq!(result, Err("shape"));
    }

    #[test]
    fn invalid_original_mac_never_calls_localizer() {
        let checked = Cell::new(false);
        let result: Result<(), &str> = verified_localization(
            &[0u8; 32],
            |_| {
                checked.set(true);
                Ok(())
            },
            |_| {
                assert!(checked.get());
                Err("original MAC")
            },
            |_| panic!("forged token was re-MACed"),
        );
        assert_eq!(result, Err("original MAC"));
    }

    #[test]
    fn all_steps_run_once_and_keep_the_original_input() {
        let step = Cell::new(0);
        let token = [7u8; 32];
        let result: Result<[u8; 32], &str> = verified_localization(
            &token,
            |input| {
                assert_eq!(step.get(), 0);
                assert_eq!(input, &token);
                step.set(1);
                Ok(())
            },
            |input| {
                assert_eq!(step.get(), 1);
                assert_eq!(input, &token);
                step.set(2);
                Ok(())
            },
            |input| {
                assert_eq!(step.get(), 2);
                step.set(3);
                Ok(*input)
            },
        );
        assert_eq!(result, Ok(token));
        assert_eq!(step.get(), 3);
    }

    #[test]
    fn localizer_error_is_not_converted_to_a_successful_token() {
        let result: Result<(), &str> = verified_localization(
            &(),
            |_| Ok(()),
            |_| Ok(()),
            |_| Err("local MAC unavailable"),
        );
        assert_eq!(result, Err("local MAC unavailable"));
    }
}
