//! Fixed-path management used by the existing Rust policy writer. Never returns private keys.
use crate::identities::{self, Snapshot, Store, LEGACY_STORE, MAX_KEY, MAX_POLICY, STORE};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Presence of this flag asks the watchdog to run the software TA.
pub const FLAG: &str = "/data/adb/ommega/soterta/enabled";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub expected: Option<String>,
    #[serde(default)]
    pub legacy_revision: Option<String>,
    #[serde(default)]
    pub store: Option<Store>,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub cpu_id: String,
}
#[derive(Serialize)]
pub struct IdentityStatus {
    pub ready: bool,
    pub note: String,
    pub cpu_id: Option<String>,
    pub fingerprint: Option<String>,
}
#[derive(Serialize)]
pub struct Response {
    pub store: Store,
    pub expected: Option<String>,
    pub legacy_revision: Option<String>,
    pub status: BTreeMap<String, IdentityStatus>,
    pub warnings: Vec<String>,
    pub shared: BTreeMap<String, Vec<String>>,
    /// Whether the software TA is switched on for the whole device.
    pub enabled: bool,
    /// SOTER transactions per package, for the application badge.
    pub counts: BTreeMap<String, u64>,
}
/// The packages sharing the requested package UID, when that UID has more than one.
///
/// Only the requested package resolves a group: a real device has dozens of
/// large shared-UID clusters and returning all of them would flood the manager
/// bridge that displays a single application.
fn shared_packages(raw: &str, package: &str) -> Option<Vec<String>> {
    let mut groups: BTreeMap<u32, std::collections::BTreeSet<String>> = BTreeMap::new();
    for line in raw.lines() {
        let mut parts = line.split_whitespace();
        let (Some(pkg), Some(uid)) = (parts.next(), parts.next()) else {
            continue;
        };
        if let Ok(uid) = uid.parse::<u32>() {
            if identities::valid_package(pkg) {
                groups
                    .entry(uid % 100_000)
                    .or_default()
                    .insert(pkg.to_owned());
            }
        }
    }
    groups
        .values()
        .find(|group| group.len() > 1 && group.contains(package))
        .map(|group| group.iter().cloned().collect())
}
/// Transactions per installed package, so the app list can badge a row without
/// asking the daemon once per application.
fn request_counts(ledger: Option<&crate::state::TaState>, packages: &str) -> BTreeMap<String, u64> {
    const MAX_ENTRIES: usize = 2048;
    let Some(ledger) = ledger else {
        return BTreeMap::new();
    };
    let mut names: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for line in packages.lines() {
        let mut fields = line.split_whitespace();
        let (Some(package), Some(uid)) = (fields.next(), fields.next()) else {
            continue;
        };
        if let Ok(uid) = uid.parse::<u32>() {
            names.entry(uid).or_default().push(package.to_owned());
        }
    }
    let mut counts = BTreeMap::new();
    for (uid, entry) in &ledger.uids {
        if entry.requests == 0 || counts.len() >= MAX_ENTRIES {
            continue;
        }
        for package in names.get(uid).into_iter().flatten() {
            if counts.len() >= MAX_ENTRIES {
                break;
            }
            counts.insert(package.clone(), entry.requests);
        }
    }
    counts
}

