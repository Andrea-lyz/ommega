//! Runtime configuration watching and application.
//
// File loading and schema handling live in the parent module. Watch callbacks
// release the configuration read guard before resolving platform state.

use std::{path::PathBuf, sync::OnceLock};

use anyhow::Result;
use kmr_common::runtime::file_watch::{self, WatchTrigger};

use super::{
    boot_patchlevel_changed, config, load_config_file_with_retry, trust_changed_beyond_patchlevels,
    Config, ConfigLoadError, CLIENTA_CONFIG_PATH, CLIENTA_CONFIG_UI_PATH, CONFIG_PATH,
};

static CONFIG_WATCHER_STARTED: OnceLock<()> = OnceLock::new();

pub(super) fn start_config_watcher() -> Result<()> {
    if CONFIG_WATCHER_STARTED.set(()).is_err() {
        return Ok(());
    }

    file_watch::spawn_path_watcher(
        "ommega-config-watch",
        PathBuf::from(CONFIG_PATH),
        reload_runtime_config,
    )?;
    // The legacy A-side flat config (`/data/misc/keystore/ommega/config`,
    // written by the WebUI) also carries `remote`/`token`/`device_id`. Watch
    // it too so toggling "remote mode" in the WebUI applies without a restart:
    // reload_runtime_config re-reads it via load_clienta_remote_override()
    // inside Config::from_file(). Watch the parent dir, so a missing file is
    // picked up when it first appears.
    file_watch::spawn_path_watcher(
        "ommega-clienta-config-watch",
        PathBuf::from(CLIENTA_CONFIG_PATH),
        reload_runtime_config,
    )?;
    // The WebUI path is normally the same inode (a symlink), so only watch it
    // when it resolves elsewhere; otherwise every save would reload twice.
    let ui_path = PathBuf::from(CLIENTA_CONFIG_UI_PATH);
    let same_file = match (
        std::fs::canonicalize(&ui_path),
        std::fs::canonicalize(CLIENTA_CONFIG_PATH),
    ) {
        (Ok(ui), Ok(keystore)) => ui == keystore,
        (Ok(_), Err(_)) => false,
        _ => true,
    };
    if !same_file {
        file_watch::spawn_path_watcher(
            "ommega-clienta-ui-config-watch",
            ui_path,
            reload_runtime_config,
        )?;
    }
    Ok(())
}

fn reload_runtime_config(trigger: WatchTrigger) {
    let new_config_file = match load_config_file_with_retry(trigger) {
        Ok(loaded) => {
            if loaded.retries > 0 {
                log::info!(
                    "Config reload via {} succeeded after {} retr{}",
                    trigger.label(),
                    loaded.retries,
                    if loaded.retries == 1 { "y" } else { "ies" }
                );
            }
            loaded.config_file
        }
        Err(ConfigLoadError::Missing(error) | ConfigLoadError::Read(error)) => {
            log::error!("failed to read changed config file: {error}; keeping current config");
            return;
        }
        Err(ConfigLoadError::Parse(error)) => {
            log::error!("failed to parse changed config file: {error}; keeping current config");
            return;
        }
    };

    let runtime_snapshot = match config().read() {
        Ok(runtime) => runtime.clone(),
        Err(_) => {
            log::error!("config lock poisoned while snapshotting config change");
            return;
        }
    };

    let previous_trust = &runtime_snapshot.trust;
    let mut applied_trust = runtime_snapshot.trust.clone();
    let mut applied_trust_intent = runtime_snapshot.trust_intent.clone();
    let mut update_patchlevels = false;
    let mut security_patch_update = None;

    if trust_changed_beyond_patchlevels(&runtime_snapshot.trust_intent, &new_config_file.trust) {
        log::warn!(
            "Trust config changed on disk beyond patch levels; restart keymint to apply it."
        );
    } else {
        match crate::plat::vbmeta::resolve_patch_levels(&new_config_file.trust) {
            Ok(patches) => {
                update_patchlevels = previous_trust.os_patchlevel != patches.os_patchlevel
                    || previous_trust.vendor_patchlevel != patches.vendor_patchlevel
                    || boot_patchlevel_changed(
                        &previous_trust.boot_patchlevel,
                        &patches.boot_patchlevel,
                    );
                if patches.write_security_patch {
                    security_patch_update = Some((
                        patches.security_patch.clone(),
                        patches.observed_security_patch,
                    ));
                }
                applied_trust.security_patch = patches.security_patch;
                applied_trust.os_patchlevel = patches.os_patchlevel;
                applied_trust.vendor_patchlevel = patches.vendor_patchlevel;
                applied_trust.boot_patchlevel = patches.boot_patchlevel;
                applied_trust_intent = new_config_file.trust.clone();
            }
            Err(error) => {
                log::error!(
                    "failed to resolve changed patch levels; keeping previous values: {error:#}"
                );
            }
        }
    }

    let keep_crypto = runtime_snapshot.crypto != new_config_file.crypto;
    if keep_crypto {
        log::warn!("crypto config changed on disk; restart keymint to apply seed changes");
    }
    if runtime_snapshot.main.use_native_strongbox != new_config_file.main.use_native_strongbox {
        log::warn!(
            "main.use_native_strongbox changed on disk; restart keymint to switch StrongBox backend"
        );
    }

    let mut updated = Config::from_file(&new_config_file, applied_trust);
    updated.main.use_native_strongbox = runtime_snapshot.main.use_native_strongbox;
    updated.trust_intent = applied_trust_intent;
    if keep_crypto {
        updated.crypto = runtime_snapshot.crypto;
    }
    match crate::keymaster::keymint_device::apply_runtime_config_update(
        updated,
        update_patchlevels,
        security_patch_update,
    ) {
        Ok(()) => {
            // Drop the cached B-side identity profile. It is fetched once and
            // reused, so without this a saved config change (a corrected token,
            // a new server) left the daemon serving the old profile — which is
            // why toggling "enable remote" off and on in the WebUI appeared to
            // fix nothing.
            crate::remote::invalidate_identity_profile();
            log::info!("config updated via {}", trigger.label());
        }
        Err(error) => log::error!(
            "failed to apply config update via {}; keeping previous runtime: {error:#}",
            trigger.label()
        ),
    }
}
