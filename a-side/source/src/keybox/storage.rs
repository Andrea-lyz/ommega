//! Keybox file persistence, fallback loading and optional StrongBox attachment.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use log::{debug, info, warn};

use super::{is_fallback_continuation, KeyBox, BUNDLED_KEYBOX_XML, KEYBOX_STRONGBOX_PATH};

fn temp_keybox_path(path: &str) -> PathBuf {
    let target = Path::new(path);
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("keybox.xml");
    let temp_name = format!(".{file_name}.tmp-{}", std::process::id());
    target
        .parent()
        .map(|parent| parent.join(&temp_name))
        .unwrap_or_else(|| PathBuf::from(temp_name))
}

pub(super) fn write_keybox_xml(path: &str, xml: &str) -> Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create keybox directory {}", parent.display()))?;
    }
    let temp_path = temp_keybox_path(path);
    {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)
            .with_context(|| {
                format!(
                    "failed to open temporary keybox.xml {}",
                    temp_path.display()
                )
            })?;
        file.write_all(xml.as_bytes()).with_context(|| {
            format!(
                "failed to write temporary keybox.xml {}",
                temp_path.display()
            )
        })?;
        file.sync_all().with_context(|| {
            format!(
                "failed to sync temporary keybox.xml {}",
                temp_path.display()
            )
        })?;
    }

    #[cfg(windows)]
    if Path::new(path).exists() {
        fs::remove_file(path)
            .with_context(|| format!("failed to replace keybox.xml at {path} on Windows"))?;
    }

    fs::rename(&temp_path, path)
        .with_context(|| format!("failed to atomically replace keybox.xml at {path}"))
}

fn write_bundled_keybox(path: &str) -> Result<()> {
    write_keybox_xml(path, BUNDLED_KEYBOX_XML)
}

pub fn ensure_keybox_file(path: &str) -> Result<()> {
    if Path::new(path).exists() {
        return Ok(());
    }
    info!("keybox.xml missing at {}; seeding bundled template", path);
    write_bundled_keybox(path)
}

pub(super) fn load_keybox_with_fallback(path: &str) -> Result<(KeyBox, bool)> {
    let (mut keybox, used_fallback) = load_batch_keybox(path)?;
    attach_strongbox_material(&mut keybox, KEYBOX_STRONGBOX_PATH);
    Ok((keybox, used_fallback))
}

/// Attach the optional dedicated StrongBox keybox. A missing file simply leaves
/// StrongBox attestation unavailable; an invalid file is reported and ignored so
/// that the batch keybox keeps working.
fn attach_strongbox_material(keybox: &mut KeyBox, path: &str) {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            debug!("no dedicated StrongBox keybox at {path}");
            return;
        }
        Err(error) => {
            warn!("failed to read {path}: {error:#}");
            return;
        }
    };
    match keybox.set_strongbox_material_from_xml(&contents) {
        Ok(()) => info!("loaded dedicated StrongBox keybox from {path}"),
        Err(error) => warn!(
            "invalid StrongBox keybox at {path}: {error:#}; StrongBox attestation stays unavailable"
        ),
    }
}

fn load_batch_keybox(path: &str) -> Result<(KeyBox, bool)> {
    match fs::read_to_string(path) {
        Ok(contents) => match KeyBox::from_xml_str(&contents) {
            Ok(keybox) => {
                let fallback_origin = is_fallback_continuation(&keybox, &contents);
                Ok((keybox, fallback_origin))
            }
            Err(error) => {
                // Do not overwrite the on-disk file here. The webroot UI writes
                // keybox.xml via `mv` + `cat << EOF`, and the inotify watcher can
                // observe an empty/partial file while the new contents are still
                // being written. Overwriting it with the bundled template here
                // would destroy a just-written valid keybox. Instead we fall back
                // to the bundled template in memory and leave the file untouched;
                // a later `IN_CLOSE_WRITE` event re-reads the completed file.
                warn!(
                    "invalid keybox.xml at {}: {:#}; using bundled template in memory (file left intact)",
                    path, error
                );
                Ok((KeyBox::new(), true))
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            info!(
                "keybox.xml missing at {}; using bundled template in memory",
                path
            );
            Ok((KeyBox::new(), true))
        }
        Err(error) => Err(error).with_context(|| format!("failed to read keybox.xml from {path}")),
    }
}
