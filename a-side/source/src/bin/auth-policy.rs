//! Fixed-path config writer used by the manager WebUI. No Binder or TEE calls.
use anyhow::{anyhow, Context, Result};
use kmr_common::{auth_signing::Config, runtime::fs::atomic_compare_replace_preserving_metadata};
use serde::Deserialize;
use std::{io::Read, path::Path};

const PATH: &str = "/data/misc/keystore/ommega/data/auth-signing.toml";
const MAX_CONFIG: usize = 512 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteRequest {
    expected: Option<String>,
    replacement: String,
}

fn write_request() -> Result<()> {
    if !std::env::args().skip(1).eq(["write"]) {
        return Err(anyhow!("usage: ommega-auth-policy write < request.json"));
    }
    let mut contents = Vec::new();
    std::io::stdin()
        .take((3 * MAX_CONFIG) as u64)
        .read_to_end(&mut contents)?;
    if contents.len() >= 3 * MAX_CONFIG {
        return Err(anyhow!("policy request too large"));
    }
    let request: WriteRequest =
        serde_json::from_slice(&contents).context("parse policy write request")?;
    if request.replacement.len() > MAX_CONFIG
        || request
            .expected
            .as_ref()
            .is_some_and(|s| s.len() > MAX_CONFIG)
    {
        return Err(anyhow!("policy file too large"));
    }
    let config: Config =
        toml::from_str(&request.replacement).context("parse auth-signing policy")?;
    config.validate().map_err(|error| anyhow!(error))?;
    // The normal daemon creates the data directory with Keystore ownership.
    // Never create a parallel store in a manager-accessible fallback path.
    atomic_compare_replace_preserving_metadata(
        Path::new(PATH),
        request.expected.as_deref().map(str::as_bytes),
        request.replacement.as_bytes(),
        0o600,
        kmr_common::consts::KEYSTORE_UID,
        kmr_common::consts::KEYSTORE_GID,
    )
    .context("save auth-signing policy")
}

fn main() {
    if let Err(error) = write_request() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
    println!("saved");
}
