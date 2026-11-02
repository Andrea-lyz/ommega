//! Periodic trigger for the automated RKP renewal.
//!
//! The decision logic lives in the `ommega-rkp auto-run` subcommand, which owns
//! the seed, the renewal window and the issuance lock. This thread only starts a
//! child process at a fixed interval, so a slow or failing request can never hold
//! the TA: the engine, the HTTP client and the keybox writer all run outside this
//! process. Failures are logged and retried on the next tick; the child applies
//! its own backoff so a broken registration cannot burn a request every tick.

use std::{path::Path, process::Command, thread, time::Duration};

use log::{debug, info, warn};

const CLIS: [&str; 2] = [
    "/data/adb/modules/ommega/libs/arm64-v8a/ommega-rkp",
    "/data/adb/modules/ommega/libs/x86_64/ommega-rkp",
];
/// Wait for the device to settle before the first check.
const FIRST_DELAY: Duration = Duration::from_secs(300);
const INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Start the renewal thread once. Silently does nothing when the tool is not
/// installed, so a partially installed module never logs noise on every boot.
pub fn spawn() {
    let Some(cli) = CLIS
        .iter()
        .map(Path::new)
        .find(|candidate| candidate.is_file())
    else {
        debug!("ommega-rkp not installed; automatic RKP renewal stays off");
        return;
    };
    let cli = cli.to_path_buf();
    let spawned = thread::Builder::new()
        .name("ommega-rkp-auto".to_string())
        .spawn(move || {
            thread::sleep(FIRST_DELAY);
            loop {
                run_once(&cli);
                thread::sleep(INTERVAL);
            }
        });
    if let Err(error) = spawned {
        warn!("could not start the automatic RKP renewal thread: {error}");
    } else {
        info!(
            "automatic RKP renewal checks every {} minutes",
            INTERVAL.as_secs() / 60
        );
    }
}

fn run_once(cli: &Path) {
    match Command::new(cli).arg("auto-run").output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let text = stdout.trim();
            // "skipped" answers are normal; only an actual run is worth a line.
            if text.contains("\"ran\":true") {
                info!("automatic RKP renewal: {text}");
            } else {
                debug!("automatic RKP renewal: {text}");
            }
        }
        Ok(output) => warn!(
            "automatic RKP renewal failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => warn!("automatic RKP renewal could not run: {error}"),
    }
}
