//! Per-uid SOTER identities from the A-side UI store.
//!
//! The UI keeps `/data/adb/ommega/soterta/identities.json`: a list of identities
//! (local / key / relay) plus a package -> identity map. The TA resolves the
//! caller's package through `/data/system/packages.list` and uses that identity's
//! device id, and - for an imported key identity - its ATTK key, instead of the
//! ledger-wide one. Anything the store does not cover keeps the old behaviour.

use std::fs;
use std::path::Path;

use rsa::pkcs8::DecodePrivateKey;
use rsa::RsaPrivateKey;
use serde_json::Value;

pub const STORE: &str = "/data/adb/ommega/soterta/identities.json";
pub const PACKAGES: &str = "/data/system/packages.list";

/// One resolved identity: the device id every blob for this uid must carry, and
/// the ATTK key that signs its ASK when the identity is an imported key.
pub struct Identity {
    /// Identity id from the store (the key the app map points at).
    pub id: String,
    /// "local", "key" or "relay".
    pub kind: String,
    /// Device id carried by every blob for this uid; a local identity has none
    /// until the TA mints its own key.
    pub cpu_id: Option<String>,
    /// Imported ATTK for a "key" identity.
    pub key: Option<RsaPrivateKey>,
    /// Relay target for a "relay" identity.
    pub url: Option<String>,
    pub token: Option<String>,
    pub device_id: Option<String>,
    pub tls_insecure: bool,
}

/// Resolve the identity the UI assigned to `uid`, if any.
pub fn for_uid(uid: u32) -> Option<Identity> {
    for_uid_at(uid, Path::new(STORE), Path::new(PACKAGES))
}

/// Testable form with explicit paths.
pub fn for_uid_at(uid: u32, store: &Path, packages: &Path) -> Option<Identity> {
    let package = package_of(uid, packages)?;
    let raw = fs::read_to_string(store).ok()?;
    let json: Value = serde_json::from_str(&raw).ok()?;
    let id = json.get("apps")?.get(package.as_str())?.as_str()?;
    let item = json
        .get("identities")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id))?;
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("local").to_string();
    let cpu_id = item.get("cpu_id").and_then(Value::as_str).map(str::to_string);
    let url = item.get("url").and_then(Value::as_str).map(str::to_string);
    let token = item.get("token").and_then(Value::as_str).map(str::to_string);
    let device_id = item.get("device_id").and_then(Value::as_str).map(str::to_string);
    let tls_insecure = item.get("tls_insecure").and_then(Value::as_bool).unwrap_or(false);
    let key = item
        .get("pem")
        .and_then(Value::as_str)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|pem| RsaPrivateKey::from_pkcs8_pem(&pem).ok());
    Some(Identity {
        id: id.to_string(),
        kind: kind,
        cpu_id: cpu_id,
        key: key,
        url: url,
        token: token,
        device_id: device_id,
        tls_insecure: tls_insecure,
    })
}

fn package_of(uid: u32, packages: &Path) -> Option<String> {
    let raw = fs::read_to_string(packages).ok()?;
    for line in raw.lines() {
        let mut fields = line.split_whitespace();
        let package = fields.next()?;
        let value = fields.next()?;
        if value.parse::<u32>().ok() == Some(uid) {
            return Some(package.to_string());
        }
    }
    None
}

/// Relay target for a uid whose identity is a relay, if it is fully configured.
pub fn relay_config(uid: u32) -> Option<crate::remote::Config> {
    let identity = for_uid(uid)?;
    if identity.kind != "relay" {
        return None;
    }
    let url = identity.url.filter(|value| !value.is_empty())?;
    Some(crate::remote::Config {
        enabled: true,
        url: url,
        token: identity.token.unwrap_or_default(),
        device_id: identity.device_id.unwrap_or_default(),
        tls_insecure: identity.tls_insecure,
        uid_map: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture(dir: &Path, store: &str, packages: &str) {
        fs::write(dir.join("identities.json"), store).unwrap();
        fs::write(dir.join("packages.list"), packages).unwrap();
    }

    #[test]
    fn an_assigned_package_resolves_its_identity() {
        let dir = std::env::temp_dir().join(format!("ommega-identities-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fixture(&dir,
            "{\"version\":1,\"apps\":{\"com.tencent.mm\":\"key-1\"},\"identities\":[{\"id\":\"key-1\",\"type\":\"key\",\"cpu_id\":\"090000005171734c42866bea148b21f5\"}]}",
            "com.tencent.mm 10503 0 /data/user/0/com.tencent.mm default:targetSdkVersion=34\n");
        let identity = for_uid_at(10503, &dir.join("identities.json"), &dir.join("packages.list")).unwrap();
        assert_eq!(identity.cpu_id.as_deref(), Some("090000005171734c42866bea148b21f5"));
        assert_eq!(identity.kind, "key");
        assert_eq!(identity.id, "key-1");
        assert!(identity.key.is_none(), "no pem recorded, so no key");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unassigned_or_unknown_uids_fall_back() {
        let dir = std::env::temp_dir().join(format!("ommega-identities-empty-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fixture(&dir,
            "{\"version\":1,\"apps\":{},\"identities\":[]}",
            "com.tencent.mm 10503 0 /data/user/0/com.tencent.mm default\n");
        assert!(for_uid_at(10503, &dir.join("identities.json"), &dir.join("packages.list")).is_none());
        assert!(for_uid_at(4242, &dir.join("identities.json"), &dir.join("packages.list")).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_store_is_not_an_error() {
        let dir = std::env::temp_dir().join(format!("ommega-identities-missing-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("packages.list"), "com.tencent.mm 10503 0 x\n").unwrap();
        assert!(for_uid_at(10503, &dir.join("identities.json"), &dir.join("packages.list")).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
