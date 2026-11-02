//! Periodic trigger for the automated RKP renewal.
//!
//! The decision logic lives in the `ommega-rkp auto-run` subcommand, which owns
//! the seed, the renewal window and the issuance lock. This thread only starts a
//! child process and then sleeps to the instant that child reports as the next
//! renewal deadline, so nothing runs while the chain is still fresh: no polling,
//! one wake per renewal window (bounded to a day so a state change made in the
//! WebUI is picked up without waiting for a distant deadline).
//!
//! The engine, the HTTP client and the keybox writer all run in the child, so a
//! slow or failing request can never hold the TA. The child applies its own
//! backoff so a broken registration cannot burn a request on every wake.

use std::{path::Path, process::Command, thread, time::Duration};

use log::{debug, info, warn};

const CLIS: [&str; 2] = [
    "/data/adb/modules/ommega/libs/arm64-v8a/ommega-rkp",
    "/data/adb/modules/ommega/libs/x86_64/ommega-rkp",
];
/// Wait for the device to settle before the first check.
const FIRST_DELAY: Duration = Duration::from_secs(300);
/// Bounds for the sleep requested by the tool: never spin, never oversleep a day.
const MIN_SLEEP: Duration = Duration::from_secs(120);
const MAX_SLEEP: Duration = Duration::from_secs(24 * 60 * 60);
const DEFAULT_SLEEP: Duration = Duration::from_secs(24 * 60 * 60);

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
                let delay = run_once(&cli);
                debug!(
                    "next automatic RKP renewal check in {} minutes",
                    delay.as_secs() / 60
                );
                thread::sleep(delay);
            }
        });
    if let Err(error) = spawned {
        warn!("could not start the automatic RKP renewal thread: {error}");
    } else {
        info!("automatic RKP renewal sleeps to the next renewal deadline");
    }
}

/// Runs one check and returns how long to wait before the next one.
fn run_once(cli: &Path) -> Duration {
    let output = match Command::new(cli).arg("auto-run").output() {
        Ok(output) => output,
        Err(error) => {
            warn!("automatic RKP renewal could not run: {error}");
            return DEFAULT_SLEEP;
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = stdout.trim();
    if !output.status.success() {
        warn!(
            "automatic RKP renewal failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return DEFAULT_SLEEP;
    }
    if text.contains("\"ran\":true") {
        info!("automatic RKP renewal: {text}");
    } else {
        debug!("automatic RKP renewal: {text}");
    }
    requested_sleep(text, now_ms())
}

/// The tool answers with `next_wake_ms`; anything unreadable falls back to a day.
fn requested_sleep(report: &str, now: u64) -> Duration {
    let wake = serde_json::from_str::<serde_json::Value>(report)
        .ok()
        .and_then(|value| {
            value
                .get("next_wake_ms")
                .and_then(serde_json::Value::as_u64)
        });
    let Some(wake) = wake else {
        return DEFAULT_SLEEP;
    };
    Duration::from_millis(wake.saturating_sub(now)).clamp(MIN_SLEEP, MAX_SLEEP)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::requested_sleep;
    use std::time::Duration;

    #[test]
    fn sleeps_exactly_to_the_reported_deadline() {
        let now = 1_000_000u64;
        let report =
            r#"{"skipped":"not-due","next_wake_ms":1000000}"#.replace("1000000}", "4600000}");
        assert_eq!(requested_sleep(&report, now), Duration::from_secs(3600));
    }

    #[test]
    fn clamps_short_and_far_deadlines() {
        assert_eq!(
            requested_sleep(r#"{"next_wake_ms":1000010}"#, 1_000_000),
            Duration::from_secs(120)
        );
        assert_eq!(
            requested_sleep(r#"{"next_wake_ms":999999999999}"#, 1_000_000),
            Duration::from_secs(24 * 60 * 60)
        );
    }

    #[test]
    fn unreadable_reports_fall_back_to_a_day() {
        assert_eq!(
            requested_sleep("not json", 0),
            Duration::from_secs(24 * 60 * 60)
        );
    }
}
