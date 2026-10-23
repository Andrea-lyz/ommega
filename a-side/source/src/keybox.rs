//! Active Keybox state, reload orchestration and KeyMint signing-info adapter.

mod material;
mod storage;

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock, RwLock,
    },
};

use anyhow::Result;
use kmr_common::Error;
use kmr_ta::device::{RetrieveCertSigningInfo, SigningInfoSnapshot, SigningKeyType};
use kmr_wire::keymint;
use log::{debug, error, info, warn};

pub use material::{CertSignAlgoInfo, KeyBox};
pub use storage::ensure_keybox_file;

use material::KeyAlgorithm;
use storage::{load_keybox_with_fallback, write_keybox_xml};

pub const KEYBOX_PATH: &str = "/data/misc/keystore/ommega/keybox.xml";

/// Optional dedicated attestation material for the StrongBox security level.
///
/// A real device signs StrongBox attestations with a keychain of its own, kept
/// separate from the TEE batch chain. Reusing one chain for both levels is
/// externally observable: an app can create a TEE key and a StrongBox key and
/// compare the issuers and serial numbers of the two chains. When this file is
/// absent, StrongBox attestation fails with `ATTESTATION_KEYS_NOT_PROVISIONED`
/// instead of silently labelling the batch chain as StrongBox.
pub const KEYBOX_STRONGBOX_PATH: &str = "/data/misc/keystore/ommega/keybox-strongbox.xml";

const BUNDLED_KEYBOX_XML: &str = include_str!("../template/keybox.xml");

lazy_static::lazy_static! {
    pub static ref KEYBOX: RwLock<KeyBox> = RwLock::new(KeyBox::new());
    static ref KEYBOX_IO_LOCK: Mutex<()> = Mutex::new(());
}

static KEYBOX_WATCHER: OnceLock<()> = OnceLock::new();
static KEYBOX_DB_RETIRE_ALLOWED: AtomicBool = AtomicBool::new(false);
static KEYBOX_RUNTIME_LOADED: AtomicBool = AtomicBool::new(false);

fn is_bundled_keybox_xml(xml: &str) -> bool {
    xml.trim() == BUNDLED_KEYBOX_XML.trim()
}

fn retire_stale_keybox_bound_entries(current_identity: [u8; 32]) {
    if !db_retirement_allowed() {
        warn!("skipping stale keybox-bound DB retirement while active keybox came from fallback");
        return;
    }

    match crate::global::DB.with(|db| {
        db.borrow_mut()
            .retire_stale_keybox_bound_entries(current_identity)
    }) {
        Ok(0) => debug!("no stale keybox-bound key entries needed retirement"),
        Ok(retired) => info!("retired {retired} stale keybox-bound key entries"),
        Err(error) => error!("failed to retire stale keybox-bound key entries: {error:#}"),
    }
}

fn install_keybox(
    new_keybox: KeyBox,
    retire_db_entries: bool,
    db_retirement_allowed: bool,
) -> bool {
    let new_identity = new_keybox.identity_digest();
    let changed = {
        let mut keybox = KEYBOX.write().unwrap();
        let changed = keybox.identity_digest() != new_identity;
        *keybox = new_keybox;
        changed
    };
    KEYBOX_DB_RETIRE_ALLOWED.store(db_retirement_allowed, Ordering::Release);
    KEYBOX_RUNTIME_LOADED.store(true, Ordering::Release);

    if changed {
        crate::keymaster::keymint_device::clear_initialized_attestation_caches();
    }

    if retire_db_entries {
        retire_stale_keybox_bound_entries(new_identity);
    }

    changed
}

pub fn db_retirement_allowed() -> bool {
    KEYBOX_DB_RETIRE_ALLOWED.load(Ordering::Acquire)
}

fn is_fallback_continuation(keybox: &KeyBox, contents: &str) -> bool {
    let current_identity = KEYBOX
        .read()
        .map(|current| current.identity_digest())
        .unwrap_or([0u8; 32]);
    is_fallback_continuation_with_state(
        keybox,
        contents,
        KEYBOX_RUNTIME_LOADED.load(Ordering::Acquire),
        db_retirement_allowed(),
        current_identity,
    )
}

fn is_fallback_continuation_with_state(
    keybox: &KeyBox,
    contents: &str,
    runtime_loaded: bool,
    retirement_allowed: bool,
    current_identity: [u8; 32],
) -> bool {
    runtime_loaded
        && !retirement_allowed
        && is_bundled_keybox_xml(contents)
        && current_identity == keybox.identity_digest()
}

pub fn reload_from_disk() -> Result<bool> {
    reload_from_disk_inner(true)
}

fn reload_from_disk_inner(retire_db_entries: bool) -> Result<bool> {
    let _io_guard = KEYBOX_IO_LOCK.lock().unwrap();
    let (keybox, used_fallback) = load_keybox_with_fallback(KEYBOX_PATH)?;
    let changed = install_keybox(keybox, retire_db_entries, !used_fallback);
    if changed {
        info!(
            "active keybox identity updated from {} (fallback={})",
            KEYBOX_PATH, used_fallback
        );
    } else {
        debug!(
            "keybox reload completed without identity change (fallback={})",
            used_fallback
        );
    }
    Ok(changed)
}

