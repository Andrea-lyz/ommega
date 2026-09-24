//! Software TA state: key ledger, anti-rollback counters and sign sessions.

use std::collections::BTreeMap;
use std::path::Path;

use rand_core::{OsRng, RngCore};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::{BigUint, RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};

use crate::blob;
use crate::error::{SOTER_ERR_BAD_VALUE, SOTER_ERR_NO_KEY, SOTER_ERR_TA_UNAVAILABLE, SOTER_OK};

/// Key size the stock TA uses for ASK and AuthKey.
const RSA_BITS: usize = 2048;
const RSA_EXPONENT: u32 = 65537;

/// A key pair in PEM form, so the whole ledger fits in one JSON file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyPair {
    pub private_pem: String,
    pub public_pem: String,
}

impl KeyPair {
    fn generate() -> Option<Self> {
        let mut rng = OsRng;
        let private =
            RsaPrivateKey::new_with_exp(&mut rng, RSA_BITS, &BigUint::from(RSA_EXPONENT)).ok()?;
        let public = RsaPublicKey::from(&private);
        Some(Self {
            private_pem: private.to_pkcs8_pem(LineEnding::LF).ok()?.to_string(),
            public_pem: public.to_public_key_pem(LineEnding::LF).ok()?,
        })
    }

    pub fn private(&self) -> Option<RsaPrivateKey> {
        RsaPrivateKey::from_pkcs8_pem(&self.private_pem).ok()
    }