fn read_policy(path: &Path) -> Result<Option<String>, String> {
    match fs::read(path) {
        Ok(raw) => {
            if raw.len() > MAX_POLICY {
                return Err("SOTER identity policy too large".into());
            }
            String::from_utf8(raw)
                .map(Some)
                .map_err(|_| "SOTER policy is not UTF-8".into())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Cannot read SOTER identity policy".into()),
    }
}
pub fn read() -> Result<Response, String> {
    read_for(None)
}

pub fn read_for(package: Option<&str>) -> Result<Response, String> {
    let expected = read_policy(Path::new(STORE))?;
    let legacy = if expected.is_none() {
        read_policy(Path::new(LEGACY_STORE))?
    } else {
        None
    };
    let store = match expected.as_ref().or(legacy.as_ref()) {
        Some(raw) => Store::parse(raw.as_bytes())?,
        None => Store::default(),
    };
    let mut status = BTreeMap::new();
    // Read local material only to expose public metadata; no ledger bytes leave Rust.
    let ledger = fs::read("/data/adb/ommega/soterta/state.json")
        .ok()
        .and_then(|raw| crate::state::TaState::from_json(&raw));
    for r in &store.identities {
        let snapshot = Snapshot::memory(Store {
            default: r.id.clone(),
            identities: vec![r.clone()],
            ..Store::default()
        });
        let ready = snapshot.resolve(Some(0), "");
        let (cpu_id, fingerprint) = match &ready {
            Ok(identities::Selection::Key { cpu_id, pair, .. }) => (
                Some(cpu_id.clone()),
                Some(identities::digest(pair.public_pem.as_bytes())),
            ),
            Ok(identities::Selection::Local { id }) => ledger
                .as_ref()
                .and_then(|s| s.identities.get(id))
                .map(|m| {
                    (
                        Some(m.cpu_id.clone()),
                        Some(identities::digest(m.key.public_pem.as_bytes())),
                    )
                })
                .unwrap_or((None, None)),
            _ => (None, None),
        };
        status.insert(
            r.id.clone(),
            IdentityStatus {
                ready: ready.is_ok(),
                note: ready.err().unwrap_or_default(),
                cpu_id,
                fingerprint,
            },
        );
    }
    let counts = request_counts(
        ledger.as_ref(),
        &fs::read_to_string(identities::PACKAGES).unwrap_or_default(),
    );
    let shared = package
        .filter(|package| identities::valid_package(package))
        .and_then(|package| {
            shared_packages(&fs::read_to_string(identities::PACKAGES).ok()?, package)
                .map(|group| (package.to_owned(), group))
        })
        .into_iter()
        .collect();
    Ok(Response {
        store,
        expected,
        legacy_revision: legacy.map(|raw| identities::digest(raw.as_bytes())),
        status,
        warnings: Vec::new(),
        shared,
        enabled: Path::new(FLAG).is_file(),
        counts,
    })
}
fn verify_expected(request: &Request, current: &Response) -> Result<(), String> {
    if request.expected != current.expected || request.legacy_revision != current.legacy_revision {
        return Err("Config changed; reload before saving".into());
    }
    Ok(())
}
fn validate_references(store: &Store) -> Result<(), String> {
    store.validate()?;
    let snapshot = Snapshot::memory(store.clone());
    let ids = store
        .apps
        .values()
        .chain((!store.default.is_empty()).then_some(&store.default));
    for id in ids {
        let mut single = snapshot.store.clone();
        single.default = id.clone();
        single.apps.clear();
        Snapshot::memory(single).resolve(Some(0), "")?;
    }
    Ok(())
}
fn owned_key(path: &str) -> bool {
    Path::new(path).parent() == Path::new(STORE).parent().map(|p| p.join("keys")).as_deref()
        || Path::new(path).parent()
            == Path::new(LEGACY_STORE)
                .parent()
                .map(|p| p.join("identities"))
                .as_deref()
}

/// The caller supplies the repository's locked, atomic compare-and-replace helper.
pub fn mutate(
    operation: &str,
    request: Request,
    commit: impl FnOnce(Option<&[u8]>, &[u8]) -> Result<(), String>,
) -> Result<Response, String> {
    let current = read()?;
    verify_expected(&request, &current)?;
    let mut store = if operation == "soter-import" {
        current.store.clone()
    } else {
        request.store.clone().ok_or("Missing SOTER replacement")?
    };
    if operation == "soter-write" {
        for record in &store.identities {
            if !record.pem.is_empty()
                && current.store.identities.iter().all(|old| {
                    old.id != record.id
                        || old.pem != record.pem
                        || old.cpu_id != record.cpu_id
                        || old.kind != record.kind
                })
            {
                return Err("Use private-key import to change identity material".into());
            }
        }
    }
    let mut created: Option<PathBuf> = None;
    if operation == "soter-import" {
        let r = store
            .identities
            .iter_mut()
            .find(|r| r.id == request.id)
            .ok_or("SOTER identity does not exist")?;
        if r.kind != "key" {
            return Err("Only a key identity can import a private key".into());
        }
        if !Path::new(&request.path).is_absolute() {
            return Err("Select an absolute on-device private-key path".into());
        }
        let mut input = OpenOptions::new();
        input.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            input.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = input
            .open(&request.path)
            .map_err(|_| "Cannot open the selected regular key file")?;
        if !file
            .metadata()
            .map_err(|_| "Cannot inspect the selected key file")?
            .is_file()
        {
            return Err("Select a regular private-key file".into());
        }
        let mut raw = String::new();
        file.take((MAX_KEY + 1) as u64)
            .read_to_string(&mut raw)
            .map_err(|_| "Cannot read the selected key file")?;
        if raw.len() > MAX_KEY {
            return Err("SOTER key file too large".into());
        }
        let (pair, cpu) = decode_key_file(&raw, &request.cpu_id)?;
        let directory = Path::new(STORE).parent().unwrap().join("keys");
        fs::create_dir_all(&directory).map_err(|_| "Cannot create SOTER key directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| "Cannot protect SOTER key directory")?;
        }
        let mut nonce = [0u8; 16];
        OsRng.fill_bytes(&mut nonce);
        let path = directory.join(format!("{}-{}.pem", r.id, hex::encode(nonce)));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|_| "Cannot create SOTER private-key file")?;
        if file
            .write_all(pair.private_pem.as_bytes())
            .and_then(|_| file.sync_all())
            .is_err()
        {
            let _ = fs::remove_file(&path);
            return Err("Cannot persist SOTER private key".into());
        }
        #[cfg(unix)]
        if fs::File::open(&directory)
            .and_then(|dir| dir.sync_all())
            .is_err()
        {
            let _ = fs::remove_file(&path);
            return Err("Cannot sync SOTER key directory".into());
        }
        created = Some(path.clone());
        r.cpu_id = cpu;
        r.pem = path.to_str().ok_or("Invalid SOTER key path")?.to_owned();
        r.fingerprint = identities::digest(pair.public_pem.as_bytes());
    } else if operation != "soter-write" {
        return Err("Unknown SOTER management operation".into());
    }
    let result = (|| {
        validate_references(&store)?;
        if !store.apps.is_empty() {
            let packages = fs::read_to_string(identities::PACKAGES)
                .map_err(|_| "Cannot check shared-UID SOTER policy")?;
            for line in packages.lines() {
                let mut fields = line.split_whitespace();
                let (Some(pkg), Some(uid)) = (fields.next(), fields.next()) else {
                    continue;
                };
                if store.apps.contains_key(pkg) {
                    store.selected_id(
                        Some(uid.parse().map_err(|_| "Invalid package UID")?),
                        &packages,
                    )?;
                }
            }
        }
        verify_expected(&request, &read()?)?;
        let contents =
            serde_json::to_vec_pretty(&store).map_err(|_| "Cannot encode SOTER policy")?;
        if contents.len() > MAX_POLICY {
            return Err("SOTER policy too large".into());
        }
        commit(current.expected.as_deref().map(str::as_bytes), &contents)?;
        let saved = read()?;
        if saved.expected.as_deref().map(str::as_bytes) != Some(contents.as_slice()) {
            return Err("SOTER policy readback mismatch".into());
        }
        Ok(saved)
    })();
    let mut saved = match result {
        Ok(saved) => saved,
        Err(error) => {
            if let Some(path) = created {
                let safe_to_remove = read()
                    .is_ok_and(|r| r.store.identities.iter().all(|r| Path::new(&r.pem) != path));
                if safe_to_remove {
                    let _ = fs::remove_file(path);
                }
            }
            return Err(error);
        }
    };
    for previous in &current.store.identities {
        if !previous.pem.is_empty()
            && owned_key(&previous.pem)
            && !saved.store.identities.iter().any(|r| r.pem == previous.pem)
            && fs::remove_file(&previous.pem).is_err()
            && Path::new(&previous.pem).exists()
        {
            saved
                .warnings
                .push("Policy saved, but a retired key file could not be removed".into());
        }
    }
    Ok(saved)
}

