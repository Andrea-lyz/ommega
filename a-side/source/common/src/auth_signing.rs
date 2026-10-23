//! Creation-time policy for A-authorized remote signing. No routing or app-name heuristics.

use kmr_wire::keymint::{Algorithm, Digest, EcCurve, KeyParam, KeyPurpose};
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

/// Match the known probe contract, not a whole package or a generic auth key.
/// The auth probe uses a random 32-byte challenge, so challenge length alone
/// must never select preservation for an ordinary payment key.
pub fn known_probe(alias: Option<&str>, params: &[KeyParam]) -> bool {
    let Some(uuid) = alias.and_then(|a| a.strip_prefix("TrustAttestor_silent_auth_")) else {
        return false;
    };
    let bytes = uuid.as_bytes();
    let uuid_v4 = bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                *c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
        && bytes[14] == b'4'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b' | b'A' | b'B');
    uuid_v4
        && eligible(params)
        && params.contains(&KeyParam::Algorithm(Algorithm::Ec))
        && params.contains(&KeyParam::EcCurve(EcCurve::P256))
        && params.contains(&KeyParam::Purpose(KeyPurpose::Verify))
        && params.contains(&KeyParam::Digest(Digest::Sha256))
        && params.contains(&KeyParam::UserAuthType(3))
        && params
            .iter()
            .any(|p| matches!(p, KeyParam::AttestationChallenge(v) if v.len() == 32))
        && !params
            .iter()
            .any(|p| matches!(p, KeyParam::AuthTimeout(v) if *v != 0))
        && !params
            .iter()
            .any(|p| matches!(p, KeyParam::Digest(d) if *d != Digest::Sha256))
        && !params.contains(&KeyParam::UnlockedDeviceRequired)
}

pub fn authorize_on_a(mode: Mode, alias: Option<&str>, params: &[KeyParam]) -> bool {
    eligible(params)
        && match mode {
            Mode::Auto => !known_probe(alias, params),
            Mode::Compatibility => true,
            Mode::Preserve => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn only_the_known_probe_is_preserved_in_auto() {
        let params = probe();
        assert!(known_probe(Some(ALIAS), &params));
        assert!(!authorize_on_a(Mode::Auto, Some(ALIAS), &params));
        for alias in [
            None,
            Some("payment"),
            Some("TrustAttestor_silent_auth_bad"),
            Some("TrustAttestor_silent_other_14c21f61-4c15-4e09-92d7-3a33d665458c"),
        ] {
            assert!(authorize_on_a(Mode::Auto, alias, &params));
        }
        assert!(authorize_on_a(Mode::Compatibility, Some(ALIAS), &params));
        assert!(!authorize_on_a(Mode::Preserve, Some("payment"), &params));
        let mut timed = params.clone();
        timed.push(KeyParam::AuthTimeout(30));
        assert!(!known_probe(Some(ALIAS), &timed));
        let mut unlocked = params;
        unlocked.push(KeyParam::UnlockedDeviceRequired);
        assert!(!known_probe(Some(ALIAS), &unlocked));
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
