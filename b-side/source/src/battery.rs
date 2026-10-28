//! Read-only primary-battery telemetry for the existing relay poll heartbeat.
//!
//! Only the standard Android battery sysfs attributes are read. Unsupported,
//! denied, absent or malformed attributes stay unknown; no Binder/HAL/TEE,
//! shell command, property override or persistent state is involved.

use std::io;
use std::time::{Duration, Instant};

const SYSFS_BATTERY: &str = "/sys/class/power_supply/battery";
/// Avoid reading sysfs at the relay's 80ms short-poll cadence. Each report is
/// sampled at most five seconds before the poll; failed refreshes clear it.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Reading {
    percent: Option<u8>,
    charging: Option<bool>,
}

fn parse_percent(raw: &str) -> Option<u8> {
    let raw = raw.trim();
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    raw.parse::<u8>().ok().filter(|percent| *percent <= 100)
}

fn parse_charging(raw: &str) -> Option<bool> {
    match raw.trim() {
        "Charging" => Some(true),
        // This reports active charging, not merely external power. A full
        // battery is not actively charging; unknown is never treated as false.
        "Discharging" | "Not charging" | "Full" => Some(false),
        _ => None,
    }
}

impl Reading {
    fn read_with(mut read: impl FnMut(&str) -> io::Result<String>) -> Self {
        if read("present").is_ok_and(|raw| raw.trim() == "0") {
            return Self::default();
        }
        Self {
            percent: read("capacity").ok().and_then(|raw| parse_percent(&raw)),
            charging: read("status").ok().and_then(|raw| parse_charging(&raw)),
        }
    }

    fn append_query(self, url: &mut String) {
        use std::fmt::Write as _;
        if let Some(percent) = self.percent {
            let _ = write!(url, "&battery_percent={percent}");
        }
        if let Some(charging) = self.charging {
            let _ = write!(url, "&battery_charging={charging}");
        }
    }
}

/// In-memory, best-effort telemetry for one relay poll loop.
#[derive(Default)]
pub struct BatteryTelemetry {
    sampled_at: Option<Instant>,
    reading: Reading,
}

impl BatteryTelemetry {
    /// Append only supported values to an existing poll URL. An older server
    /// ignores these optional query fields; an unreadable sample adds nothing.
    pub fn append_query(&mut self, url: &mut String) {
        self.refresh_with(Instant::now(), || {
            Reading::read_with(|attribute| {
                std::fs::read_to_string(format!("{SYSFS_BATTERY}/{attribute}"))
            })
        });
        self.reading.append_query(url);
    }

    fn refresh_with(&mut self, now: Instant, read: impl FnOnce() -> Reading) {
        if self
            .sampled_at
            .is_some_and(|last| now.saturating_duration_since(last) < SAMPLE_INTERVAL)
        {
            return;
        }
        // Replace rather than retain the last good reading on failure, so
        // subsequent heartbeats cannot keep an unsupported value looking fresh.
        self.reading = read();
        self.sampled_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_preserves_empty_and_full_without_fabricating_values() {
        assert_eq!(parse_percent("0\n"), Some(0));
        assert_eq!(parse_percent(" 100\n"), Some(100));
        assert_eq!(parse_percent("42"), Some(42));
        for raw in [
            "",
            " ",
            "-1",
            "+1",
            "101",
            "256",
            "999999999999",
            "3.5",
            "42%",
            "NaN",
        ] {
            assert_eq!(parse_percent(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn charging_only_accepts_known_sysfs_states() {
        assert_eq!(parse_charging("Charging\n"), Some(true));
        for raw in ["Discharging", "Not charging", "Full"] {
            assert_eq!(parse_charging(raw), Some(false));
        }
        for raw in ["", "Unknown", "charging", "1", "online"] {
            assert_eq!(parse_charging(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn mock_sysfs_keeps_attributes_independent_and_absent_battery_unknown() {
        let reading = Reading::read_with(|attribute| match attribute {
            "capacity" => Ok("0\n".into()),
            _ => Err(io::ErrorKind::PermissionDenied.into()),
        });
        assert_eq!(
            reading,
            Reading {
                percent: Some(0),
                charging: None
            }
        );
        let reading = Reading::read_with(|attribute| match attribute {
            "capacity" => Ok("101".into()),
            "status" => Ok("Charging".into()),
            _ => Err(io::ErrorKind::NotFound.into()),
        });
        assert_eq!(
            reading,
            Reading {
                percent: None,
                charging: Some(true)
            }
        );
        assert_eq!(
            Reading::read_with(|_| Err(io::ErrorKind::NotFound.into())),
            Reading::default()
        );
        assert_eq!(
            Reading::read_with(|attribute| match attribute {
                "present" => Ok("0\n".into()),
                _ => panic!("an absent battery must not be sampled"),
            }),
            Reading::default()
        );
    }

    #[test]
    fn query_omits_unknowns_but_includes_zero_and_false() {
        let mut url = "https://relay.invalid/api/b/poll/?device_id=mock".to_string();
        Reading::default().append_query(&mut url);
        assert!(!url.contains("battery_"));
        Reading {
            percent: Some(0),
            charging: Some(false),
        }
        .append_query(&mut url);
        assert!(url.ends_with("&battery_percent=0&battery_charging=false"));
    }

    #[test]
    fn short_polls_reuse_sample_but_a_failed_refresh_clears_it() {
        let now = Instant::now();
        let mut telemetry = BatteryTelemetry::default();
        let reading = Reading {
            percent: Some(100),
            charging: Some(true),
        };
        telemetry.refresh_with(now, || reading);
        telemetry.refresh_with(now + SAMPLE_INTERVAL - Duration::from_millis(1), || {
            panic!("short polling must not re-read sysfs")
        });
        assert_eq!(telemetry.reading, reading);
        telemetry.refresh_with(now + SAMPLE_INTERVAL, Reading::default);
        assert_eq!(telemetry.reading, Reading::default());
        let mut query = String::new();
        telemetry.reading.append_query(&mut query);
        assert!(query.is_empty());
    }
}
