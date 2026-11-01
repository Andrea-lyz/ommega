//! Validated, fail-closed identity policy. Resolve once for each transaction.
use crate::{blob, remote, state::KeyPair};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const STORE: &str = "/data/misc/keystore/ommega/data/soter/identities.json";
pub const LEGACY_STORE: &str = "/data/adb/ommega/soterta/identities.json";
pub const PACKAGES: &str = "/data/system/packages.list";
pub const MAX_POLICY: usize = 512 * 1024;
pub const MAX_KEY: usize = 32 * 1024;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cpu_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pem: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub device_id: String,
    #[serde(default)]
    pub tls_insecure: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub uid_map: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Store {
    pub version: u32,
    #[serde(default)]
    pub default: String,
    pub identities: Vec<Record>,
    #[serde(default)]
    pub apps: BTreeMap<String, String>,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            version: 1,
            default: String::new(),
            identities: Vec::new(),
            apps: BTreeMap::new(),
        }
    }
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
pub fn valid_package(pkg: &str) -> bool {
    pkg.len() <= 255
        && (pkg == "android" || pkg.contains('.'))
        && pkg.split('.').all(|part| {
            !part.is_empty()
                && part.as_bytes()[0].is_ascii_alphabetic()
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}
pub fn valid_cpu_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

impl Store {
    pub fn parse(raw: &[u8]) -> Result<Self, String> {
        if raw.len() > MAX_POLICY {
            return Err("SOTER identity policy too large".into());
        }
        let store: Self =
            serde_json::from_slice(raw).map_err(|_| "Invalid SOTER identity policy")?;
        store.validate()?;
        Ok(store)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.identities.len() > 256 || self.apps.len() > 4096 {
            return Err("Unsupported or oversized SOTER policy".into());
        }
        let mut ids = BTreeSet::new();
        for r in &self.identities {
            if !valid_id(&r.id)
                || !ids.insert(r.id.as_str())
                || r.name.trim().is_empty()
                || r.name.chars().count() > 80
                || !matches!(r.kind.as_str(), "local" | "key" | "relay")
            {
                return Err("Invalid or duplicate SOTER identity".into());
            }
            if r.url.len() > 4096
                || r.token.len() > 8192
                || r.device_id.len() > 255
                || r.uid_map.len() > 8192
                || r.fingerprint.len() > 128
                || r.pem.len() > 4096
            {
                return Err("SOTER identity field too large".into());
            }
            if !r.cpu_id.is_empty() && !valid_cpu_id(&r.cpu_id) {
                return Err("Invalid SOTER device ID".into());
            }
            if !r.pem.is_empty() {
                let path = Path::new(&r.pem);
                let canonical = Path::new(STORE).parent().unwrap().join("keys");
                let legacy = Path::new(LEGACY_STORE).parent().unwrap().join("identities");
                if path.parent() != Some(canonical.as_path())
                    && path.parent() != Some(legacy.as_path())
                {
                    return Err("SOTER key must be in the managed key directory".into());
                }
                if path.file_name().and_then(|n| n.to_str()).is_none_or(|n| {
                    !n.ends_with(".pem")
                        || !n
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                }) {
                    return Err("Invalid SOTER key path".into());
                }
            }
        }
        if !self.default.is_empty() && !ids.contains(self.default.as_str()) {
            return Err("SOTER default identity does not exist".into());
        }
        for (pkg, id) in &self.apps {
            if !valid_package(pkg) || !ids.contains(id.as_str()) {
                return Err("Invalid SOTER application identity reference".into());
            }
        }
        Ok(())
    }
    pub fn record(&self, id: &str) -> Result<&Record, String> {
        self.identities
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| "SOTER identity does not exist".into())
    }
    /// Shared-UID packages cannot be independently identified at the HAL boundary.
    pub fn selected_id(&self, uid: Option<u32>, packages: &str) -> Result<String, String> {
        let Some(uid) = uid else {
            return Ok(self.default.clone());
        };
        if self.apps.is_empty() {
            return Ok(self.default.clone());
        }
        let app_id = uid % 100_000;
        let mut choices = BTreeSet::new();
        for line in packages.lines() {
            let mut fields = line.split_whitespace();
            let (Some(pkg), Some(number)) = (fields.next(), fields.next()) else {
                continue;
            };
            if number
                .parse::<u32>()
                .ok()
                .is_some_and(|n| n % 100_000 == app_id)
            {
                choices.insert(self.apps.get(pkg).unwrap_or(&self.default).clone());
            }
        }
        match choices.len() {
            0 if app_id < 10_000 => Ok(self.default.clone()),
            0 => Err("Cannot resolve the SOTER caller package".into()),
            1 => Ok(choices.into_iter().next().unwrap()),
            _ => Err("Conflicting SOTER identity policies for a shared UID".into()),
        }
    }
}

pub enum Selection {
    Global,
    Local {
        id: String,
    },
    Key {
        id: String,
        cpu_id: String,
        pair: KeyPair,
    },
    Relay {
        id: String,
        config: remote::Config,
    },
}
impl Selection {
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Global => None,
            Self::Local { id } | Self::Key { id, .. } | Self::Relay { id, .. } => Some(id),
        }
    }
    pub fn revision(&self) -> String {
        match self {
            Self::Global => String::new(),
            Self::Local { id } => digest(format!("local:{id}").as_bytes()),
            Self::Key { id, cpu_id, pair } => {
                digest(format!("key:{id}:{cpu_id}:{}", pair.public_pem).as_bytes())
            }
            Self::Relay { id, config } => {
                digest(format!("relay:{id}:{}", config.route_scope()).as_bytes())
            }
        }
    }
}

