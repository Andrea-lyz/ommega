//! Fixed-path WebUI policy writer and early-boot XML preparation. No HAL calls.
#[path = "../capability_mask.rs"]
mod capability_mask;

use anyhow::{anyhow, Context, Result};
use capability_mask::{Config, CONFIG_PATH, MAX_CONFIG, XML_PATH};
use kmr_common::runtime::fs::{
    atomic_compare_replace_preserving_metadata, atomic_replace_preserving_metadata,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{io::Read, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteRequest {
    expected: Option<String>,
    replacement: String,
}

fn read_config() -> Result<Option<String>> {
    match std::fs::read_to_string(CONFIG_PATH) {
        Ok(text) if text.len() <= MAX_CONFIG => Ok(Some(text)),
        Ok(_) => Err(anyhow!("System capability policy too large")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn config_json(config: Config) -> Value {
    json!({"strongbox": config.strongbox, "device_id_attestation": config.device_id_attestation})
}

fn parse_config(original: Option<&str>) -> Result<Config> {
    original
        .map(Config::parse)
        .transpose()
        .map_err(|error| anyhow!(error))
        .map(Option::unwrap_or_default)
}

fn prepare(module: &Path) -> Result<()> {
    let desired = read_config().and_then(|text| parse_config(text.as_deref()));
    // Never leave a previous boot's mask behind when a hand-edited policy is invalid.
    let config = desired.as_ref().copied().unwrap_or_default();
    atomic_replace_preserving_metadata(
        &module.join(XML_PATH),
        config.permissions_xml().as_bytes(),
        0o644,
        0,
        0,
    )
    .context("prepare system capability XML")?;
    desired?;
    Ok(())
}

fn execute() -> Result<()> {
    match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [command] if command == "status" => {
            let original = read_config()?;
            let config = parse_config(original.as_deref())?;
            let mounted = std::fs::read_to_string(format!("/{XML_PATH}"));
            let (mounted_config, mount_error) = match mounted {
                Ok(xml) => match Config::from_xml(&xml) {
                    Ok(active) => (Some(config_json(active)), None),
                    Err(error) => (None, Some(error.to_owned())),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, None),
                Err(error) => (None, Some(error.to_string())),
            };
            println!(
                "{}",
                json!({"original": original, "config": config_json(config),
                "mounted": mounted_config, "mount_error": mount_error})
            );
        }
        [command] if command == "write" => {
            let mut bytes = Vec::new();
            std::io::stdin()
                .take((3 * MAX_CONFIG) as u64)
                .read_to_end(&mut bytes)?;
            if bytes.len() >= 3 * MAX_CONFIG {
                return Err(anyhow!("System capability request too large"));
            }
            let request: WriteRequest = serde_json::from_slice(&bytes)?;
            if request
                .expected
                .as_ref()
                .is_some_and(|text| text.len() > MAX_CONFIG)
            {
                return Err(anyhow!("System capability policy too large"));
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
            .context("save system capability policy; reload if another editor changed it")?;
            println!("saved");
        }
        [command, module] if command == "prepare" => prepare(Path::new(module))?,
        _ => {
            return Err(anyhow!(
                "usage: ommega-capability-policy status | write < request.json | prepare MODULE"
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
