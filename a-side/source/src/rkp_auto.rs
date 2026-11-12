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

use std::{io, path::Path, process::Command, thread, time::Duration};

use log::{debug, info, warn};

const CLIS: [&str; 2] = [
    "/data/adb/modules/ommega/libs/arm64-v8a/ommega-rkp",
    "/data/adb/modules/ommega/libs/x86_64/ommega-rkp",
];
/// Wait for the device to settle before the first check.
const FIRST_DELAY: Duration = Duration::from_secs(300);
/// Bounds for the deadline requested by the tool: never spin, never oversleep a day.
const MIN_SLEEP_MS: u64 = 120_000;
const MAX_SLEEP_MS: u64 = 24 * 60 * 60 * 1000;

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
            // Keep the startup settling delay independent of boot-time clock fixes.
            thread::sleep(FIRST_DELAY);
            loop {
                let wake_ms = run_once(&cli);
                debug!(
                    "next automatic RKP renewal check in {} minutes",
                    wake_ms.saturating_sub(now_ms()) / 60_000
                );
                if let Err(error) = sleep_until(wake_ms) {
                    warn!("automatic RKP renewal deadline wait failed: {error}");
                    // Unexpected kernel errors must not turn this into a busy loop.
                    thread::sleep(Duration::from_millis(MIN_SLEEP_MS));
                }
            }
        });
    if let Err(error) = spawned {
        warn!("could not start the automatic RKP renewal thread: {error}");
    } else {
        info!("automatic RKP renewal sleeps to the next renewal deadline");
    }
}

/// Runs one check and returns the next absolute wall-clock deadline.
fn run_once(cli: &Path) -> u64 {
    let output = match Command::new(cli).arg("auto-run").output() {
        Ok(output) => output,
        Err(error) => {
            warn!("automatic RKP renewal could not run: {error}");
            return now_ms().saturating_add(MAX_SLEEP_MS);
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = stdout.trim();
    if !output.status.success() {
        warn!(
            "automatic RKP renewal failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return now_ms().saturating_add(MAX_SLEEP_MS);
    }
    if text.contains("\"ran\":true") {
        info!("automatic RKP renewal: {text}");
    } else {
        debug!("automatic RKP renewal: {text}");
    }
    requested_wake_ms(text, now_ms())
}

/// The tool answers with `next_wake_ms`; anything unreadable falls back to a day.
fn requested_wake_ms(report: &str, now: u64) -> u64 {
    let wake = serde_json::from_str::<serde_json::Value>(report)
        .ok()
        .and_then(|value| {
            value
                .get("next_wake_ms")
                .and_then(serde_json::Value::as_u64)
        });
    wake.unwrap_or_else(|| now.saturating_add(MAX_SLEEP_MS))
        .clamp(
            now.saturating_add(MIN_SLEEP_MS),
            now.saturating_add(MAX_SLEEP_MS),
        )
}

/// Certificate expiry is wall-clock time. A relative thread::sleep uses a clock
/// that stops during Android deep sleep, delaying renewal by every suspended
/// hour. An absolute CLOCK_REALTIME wait includes suspend and clock corrections.
/// It does not wake the device: an elapsed deadline is handled on resume.
fn sleep_until(wake_ms: u64) -> io::Result<()> {
    sleep_until_with(wake_ms, |clock, flags, deadline| unsafe {
        libc::clock_nanosleep(clock, flags, deadline, std::ptr::null_mut())
    })
}

fn sleep_until_with(
    wake_ms: u64,
    mut wait: impl FnMut(libc::clockid_t, libc::c_int, &libc::timespec) -> libc::c_int,
) -> io::Result<()> {
    let deadline = libc::timespec {
        tv_sec: (wake_ms / 1000) as libc::time_t,
        tv_nsec: ((wake_ms % 1000) * 1_000_000) as libc::c_long,
    };
    loop {
        // Unlike nanosleep, clock_nanosleep returns an error number directly.
        match wait(libc::CLOCK_REALTIME, libc::TIMER_ABSTIME, &deadline) {
            0 => return Ok(()),
            // Keep the original absolute deadline across signals, not a new delay.
            libc::EINTR => continue,
            error => return Err(io::Error::from_raw_os_error(error)),
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{requested_wake_ms, sleep_until_with, MAX_SLEEP_MS, MIN_SLEEP_MS};

    #[test]
    fn preserves_the_reported_wall_clock_deadline() {
        let now = 1_000_000;
        let report = r#"{"skipped":"not-due","next_wake_ms":4600000}"#;
        assert_eq!(requested_wake_ms(report, now), 4_600_000);
    }

    #[test]
    fn clamps_elapsed_short_and_far_deadlines() {
        let now = 1_000_000;
        for wake in [0, now - 1, now, now + 10] {
            let report = format!(r#"{{"next_wake_ms":{wake}}}"#);
            assert_eq!(requested_wake_ms(&report, now), now + MIN_SLEEP_MS);
        }
        assert_eq!(
            requested_wake_ms(r#"{"next_wake_ms":999999999999}"#, now),
            now + MAX_SLEEP_MS
        );
    }

    #[test]
    fn unreadable_reports_fall_back_to_a_day() {
        let now = 1_000_000;
        for report in [
            "not json",
            "{}",
            r#"{"next_wake_ms":null}"#,
            r#"{"next_wake_ms":-1}"#,
        ] {
            assert_eq!(requested_wake_ms(report, now), now + MAX_SLEEP_MS);
        }
    }

    #[test]
    fn deadline_bounds_do_not_overflow() {
        assert_eq!(requested_wake_ms("{}", u64::MAX - 1), u64::MAX);
    }

    #[test]
    fn absolute_wait_uses_suspend_inclusive_wall_time() {
        // A realistic certificate deadline, retaining milliseconds. After Android has
        // suspended past it, REALTIME is already due even if awake time is not.
        let wake_ms = 1_794_499_808_123;
        let mut calls = 0;
        sleep_until_with(wake_ms, |clock, flags, deadline| {
            calls += 1;
            assert_eq!(clock, libc::CLOCK_REALTIME);
            assert_eq!(flags, libc::TIMER_ABSTIME);
            assert_eq!(deadline.tv_sec, 1_794_499_808);
            assert_eq!(deadline.tv_nsec, 123_000_000);
            0
        })
        .unwrap();
        assert_eq!(calls, 1);
    }

    #[test]
    fn interrupted_wait_keeps_the_original_absolute_deadline() {
        let mut calls = 0;
        sleep_until_with(4_600_123, |clock, flags, deadline| {
            calls += 1;
            assert_eq!(clock, libc::CLOCK_REALTIME);
            assert_eq!(flags, libc::TIMER_ABSTIME);
            assert_eq!(deadline.tv_sec, 4_600);
            assert_eq!(deadline.tv_nsec, 123_000_000);
            if calls < 3 {
                libc::EINTR
            } else {
                0
            }
        })
        .unwrap();
        assert_eq!(calls, 3);
    }

    #[test]
    fn unexpected_wait_error_returns_without_spinning() {
        let mut calls = 0;
        let error = sleep_until_with(4_600_000, |_, _, _| {
            calls += 1;
            libc::EINVAL
        })
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EINVAL));
        assert_eq!(calls, 1);
    }
}