/// A policy snapshot owns the exact bytes and resolves all transaction fields once.
pub struct Snapshot {
    pub store: Store,
    pub present: bool,
    source: Option<PathBuf>,
    raw: Vec<u8>,
    observed_paths: Option<(PathBuf, PathBuf)>,
}
impl Snapshot {
    pub fn memory(store: Store) -> Self {
        Self {
            store,
            present: true,
            source: None,
            raw: Vec::new(),
            observed_paths: None,
        }
    }
    pub fn absent() -> Self {
        Self {
            store: Store::default(),
            present: false,
            source: None,
            raw: Vec::new(),
            observed_paths: None,
        }
    }
    pub fn load() -> Result<Self, String> {
        Self::load_at(Path::new(STORE), Path::new(LEGACY_STORE))
    }
    pub fn load_at(primary: &Path, legacy: &Path) -> Result<Self, String> {
        for path in [primary, legacy] {
            match fs::read(path) {
                Ok(raw) => {
                    let store = Store::parse(&raw)?;
                    return Ok(Self {
                        store,
                        present: true,
                        source: Some(path.to_owned()),
                        raw,
                        observed_paths: Some((primary.to_owned(), legacy.to_owned())),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(_) => return Err("Cannot read SOTER identity policy".into()),
            }
        }
        let mut snapshot = Self::absent();
        snapshot.observed_paths = Some((primary.to_owned(), legacy.to_owned()));
        Ok(snapshot)
    }
    pub(crate) fn is_canonical(&self) -> bool {
        self.source.as_deref() == Some(Path::new(STORE))
    }

    pub fn unchanged(&self) -> Result<(), String> {
        if self.source.is_none()
            && self
                .observed_paths
                .as_ref()
                .is_some_and(|(a, b)| a.exists() || b.exists())
        {
            return Err("SOTER identity policy appeared during the operation; retry".into());
        }
        if let Some(path) = &self.source {
            if fs::read(path).map_err(|_| "SOTER policy disappeared during the operation")?
                != self.raw
            {
                return Err("SOTER policy changed during the operation; retry".into());
            }
            if self
                .observed_paths
                .as_ref()
                .is_some_and(|(primary, _)| path != primary && primary.exists())
            {
                return Err("SOTER policy was migrated during the operation; retry".into());
            }
        }
        Ok(())
    }
    pub fn resolve(&self, uid: Option<u32>, packages: &str) -> Result<Selection, String> {
        self.store.validate()?;
        let id = self.store.selected_id(uid, packages)?;
        if id.is_empty() {
            return Ok(Selection::Global);
        }
        let r = self.store.record(&id)?;
        match r.kind.as_str() {
            "local" => Ok(Selection::Local { id }),
            "key" => {
                if !valid_cpu_id(&r.cpu_id) || r.pem.is_empty() {
                    return Err("The selected SOTER key identity is not configured".into());
                }
                let raw =
                    fs::read(&r.pem).map_err(|_| "Cannot read the selected SOTER private key")?;
                let text = std::str::from_utf8(&raw).map_err(|_| "Invalid SOTER PEM encoding")?;
                let pair = validate_key(text)?;
                Ok(Selection::Key {
                    id,
                    cpu_id: r.cpu_id.to_ascii_lowercase(),
                    pair,
                })
            }
            "relay" => Ok(Selection::Relay {
                id,
                config: relay_config(r)?,
            }),
            _ => Err("Unknown SOTER identity type".into()),
        }
    }
}

pub fn relay_config(r: &Record) -> Result<remote::Config, String> {
    let config = remote::Config {
        enabled: true,
        url: r.url.clone(),
        token: r.token.clone(),
        device_id: r.device_id.clone(),
        tls_insecure: r.tls_insecure,
        uid_map: r.uid_map.clone(),
    };
    config.validate()?;
    Ok(config)
}
/// Both PEM formats are accepted, but all private parameters and RSA size are checked.
pub fn validate_key(pem: &str) -> Result<KeyPair, String> {
    if pem.len() > MAX_KEY {
        return Err("SOTER private key is too large".into());
    }
    let key = RsaPrivateKey::from_pkcs8_pem(pem)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(pem))
        .map_err(|_| "Invalid RSA private key")?;
    key.validate()
        .map_err(|_| "Invalid RSA private parameters")?;
    if key.n().bits() != 2048 || key.e() != &rsa::BigUint::from(65537u32) {
        return Err("SOTER requires RSA-2048 with exponent 65537".into());
    }
    let public = RsaPublicKey::from(&key);
    let probe = b"ommega-soter-key-validation";
    let signature = blob::sign(&key, probe).ok_or("SOTER private key cannot sign")?;
    if !blob::verify(&public, probe, &signature) {
        return Err("SOTER private key self-test failed".into());
    }
    Ok(KeyPair {
        private_pem: key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|_| "Cannot encode SOTER key")?
            .to_string(),
        public_pem: public
            .to_public_key_pem(LineEnding::LF)
            .map_err(|_| "Cannot encode SOTER public key")?,
    })
}

/// Cooperating manager writes cannot race live signing. No persistent lock file.
pub fn lock_policy(exclusive: bool) -> Result<Option<fs::File>, String> {
    let directory = Path::new(LEGACY_STORE).parent().unwrap();
    let file = match fs::File::open(directory) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot lock SOTER policy".into()),
    };
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let mode = if exclusive {
            libc::LOCK_EX
        } else {
            libc::LOCK_SH
        };
        if unsafe { libc::flock(file.as_raw_fd(), mode | libc::LOCK_NB) } != 0 {
            // A manager write must not interleave with signing, so it fails and
            // asks for a retry. A read that merely collides keeps going without
            // the guard: policy renames are atomic and a key that disappeared
            // fails closed, which is better than refusing a live SOTER call.
            if exclusive {
                return Err(
                    "A SOTER request or policy write is active; retry after it finishes".into(),
                );
            }
            return Ok(None);
        }
    }
    #[cfg(not(unix))]
    let _ = exclusive;
    Ok(Some(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn local(id: &str) -> Record {
        Record {
            id: id.into(),
            name: id.into(),
            kind: "local".into(),
            ..Record::default()
        }
    }
    #[test]
    fn defaults_shared_uid_and_profiles_are_explicit() {
        let mut store = Store {
            default: "a".into(),
            identities: vec![local("a"), local("b")],
            ..Store::default()
        };
        assert_eq!(store.selected_id(Some(11000), "").unwrap(), "a");
        store.apps.insert("org.example.one".into(), "b".into());
        assert_eq!(
            store
                .selected_id(Some(111000), "org.example.one 11000 0 x\n")
                .unwrap(),
            "b"
        );
        assert!(store
            .selected_id(
                Some(11000),
                "org.example.one 11000 0 x\norg.example.two 11000 0 x"
            )
            .is_err());
        assert_eq!(store.selected_id(None, "").unwrap(), "a");
        assert!(store.selected_id(Some(12000), "").is_err());
    }
    #[test]
    fn corrupt_policy_and_invalid_references_fail_closed() {
        assert!(Store::parse(b"{").is_err());
        let mut store = Store {
            default: "missing".into(),
            ..Store::default()
        };
        assert!(store.validate().is_err());
        store.default.clear();
        store
            .apps
            .insert("org.example.one".into(), "missing".into());
        assert!(store.validate().is_err());
    }
    #[test]
    fn incomplete_identity_never_becomes_global() {
        let r = Record {
            id: "a".into(),
            name: "draft".into(),
            kind: "key".into(),
            cpu_id: "a".repeat(32),
            ..Record::default()
        };
        let snapshot = Snapshot::memory(Store {
            default: "a".into(),
            identities: vec![r],
            ..Store::default()
        });
        assert!(snapshot.resolve(Some(11000), "").is_err());
        let r = Record {
            id: "a".into(),
            name: "draft".into(),
            kind: "relay".into(),
            url: "https://example.invalid".into(),
            ..Record::default()
        };
        assert!(relay_config(&r).is_err());
    }
}
