//! Runtime policy for newly created remote authentication-signing keys.

use anyhow::{anyhow, Context, Result};
use der::Decode;
use kmr_common::auth_signing::{self, Config};
use kmr_wire::keymint::KeyParam;
use std::{fs, os::unix::fs::MetadataExt, sync::Mutex};

use crate::plat::aaid::AttestationApplicationId;

const PATH: &str = "/data/misc/keystore/ommega/data/auth-signing.toml";
const MAX_CONFIG_BYTES: u64 = 512 * 1024;
type Stamp = (u64, u64, u64, i64, i64);
type Cached = Option<(Stamp, Result<Config, String>)>;
static CACHE: Mutex<Cached> = Mutex::new(None);

fn load() -> Result<Config> {
    let mut cache = CACHE
        .lock()
        .map_err(|_| anyhow!("auth-signing policy lock poisoned"))?;
    let file = match fs::File::open(PATH) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            *cache = None;
            return Ok(Config::default());
        }
        Err(e) => return Err(e).context("open auth-signing policy"),
    };
    let meta = file.metadata().context("stat auth-signing policy")?;
    let stamp = (
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
    );
    if let Some((previous, result)) = cache.as_ref() {
        if *previous == stamp {
            return result.clone().map_err(|e| anyhow!(e));
        }
    }
    let result = (|| {
        use std::io::Read;
        if !meta.is_file() || meta.len() > MAX_CONFIG_BYTES {
            return Err(anyhow!("invalid auth-signing policy file size/type"));
        }
        let mut contents = String::new();
        file.take(MAX_CONFIG_BYTES + 1)
            .read_to_string(&mut contents)
            .context("read auth-signing policy")?;
        if contents.len() as u64 > MAX_CONFIG_BYTES {
            return Err(anyhow!("auth-signing policy too large"));
        }
        let config: Config = toml::from_str(&contents).context("parse auth-signing policy")?;
        config.validate().map_err(|e| anyhow!(e))?;
        Ok(config)
    })()
    .map_err(|e: anyhow::Error| e.to_string());
    if let Err(error) = &result {
        log::error!("Invalid auth-signing policy: {error}");
    }
    *cache = Some((stamp, result.clone()));
    result.map_err(|e| anyhow!(e))
}

pub fn authorize_on_a(
    alias: Option<&str>,
    params: &[KeyParam],
    supplied_issuer: bool,
) -> Result<bool> {
    if supplied_issuer
        || !auth_signing::eligible(params)
        || !params
            .iter()
            .any(|p| matches!(p, KeyParam::AttestationChallenge(_)))
    {
        return Ok(false);
    }
    let config = load()?;
    let app_id = params.iter().find_map(|p| match p {
        KeyParam::AttestationApplicationId(value) => Some(value.as_slice()),
        _ => None,
    });
    // AAID was added by Keystore for the authenticated caller, not by the app.
    // Reusing it keeps global interception free of additional package queries.
    let aaid = app_id
        .map(AttestationApplicationId::from_der)
        .transpose()
        .context("decode caller AAID for auth-signing policy")?;
    let packages: Vec<_> = aaid
        .as_ref()
        .into_iter()
        .flat_map(|id| id.package_info_records.iter())
        .map(|record| std::str::from_utf8(record.package_name.as_bytes()))
        .collect::<Result<_, _>>()
        .context("decode caller package names")?;
    if !config.apps.is_empty() && packages.is_empty() {
        return Err(anyhow!(
            "caller AAID missing for auth-signing app exceptions"
        ));
    }
    let mode = config.mode_for_packages(packages);
    let decision = auth_signing::authorize_on_a(mode, alias, params);
    log::info!("Auth-signing creation policy: mode={mode:?} authorize_on_a={decision}");
    Ok(decision)
}
