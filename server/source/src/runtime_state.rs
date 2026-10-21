//! Small single-tenant runtime state persisted across restarts.
//!
//! Only the two admin switches live here. Upstream keeps this in its multi-tenant
//! `statedb`; this fork is one A and one B, so a tiny JSON file is enough and
//! avoids pulling in the whole state database.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const AUTOKEYBOX: &str = "autokeybox";
pub const SERVERBOX: &str = "serverbox";

const DEFAULT_PATH: &str = "data/runtime-state.json";
static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct RuntimeState {
    #[serde(default)]
    pub(crate) autokeybox: Option<bool>,
    #[serde(default)]
    pub(crate) serverbox: Option<bool>,
}

fn state_path() -> PathBuf {
    std::env::var_os("OMMEGA_RUNTIME_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_PATH))
}

pub(crate) fn read_state(path: &Path) -> RuntimeState {
    let Ok(bytes) = fs::read(path) else {
        return RuntimeState::default();
    };
    match serde_json::from_slice(&bytes) {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(
                "runtime state {} is unreadable ({error}); using defaults",
                path.display()
            );
            RuntimeState::default()
        }
    }
}

pub(crate) fn write_state(path: &Path, state: &RuntimeState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let bytes = serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    if let Err(error) = fs::rename(&tmp, path) {
        // Windows does not replace an existing file with rename; the server runs
        // on Linux, but the unit tests also run on Windows.
        if path.exists() {
            let _ = fs::remove_file(path);
            fs::rename(&tmp, path)?;
        } else {
            return Err(error);
        }
    }
    Ok(())
}

/// Persisted value for `key`, or `default` when it has never been written.
pub fn desired(key: &str, default: bool) -> bool {
    let state = read_state(&state_path());
    match key {
        AUTOKEYBOX => state.autokeybox,
        SERVERBOX => state.serverbox,
        _ => None,
    }
    .unwrap_or(default)
}

/// Persist one switch; the other key in the file is preserved.
pub fn set(key: &str, value: bool) {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let path = state_path();
    let mut state = read_state(&path);
    match key {
        AUTOKEYBOX => state.autokeybox = Some(value),
        SERVERBOX => state.serverbox = Some(value),
        _ => return,
    }
    if let Err(error) = write_state(&path, &state) {
        tracing::warn!(
            "failed to persist runtime state {}: {error}",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_round_trip_and_preserve_each_other() {
        let dir = std::env::temp_dir().join(format!("ommega-runtime-{}", std::process::id()));
        let path = dir.join("state.json");
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(read_state(&path).serverbox, None);

        let mut state = read_state(&path);
        state.serverbox = Some(true);
        write_state(&path, &state).unwrap();
        assert_eq!(read_state(&path).serverbox, Some(true));

        let mut state = read_state(&path);
        state.autokeybox = Some(false);
        write_state(&path, &state).unwrap();
        let state = read_state(&path);
        assert_eq!(state.serverbox, Some(true));
        assert_eq!(state.autokeybox, Some(false));

        let _ = fs::remove_dir_all(&dir);
    }
}