fn decode_key_file(raw: &str, cpu_input: &str) -> Result<(crate::state::KeyPair, String), String> {
    if raw.len() > MAX_KEY {
        return Err("SOTER key file too large".into());
    }
    let (pem, cpu) = if raw.trim_start().starts_with('{') {
        let bundle: serde_json::Value =
            serde_json::from_str(raw).map_err(|_| "Invalid SOTER key bundle")?;
        let pem = ["attk_private_pem", "attk_priv", "private_pem"]
            .iter()
            .find_map(|key| bundle.get(key).and_then(|v| v.as_str()))
            .ok_or("Key bundle has no private key")?;
        (
            pem.to_owned(),
            bundle
                .get("cpu_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(cpu_input)
                .to_ascii_lowercase(),
        )
    } else {
        (raw.to_owned(), cpu_input.to_ascii_lowercase())
    };
    if !identities::valid_cpu_id(&cpu) {
        return Err("cpu_id must contain exactly 32 hexadecimal characters".into());
    }
    Ok((identities::validate_key(&pem)?, cpu))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response() -> Response {
        Response {
            store: Store::default(),
            expected: Some("new snapshot".into()),
            legacy_revision: None,
            status: BTreeMap::new(),
            warnings: Vec::new(),
            shared: BTreeMap::new(),
            enabled: true,
            counts: BTreeMap::new(),
        }
    }
    #[test]
    fn stale_and_migrated_snapshots_reject_the_write() {
        let mut request = Request {
            expected: Some("old snapshot".into()),
            legacy_revision: None,
            store: None,
            id: String::new(),
            path: String::new(),
            cpu_id: String::new(),
        };
        let current = response();
        assert!(verify_expected(&request, &current).is_err());
        request.expected = current.expected.clone();
        assert!(verify_expected(&request, &current).is_ok());
        request.legacy_revision = Some("another legacy revision".into());
        assert!(verify_expected(&request, &current).is_err());
    }
    #[test]
    fn deletion_cannot_leave_application_references() {
        let mut store = Store::default();
        store
            .apps
            .insert("org.example.one".into(), "removed".into());
        assert!(validate_references(&store).is_err());
        store.apps.clear();
        assert!(validate_references(&store).is_ok());
    }
    #[test]
    fn imported_pem_and_json_require_real_private_material() {
        let state = crate::state::TaState::generate_local().unwrap();
        let cpu = "a".repeat(32);
        assert!(decode_key_file(&state.attk.public_pem, &cpu).is_err());
        assert!(decode_key_file(&state.attk.private_pem, "").is_err());
        let (pair, id) = decode_key_file(&state.attk.private_pem, &cpu).unwrap();
        assert_eq!(pair.public_pem, state.attk.public_pem);
        assert_eq!(id, cpu);
        let bundle = serde_json::json!({"cpu_id":cpu.to_ascii_uppercase(),"private_pem":state.attk.private_pem});
        let (_, id) = decode_key_file(&bundle.to_string(), "").unwrap();
        assert_eq!(id, cpu);
        assert!(!serde_json::to_string(&response())
            .unwrap()
            .contains("PRIVATE KEY"));
    }
    #[test]
    fn shared_groups_are_sorted_and_work_across_profiles() {
        let listing =
            "org.example.one 11000 0 x\norg.example.two 111000 0 x\norg.example.one 11000 0 x\n";
        assert_eq!(
            shared_packages(listing, "org.example.one").as_deref(),
            Some(["org.example.one".to_owned(), "org.example.two".to_owned()].as_slice())
        );
        assert_eq!(
            shared_packages(listing, "org.example.two"),
            shared_packages(listing, "org.example.one")
        );
        // A package that does not share its UID reports no group, and an
        // unrelated large cluster never appears in that answer.
        assert!(shared_packages(listing, "org.example.three").is_none());
        let system = "android 1000 0 x\ncom.android.settings 1000 0 x\n";
        assert_eq!(
            shared_packages(system, "com.android.settings").as_deref(),
            Some(["android".to_owned(), "com.android.settings".to_owned()].as_slice())
        );
    }
}