    pub fn public(&self) -> Option<RsaPublicKey> {
        RsaPublicKey::from_public_key_pem(&self.public_pem).ok()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UidState {
    /// Anti-rollback counter, strictly increasing per uid on the wire.
    pub counter: u64,
    #[serde(default)]
    pub ask: Option<KeyPair>,
    #[serde(default)]
    pub auth: BTreeMap<String, KeyPair>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub uid: u32,
    pub kname: String,
    pub challenge: String,
}

/// Everything the software TA knows. Serialized as a single JSON document so the
/// injector can keep it under `/data/adb/ommega/soterta/state.json` (mode 0600).
#[derive(Debug, Serialize, Deserialize)]
pub struct TaState {
    /// Synthetic device id: 32 hex characters, generated locally and never
    /// copied from another device.
    pub cpu_id: String,
    /// Device key that signs ASK blobs. A real device signs those with the
    /// ATTK inside the secure world, which cannot be reproduced here.
    pub attk: KeyPair,
    #[serde(default)]
    pub uids: BTreeMap<u32, UidState>,
    #[serde(default)]
    pub sessions: BTreeMap<u64, Session>,
    #[serde(default)]
    pub next_session: u64,
    /// Fingerprint template id reported in sign results, one per uid and stable
    /// across signatures (a finger id does not change between payments).
    #[serde(default)]
    pub fingerprints: BTreeMap<u32, String>,
}

impl TaState {
    /// Fresh local state: new device id and a new device signing key.
    pub fn generate_local() -> Option<Self> {
        let mut rng = OsRng;
        let mut id = [0u8; 12];
        rng.fill_bytes(&mut id);
        Some(Self {
            cpu_id: format!("00000000{}", hex::encode(id)),
            attk: KeyPair::generate()?,
            uids: BTreeMap::new(),
            sessions: BTreeMap::new(),
            next_session: 0,
            fingerprints: BTreeMap::new(),
        })
    }

    pub fn from_json(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).unwrap_or_default()
    }

    pub fn load(path: &Path) -> Option<Self> {
        std::fs::read(path).ok().and_then(|bytes| Self::from_json(&bytes))
    }

    pub fn store(&self, path: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        std::fs::write(path, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    fn next_counter(&mut self, uid: u32) -> u64 {
        let entry = self.uids.entry(uid).or_default();
        entry.counter = entry.counter.wrapping_add(1);
        entry.counter
    }

    fn fingerprint_id(&mut self, uid: u32) -> String {
        if let Some(id) = self.fingerprints.get(&uid) {
            return id.clone();
        }
        let value = OsRng.next_u32() % 100_000_000;
        let id = format!("7{value:08}");
        self.fingerprints.insert(uid, id.clone());
        id
    }

    fn ask_of(&self, uid: u32) -> Option<KeyPair> {
        self.uids.get(&uid).and_then(|entry| entry.ask.clone())
    }

    fn auth_of(&self, uid: u32, kname: &str) -> Option<KeyPair> {
        self.uids
            .get(&uid)
            .and_then(|entry| entry.auth.get(kname))
            .cloned()
    }

    /// `generateAskKeyPair`: create the uid's ASK, keep an existing one.
    pub fn generate_ask(&mut self, uid: u32) -> i32 {
        let entry = self.uids.entry(uid).or_default();
        if entry.ask.is_none() {
            let Some(pair) = KeyPair::generate() else {
                return SOTER_ERR_TA_UNAVAILABLE;
            };
            entry.ask = Some(pair);
        }
        SOTER_OK
    }

    /// `hasAskAlready`: `SOTER_OK` means "yes", anything else means "no".
    pub fn has_ask(&self, uid: u32) -> i32 {
        if self.ask_of(uid).is_some() {
            SOTER_OK
        } else {
            SOTER_ERR_NO_KEY
        }
    }

    /// `exportAskPublicKey`: ASK blob signed with the device key.
    pub fn export_ask(&mut self, uid: u32) -> (i32, Vec<u8>) {
        let Some(pair) = self.ask_of(uid) else {
            return (SOTER_ERR_NO_KEY, Vec::new());
        };
        let Some(device_key) = self.attk.private() else {
            return (SOTER_ERR_TA_UNAVAILABLE, Vec::new());
        };
        let counter = self.next_counter(uid);
        let json = blob::key_json(&pair.public_pem, &self.cpu_id, counter, uid);
        let Some(signature) = blob::sign(&device_key, &json) else {
            return (SOTER_ERR_TA_UNAVAILABLE, Vec::new());
        };
        (SOTER_OK, blob::encode(&json, &signature))
    }

    /// `generateAuthKeyPair`: needs the uid's ASK to exist, since the ASK is
    /// what signs the AuthKey blob.
    pub fn generate_auth(&mut self, uid: u32, kname: &str) -> i32 {
        if kname.is_empty() {
            return SOTER_ERR_BAD_VALUE;
        }
        if self.ask_of(uid).is_none() {
            return SOTER_ERR_NO_KEY;
        }
        let entry = self.uids.entry(uid).or_default();
        if !entry.auth.contains_key(kname) {
            let Some(pair) = KeyPair::generate() else {
                return SOTER_ERR_TA_UNAVAILABLE;
            };
            entry.auth.insert(kname.to_string(), pair);
        }
        SOTER_OK
    }

    pub fn has_auth(&self, uid: u32, kname: &str) -> i32 {
        if kname.is_empty() || self.auth_of(uid, kname).is_none() {
            SOTER_ERR_NO_KEY
        } else {
            SOTER_OK
        }
    }

    /// `exportAuthKeyPublicKey`: AuthKey blob signed with the uid's ASK.
    pub fn export_auth(&mut self, uid: u32, kname: &str) -> (i32, Vec<u8>) {
        let Some(ask) = self.ask_of(uid) else {
            return (SOTER_ERR_NO_KEY, Vec::new());
        };
        let Some(auth) = self.auth_of(uid, kname) else {
            return (SOTER_ERR_NO_KEY, Vec::new());
        };
        let Some(ask_key) = ask.private() else {
            return (SOTER_ERR_TA_UNAVAILABLE, Vec::new());
        };
        let counter = self.next_counter(uid);
        let json = blob::key_json(&auth.public_pem, &self.cpu_id, counter, uid);
        let Some(signature) = blob::sign(&ask_key, &json) else {
            return (SOTER_ERR_TA_UNAVAILABLE, Vec::new());
        };
        (SOTER_OK, blob::encode(&json, &signature))
    }

    pub fn remove_auth(&mut self, uid: u32, kname: &str) -> i32 {
        let Some(entry) = self.uids.get_mut(&uid) else {
            return SOTER_ERR_NO_KEY;
        };
        if entry.auth.remove(kname).is_some() {
            SOTER_OK
        } else {
            SOTER_ERR_NO_KEY
        }
    }

    /// `removeAllUidKey`: drop the ASK, every AuthKey and any open session.
    pub fn remove_all_uid(&mut self, uid: u32) -> i32 {
        let existed = self.uids.remove(&uid).is_some();
        self.sessions.retain(|_, session| session.uid != uid);
        if existed {
            SOTER_OK
        } else {
            SOTER_ERR_NO_KEY
        }
    }

    /// `initSign`: open a session for an existing AuthKey.
    ///
    /// The stock TA refuses to sign without a fresh fingerprint match; this one
    /// does not, which is the entire point of the local check path.
    pub fn init_sign(&mut self, uid: u32, kname: &str, challenge: &str) -> (i32, u64) {
        if kname.is_empty() {
            return (SOTER_ERR_BAD_VALUE, 0);
        }
        if self.auth_of(uid, kname).is_none() {
            return (SOTER_ERR_NO_KEY, 0);
        }
        self.next_session = self.next_session.wrapping_add(1).max(1);
        let session = self.next_session;
        self.sessions.insert(
            session,
            Session {
                uid,
                kname: kname.to_string(),
                challenge: challenge.to_string(),
            },
        );
        (SOTER_OK, session)
    }

    /// `finishSign`: sign the challenge once with the AuthKey and close the
    /// session, whether or not signing worked.
    pub fn finish_sign(&mut self, session: u64) -> (i32, Vec<u8>) {
        let Some(entry) = self.sessions.remove(&session) else {
            return (SOTER_ERR_BAD_VALUE, Vec::new());
        };
        let Some(key) = self.auth_of(entry.uid, &entry.kname) else {
            return (SOTER_ERR_NO_KEY, Vec::new());
        };
        let Some(private) = key.private() else {
            return (SOTER_ERR_TA_UNAVAILABLE, Vec::new());
        };
        let counter = self.next_counter(entry.uid);
        let fid = self.fingerprint_id(entry.uid);
        let json = blob::result_json(&entry.challenge, &fid, counter, &self.cpu_id, entry.uid);
        let Some(signature) = blob::sign(&private, &json) else {
            return (SOTER_ERR_TA_UNAVAILABLE, Vec::new());
        };
        (SOTER_OK, blob::encode(&json, &signature))
    }

    /// `getDeviceId`: ASCII bytes of the same 32-hex-character string that is
    /// embedded as `cpu_id` in every blob. The stock HAL reports the id as a
    /// byte buffer and the Java layer passes it through, so keeping both
    /// spellings identical keeps a checker from seeing two ids on one device.
    pub fn device_id(&self) -> (i32, Vec<u8>) {
        (SOTER_OK, self.cpu_id.as_bytes().to_vec())
    }
}

