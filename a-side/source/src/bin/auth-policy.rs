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

fn policy_destination(command: &str, replacement: &str) -> Result<(&'static str, u32)> {
    match command {
        "write" => {
            let config: Config =
                toml::from_str(replacement).context("parse auth-signing policy")?;
            config.validate().map_err(|error| anyhow!(error))?;
            Ok((PATH, 0o600))
        }
        "write-target" => Ok(("/data/misc/keystore/ommega/target.txt", 0o644)),
        "write-system-apps" => Ok(("/data/misc/keystore/ommega/system_app", 0o644)),
        "write-local-keygen" => {
            let mut seen = std::collections::BTreeSet::new();
            for line in replacement.lines().map(str::trim) {
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if !kmr_common::cpu_mask::valid_package(line) || !seen.insert(line.to_string()) {
                    return Err(anyhow!("invalid local-keygen policy"));
                }
            }
            Ok(("/data/misc/keystore/ommega/data/local-keygen.txt", 0o600))
        }
        "write-target-security" => {
            let table: toml::Table =
                toml::from_str(replacement).context("parse target security policy")?;
            let valid_version = table.get("version").and_then(toml::Value::as_integer) == Some(1);
            let valid_packages = table
                .get("packages")
                .and_then(toml::Value::as_table)
                .is_some_and(|packages| {
                    packages.iter().all(|(package, mode)| {
                        kmr_common::cpu_mask::valid_package(package)
                            && mode.as_str().is_some_and(|value| {
                                matches!(value, "global_default" | "strongbox" | "tee")
                            })
                    })
                });
            if table.len() != 2 || !valid_version || !valid_packages {
                return Err(anyhow!("invalid target security policy"));
            }
            Ok(("/data/misc/keystore/ommega/target-security.toml", 0o644))
        }
        _ => Err(anyhow!("unknown policy operation")),
    }
}

fn write_request() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [command] = args.as_slice() else {
        return Err(anyhow!(
            "usage: ommega-auth-policy write|write-target|write-target-security|write-system-apps|write-local-keygen < request.json"
        ));
    };
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
    let (path, mode) = policy_destination(command, &request.replacement)?;
    // Only the selected fixed destination is writable. The normal daemon
    // prepares the parent directory with Keystore ownership.
    atomic_compare_replace_preserving_metadata(
        Path::new(path),
        request.expected.as_deref().map(str::as_bytes),
        request.replacement.as_bytes(),
        mode,
        kmr_common::consts::KEYSTORE_UID,
        kmr_common::consts::KEYSTORE_GID,
    )
    .context("save policy")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The legacy operations answer with the single word "saved"; the SOTER
    // management interface answers with one JSON document. They must never
    // share a process path, or the extra word corrupts the JSON the WebUI reads.
    let result = match args.first() {
        Some(command) if command.starts_with("soter-") => soter_request(&args),
        _ => write_request().map(|()| println!("saved")),
    };
    if let Err(error) = result {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn soter_request(args: &[String]) -> Result<()> {
    let command = args
        .first()
        .ok_or_else(|| anyhow!("Expected one SOTER management operation"))?;
    if command != "soter-read" && args.len() != 1 {
        return Err(anyhow!("SOTER management takes no extra argument"));
    }
    let _lock = if command != "soter-read" {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::PermissionsExt;
        let directory = Path::new(soter_ta::identities::STORE).parent().unwrap();
        std::fs::create_dir_all(directory.join("keys"))?;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        let lock = std::fs::File::open(directory.join("keys"))?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(anyhow!(
                "Another SOTER write is active; reload before saving"
            ));
        }
        Some(lock)
    } else {
        None
    };
    let _live_guard = if command != "soter-read" {
        soter_ta::identities::lock_policy(true).map_err(|e| anyhow!(e))?
    } else {
        None
    };
    let response = if command == "soter-read" {
        soter_ta::management::read_for(args.get(1).map(String::as_str)).map_err(|e| anyhow!(e))?
    } else {
        let mut raw = Vec::new();
        std::io::stdin()
            .take((3 * MAX_CONFIG + 1) as u64)
            .read_to_end(&mut raw)?;
        if raw.len() > 3 * MAX_CONFIG {
            return Err(anyhow!("SOTER request too large"));
        }
        let request = serde_json::from_slice(&raw).context("parse SOTER management request")?;
        soter_ta::management::mutate(command, request, |expected, replacement| {
            let path = Path::new(soter_ta::identities::STORE);
            std::fs::create_dir_all(path.parent().unwrap())
                .map_err(|_| "Cannot create SOTER policy directory")?;
            atomic_compare_replace_preserving_metadata(path, expected, replacement, 0o600, 0, 0)
                .map_err(|e| e.to_string())
        })
        .map_err(|e| anyhow!(e))?
    };
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_policy_destinations_are_fixed_and_security_toml_is_checked() {
        assert_eq!(
            policy_destination("write-target", "com.example.app\n").unwrap(),
            ("/data/misc/keystore/ommega/target.txt", 0o644)
        );
        assert_eq!(
            policy_destination("write-target-security", "version = 1\n[packages]\n").unwrap(),
            ("/data/misc/keystore/ommega/target-security.toml", 0o644)
        );
        assert!(policy_destination("write-target-security", "[invalid").is_err());
        assert!(policy_destination("write-target-security", "version = 2\n[packages]\n").is_err());
        assert!(policy_destination(
            "write-target-security",
            "version = 1\n[packages]\n\"com.example.app\" = \"bad\"\n"
        )
        .is_err());
        assert_eq!(
            policy_destination("write-system-apps", "").unwrap(),
            ("/data/misc/keystore/ommega/system_app", 0o644)
        );
        assert_eq!(
            policy_destination("write-local-keygen", "com.example.app\n").unwrap(),
            ("/data/misc/keystore/ommega/data/local-keygen.txt", 0o600)
        );
        assert!(policy_destination("/tmp/unrelated", "").is_err());
        assert!(policy_destination("write", "[invalid").is_err());
        assert!(policy_destination("write-local-keygen", "bad package\n").is_err());
    }
}