pub fn initialize() -> Result<()> {
    ensure_keybox_file(KEYBOX_PATH)?;
    reload_from_disk_inner(false)?;
    KEYBOX_WATCHER.get_or_init(|| {
        if let Err(error) = kmr_common::runtime::file_watch::spawn_path_watcher(
            "ommega-keybox-watch",
            PathBuf::from(KEYBOX_PATH),
            |_trigger| {
                if let Err(reload_error) = reload_from_disk() {
                    error!("failed to reload keybox.xml after change: {reload_error:#}");
                }
            },
        ) {
            error!("failed to watch keybox.xml: {error:#}");
        }
        if let Err(error) = kmr_common::runtime::file_watch::spawn_path_watcher(
            "ommega-keybox-strongbox-watch",
            PathBuf::from(KEYBOX_STRONGBOX_PATH),
            |_trigger| {
                if let Err(reload_error) = reload_from_disk() {
                    error!("failed to reload the StrongBox keybox after change: {reload_error:#}");
                }
            },
        ) {
            error!("failed to watch the StrongBox keybox: {error:#}");
        }
    });
    Ok(())
}

pub fn update_rsa_keybox(key_der: Vec<u8>, chain: Vec<keymint::Certificate>) -> Result<bool> {
    update_keybox_file(KeyAlgorithm::Rsa, key_der, chain)
}

pub fn update_ec_keybox(key_der: Vec<u8>, chain: Vec<keymint::Certificate>) -> Result<bool> {
    update_keybox_file(KeyAlgorithm::Ec, key_der, chain)
}

fn update_keybox_file(
    algorithm: KeyAlgorithm,
    key_der: Vec<u8>,
    chain: Vec<keymint::Certificate>,
) -> Result<bool> {
    let _io_guard = KEYBOX_IO_LOCK.lock().unwrap();
    let mut keybox = KEYBOX.read().unwrap().clone();
    keybox.update_keybox(algorithm, key_der, chain)?;
    write_keybox_xml(KEYBOX_PATH, &keybox.to_xml_string())?;
    Ok(install_keybox(keybox, true, true))
}

pub fn current_identity_digest() -> [u8; 32] {
    KEYBOX.read().unwrap().identity_digest()
}

pub(crate) fn signing_certificate_ders_from_disk() -> Result<Vec<Vec<u8>>> {
    let _io_guard = KEYBOX_IO_LOCK.lock().unwrap();
    let (keybox, _) = load_keybox_with_fallback(KEYBOX_PATH)?;
    Ok(keybox.signing_certificate_ders())
}

pub struct KeyboxManager {
    /// Security level of the TA instance that owns this manager; selects which
    /// keybox material may be used for attestation.
    pub security_level: keymint::SecurityLevel,
}

impl RetrieveCertSigningInfo for KeyboxManager {
    fn signing_info(&self, key_type: SigningKeyType) -> Result<SigningInfoSnapshot, Error> {
        let keybox = KEYBOX
            .read()
            .map_err(|_| kmr_common::km_err!(UnknownError, "failed to lock KEYBOX"))?;
        keybox.signing_info_at(self.security_level, key_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_temp_keybox(name: &str, contents: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("ommega-keybox-{name}-{}.xml", std::process::id()));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn invalid_file_falls_back_in_memory_without_overwriting() {
        let path = write_temp_keybox("invalid", "<not-xml>");
        let (keybox, used_fallback) = load_keybox_with_fallback(path.to_str().unwrap()).unwrap();
        assert!(used_fallback);
        assert_eq!(keybox.identity_digest(), KeyBox::new().identity_digest());
        // The on-disk file must be left untouched so a partially-written keybox
        // isn't clobbered while the UI is still writing it.
        let written = fs::read_to_string(path).unwrap();
        assert_eq!(written, "<not-xml>");
    }

    #[test]
    fn valid_keybox_file_is_not_rewritten() {
        let path = write_temp_keybox("valid", BUNDLED_KEYBOX_XML);
        let (_, used_fallback) = load_keybox_with_fallback(path.to_str().unwrap()).unwrap();
        assert!(!used_fallback);
        let written = fs::read_to_string(path).unwrap();
        assert_eq!(written, BUNDLED_KEYBOX_XML);
    }

    #[test]
    fn explicit_bundled_template_is_retirement_eligible_before_runtime_fallback() {
        let keybox = KeyBox::from_xml_str(BUNDLED_KEYBOX_XML).unwrap();

        assert!(!is_fallback_continuation_with_state(
            &keybox,
            BUNDLED_KEYBOX_XML,
            false,
            false,
            keybox.identity_digest(),
        ));
    }

    #[test]
    fn non_bundled_keybox_is_retirement_eligible() {
        let modified_xml = format!("{BUNDLED_KEYBOX_XML}\n<!-- explicit local keybox -->\n");
        let path = write_temp_keybox("modified", &modified_xml);

        let (_, used_fallback) = load_keybox_with_fallback(path.to_str().unwrap()).unwrap();

        assert!(!used_fallback);
    }

    #[test]
    fn rewritten_bundled_template_can_continue_runtime_fallback() {
        let keybox = KeyBox::from_xml_str(BUNDLED_KEYBOX_XML).unwrap();

        assert!(is_fallback_continuation_with_state(
            &keybox,
            BUNDLED_KEYBOX_XML,
            true,
            false,
            keybox.identity_digest(),
        ));
        assert!(!is_fallback_continuation_with_state(
            &keybox,
            BUNDLED_KEYBOX_XML,
            true,
            true,
            keybox.identity_digest(),
        ));
    }
}
