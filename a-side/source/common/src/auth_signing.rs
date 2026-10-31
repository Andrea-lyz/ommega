//! Creation-time policy for A-authorized remote signing. No routing or app-name heuristics.

use kmr_wire::keymint::{Algorithm, KeyParam, KeyPurpose};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Auto,
    Compatibility,
    Preserve,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub default: Mode,
    #[serde(default)]
    pub apps: BTreeMap<String, Mode>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            default: Mode::Auto,
            apps: BTreeMap::new(),
        }
    }
}

pub fn valid_package(package: &str) -> bool {
    package.len() <= 255
        && package.split('.').count() >= 2
        && package.split('.').all(|part| {
            !part.is_empty()
                && part.as_bytes()[0].is_ascii_alphabetic()
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        })
}

impl Config {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1 {
            return Err("unsupported auth-signing config version");
        }
        if self.apps.len() > 4096 || self.apps.keys().any(|p| !valid_package(p)) {
            return Err("invalid auth-signing app exceptions");
        }
        Ok(())
    }

    /// A shared UID has one AAID. Explicit preserve wins a conflict, followed
    /// by auto, then compatibility; inherited packages do not cancel overrides.
    pub fn mode_for_packages<'a>(&self, packages: impl IntoIterator<Item = &'a str>) -> Mode {
        packages
            .into_iter()
            .filter_map(|p| self.apps.get(p))
            .copied()
            .max_by_key(|m| match m {
                Mode::Preserve => 2,
                Mode::Auto => 1,
                Mode::Compatibility => 0,
            })
            .unwrap_or(self.default)
    }
}

/// This policy only supports HAT-bound asymmetric signing keys. Other purposes
/// and trusted-presence/confirmation requirements retain their existing path.
pub fn eligible(params: &[KeyParam]) -> bool {
    params.iter().any(|p| matches!(p, KeyParam::Purpose(KeyPurpose::Sign)))
        && params.iter().any(|p| matches!(p, KeyParam::Algorithm(Algorithm::Ec | Algorithm::Rsa)))
        && params.iter().any(|p| matches!(p, KeyParam::UserSecureId(_)))
        && params.iter().any(|p| matches!(p, KeyParam::UserAuthType(value) if *value != 0))
        && !params.iter().any(|p| matches!(p, KeyParam::Purpose(purpose) if !matches!(purpose, KeyPurpose::Sign | KeyPurpose::Verify)))
        && !params.iter().any(|p| matches!(p, KeyParam::NoAuthRequired | KeyParam::TrustedUserPresenceRequired | KeyParam::TrustedConfirmationRequired | KeyParam::AllowWhileOnBody))
}

/// Aliases are application-owned names, not evidence for an authentication policy.
/// Auto preserves the requested hardware authorization. Relaxing it requires an
/// explicit compatibility setting for the caller.
pub fn authorize_on_a(mode: Mode, _alias: Option<&str>, params: &[KeyParam]) -> bool {
    eligible(params) && matches!(mode, Mode::Compatibility)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kmr_wire::keymint::{Digest, EcCurve};
    const ALIAS: &str = "TrustAttestor_silent_auth_14c21f61-4c15-4e09-92d7-3a33d665458c";

    fn probe() -> Vec<KeyParam> {
        vec![
            KeyParam::Purpose(KeyPurpose::Sign),
            KeyParam::Purpose(KeyPurpose::Verify),
            KeyParam::Algorithm(Algorithm::Ec),
            KeyParam::EcCurve(EcCurve::P256),
            KeyParam::Digest(Digest::Sha256),
            KeyParam::UserSecureId(42),
            KeyParam::UserAuthType(3),
            KeyParam::AttestationChallenge(vec![7; 32]),
        ]
    }

    #[test]
    fn auto_and_preserve_are_alias_independent() {
        let mut timed = probe();
        timed.push(KeyParam::AuthTimeout(30));
        let mut unlocked = probe();
        unlocked.push(KeyParam::UnlockedDeviceRequired);
        for params in [probe(), timed, unlocked] {
            for alias in [
                None,
                Some("payment"),
                Some(ALIAS),
                Some("TrustAttestor_silent_auth_bad"),
                Some("arbitrary-auth-key"),
            ] {
                assert!(!authorize_on_a(Mode::Auto, alias, &params));
                assert!(!authorize_on_a(Mode::Preserve, alias, &params));
                assert!(authorize_on_a(Mode::Compatibility, alias, &params));
            }
        }
    }

    #[test]
    fn unsupported_requirements_never_relax_b_authentication() {
        for extra in [
            KeyParam::Purpose(KeyPurpose::Decrypt),
            KeyParam::Purpose(KeyPurpose::AgreeKey),
            KeyParam::TrustedConfirmationRequired,
            KeyParam::TrustedUserPresenceRequired,
            KeyParam::AllowWhileOnBody,
            KeyParam::NoAuthRequired,
        ] {
            let mut params = probe();
            params.push(extra);
            assert!(!authorize_on_a(Mode::Compatibility, None, &params));
        }
        let mut params = probe();
        params.retain(|p| !matches!(p, KeyParam::UserSecureId(_)));
        assert!(!eligible(&params));
    }

    #[test]
    fn exceptions_and_shared_uid_resolution_are_deterministic() {
        let mut config = Config::default();
        config
            .apps
            .insert("com.example.pay".into(), Mode::Compatibility);
        config
            .apps
            .insert("com.example.probe".into(), Mode::Preserve);
        assert_eq!(
            config.mode_for_packages(["com.example.pay", "com.example.other"]),
            Mode::Compatibility
        );
        for packages in [
            ["com.example.pay", "com.example.probe"],
            ["com.example.probe", "com.example.pay"],
        ] {
            assert_eq!(config.mode_for_packages(packages), Mode::Preserve);
        }
        config.default = Mode::Preserve;
        assert_eq!(
            config.mode_for_packages(["com.unknown.app"]),
            Mode::Preserve
        );
        assert_eq!(
            config.mode_for_packages(["com.example.pay"]),
            Mode::Compatibility
        );
        assert!(config.validate().is_ok());
        config.apps.insert("com.bad;command".into(), Mode::Auto);
        assert!(config.validate().is_err());
    }

    #[test]
    fn decisions_do_not_leak_between_concurrent_requests() {
        let threads: Vec<_> = (0..16)
            .map(|i| {
                std::thread::spawn(move || {
                    let mode = if i % 2 == 0 {
                        Mode::Preserve
                    } else {
                        Mode::Compatibility
                    };
                    for _ in 0..100 {
                        assert_eq!(authorize_on_a(mode, None, &probe()), i % 2 != 0);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }
}
