//! Fixed-path CPU scope writer and status reader. No service or hardware calls.
#[path = "../attestation_counts.rs"]
pub mod attestation_counts;
use anyhow::{anyhow, Context, Result};
use kmr_common::{
    cpu_mask::{Config, Profile, CONFIG_PATH, MAX_PROFILE_BYTES, PROFILE_PATH},
    runtime::fs::atomic_compare_replace_preserving_metadata,
};
use serde::Deserialize;
use serde_json::json;
use std::{io::Read, path::Path};

const MAX_CONFIG: usize = 512 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteRequest {
    expected: Option<String>,
    replacement: String,
}

fn read_config() -> Result<Option<String>> {
    match std::fs::read_to_string(CONFIG_PATH) {
        Ok(contents) if contents.len() <= MAX_CONFIG => Ok(Some(contents)),
        Ok(_) => Err(anyhow!("CPU scope too large")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn execute() -> Result<()> {
    match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [command] if command == "status" => {
            let original = read_config()?;
            let config = original
                .as_deref()
                .map(Config::parse)
                .transpose()
                .map_err(|error| anyhow!(error))?
                .unwrap_or_default();
            let profile = std::fs::read_to_string(PROFILE_PATH)
                .ok()
                .filter(|contents| contents.len() <= MAX_PROFILE_BYTES)
                .and_then(|contents| Profile::parse(&contents).ok());
            let standalone = Path::new("/data/adb/modules/zygisk_maskcpu");
            let standalone_active = standalone.is_dir()
                && !standalone.join("disable").exists()
                && !standalone.join("remove").exists();
            let stats =
                attestation_counts::read_at(Path::new(attestation_counts::DATABASE_PATH)).ok();
            println!(
                "{}",
                json!({"original": original, "config": config, "profile": profile,
                "engine": "exp6", "standalone_active": standalone_active,
                "attestation_counts": stats.as_ref().and_then(|value| value.counts.as_ref()),
                "last_b_latency_ms": stats.as_ref().and_then(|value| value.last_b_latency_ms),
                "attestation_completed_at_ms": stats.as_ref().and_then(|value| value.completed_at_ms)})
            );
        }
        [command] if command == "write" => {
            let mut bytes = Vec::new();
            std::io::stdin()
                .take((3 * MAX_CONFIG) as u64)
                .read_to_end(&mut bytes)?;
            if bytes.len() >= 3 * MAX_CONFIG {
                return Err(anyhow!("CPU scope request too large"));
            }
            let request: WriteRequest = serde_json::from_slice(&bytes)?;
            if request.replacement.len() > MAX_CONFIG
                || request
                    .expected
                    .as_ref()
                    .is_some_and(|value| value.len() > MAX_CONFIG)
            {
                return Err(anyhow!("CPU scope too large"));
            }
            Config::parse(&request.replacement).map_err(|error| anyhow!(error))?;
            atomic_compare_replace_preserving_metadata(
                Path::new(CONFIG_PATH),
                request.expected.as_deref().map(str::as_bytes),
                request.replacement.as_bytes(),
                0o600,
                kmr_common::consts::KEYSTORE_UID,
                kmr_common::consts::KEYSTORE_GID,
            )
            .context("save CPU scope")?;
            println!("saved");
        }
        _ => {
            return Err(anyhow!(
                "usage: ommega-cpu-policy status | write < request.json"
            ))
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = execute() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
