// Copyright 2026, The Android Open Source Project
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Relay verdict state and best-effort publication for status reporting.

use std::{
    sync::RwLock,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;

static REMOTE_HEALTH: RwLock<Option<RemoteHealth>> = RwLock::new(None);

/// A snapshot of relay reachability for operator-facing status.
#[derive(Debug, Clone, Default)]
pub struct RemoteHealth {
    pub last_ok_unix: Option<i64>,
    pub last_error_unix: Option<i64>,
    pub last_status: Option<u16>,
    pub last_kind: Option<&'static str>,
    pub last_message: Option<String>,
    /// The verdict of the most recent request: "ok" or "error". Both clocks
    /// have one-second resolution, so a failure and a success inside the same
    /// second cannot be ordered by time. This records which one came last.
    pub last_verdict: Option<&'static str>,
}

const VERDICT_OK: &str = "ok";
const VERDICT_ERROR: &str = "error";

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether a successful request should rewrite the published snapshot.
///
/// The WebUI reads the file, so it must not keep reporting an outage that is
/// over, and it must not inherit one from an earlier process: the first success
/// after a restart publishes a clean snapshot. Every other success is silent —
/// a healthy relay would otherwise rewrite the file on every request.
///
/// The verdict of the last request decides this, not the two clocks. A failure
/// and a success inside the same second tie them, and a tie used to freeze the
/// error in the published file for good: every later success then looked like
/// a non-event and the panel kept reporting a failure that was long over.
fn ok_snapshot_is_news(health: &RemoteHealth) -> bool {
    health.last_verdict != Some(VERDICT_OK)
}

pub(super) fn record_remote_ok() {
    let publish = if let Ok(mut guard) = REMOTE_HEALTH.write() {
        let health = guard.get_or_insert_with(RemoteHealth::default);
        let publish = ok_snapshot_is_news(health);
        health.last_ok_unix = Some(now_unix());
        health.last_verdict = Some(VERDICT_OK);
        publish
    } else {
        false
    };
    if publish {
        write_health_snapshot();
    }
}

fn set_attestation_failure(health: &mut RemoteHealth, now: i64) -> bool {
    // Preserve a more specific transport or server error when a retry fails.
    if health.last_verdict == Some(VERDICT_ERROR) {
        return false;
    }
    health.last_error_unix = Some(now);
    health.last_status = None;
    health.last_kind = Some("attestation");
    health.last_message =
        Some("Remote attestation response could not be validated or saved".into());
    health.last_verdict = Some(VERDICT_ERROR);
    true
}

pub(super) fn record_attestation_failure() {
    let publish = if let Ok(mut guard) = REMOTE_HEALTH.write() {
        let health = guard.get_or_insert_with(RemoteHealth::default);
        set_attestation_failure(health, now_unix())
    } else {
        false
    };
    if publish {
        write_health_snapshot();
    }
}

pub(super) fn record_remote_failure(status: Option<u16>, kind: &'static str, message: &str) {
    if let Ok(mut guard) = REMOTE_HEALTH.write() {
        let health = guard.get_or_insert_with(RemoteHealth::default);
        health.last_error_unix = Some(now_unix());
        health.last_status = status;
        health.last_kind = Some(kind);
        health.last_message = Some(message.chars().take(200).collect());
        health.last_verdict = Some(VERDICT_ERROR);
    }
    write_health_snapshot();
}

/// Current relay health, for status reporting.
pub fn remote_health() -> Option<RemoteHealth> {
    REMOTE_HEALTH.read().ok().and_then(|g| g.clone())
}

/// Path the daemon publishes health to, so the separate WebUI process can show
/// why remote attestation is failing without guessing.
pub const REMOTE_HEALTH_PATH: &str = "/data/misc/keystore/ommega/remote-health.json";

/// Publish the current health snapshot for the WebUI. Best-effort: a failure
/// here must never disturb an attestation path.
fn write_health_snapshot() {
    let Some(health) = remote_health() else {
        return;
    };
    let payload = json!({
        "last_ok_unix": health.last_ok_unix,
        "last_error_unix": health.last_error_unix,
        "last_status": health.last_status,
        "last_kind": health.last_kind,
        "last_message": health.last_message,
        "last_verdict": health.last_verdict,
    });
    if let Some(parent) = std::path::Path::new(REMOTE_HEALTH_PATH).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(REMOTE_HEALTH_PATH, payload.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_attestation_is_a_failure_without_hiding_a_server_error() {
        let mut health = RemoteHealth {
            last_ok_unix: Some(10),
            last_verdict: Some(VERDICT_OK),
            ..RemoteHealth::default()
        };
        assert!(set_attestation_failure(&mut health, 11));
        assert_eq!(health.last_error_unix, Some(11));
        assert_eq!(health.last_kind, Some("attestation"));
        assert_eq!(health.last_verdict, Some(VERDICT_ERROR));

        health.last_status = Some(401);
        health.last_kind = Some("unauthorized");
        assert!(!set_attestation_failure(&mut health, 12));
        assert_eq!(health.last_status, Some(401));
        assert_eq!(health.last_kind, Some("unauthorized"));
    }

    #[test]
    fn health_records_the_last_server_verdict() {
        record_remote_failure(
            Some(401),
            "unauthorized",
            "missing or invalid X-Relay-Token",
        );
        let health = remote_health().expect("health should be recorded");
        assert_eq!(health.last_status, Some(401));
        assert_eq!(health.last_kind, Some("unauthorized"));
        assert_eq!(health.last_verdict, Some(VERDICT_ERROR));
        assert!(health
            .last_message
            .as_deref()
            .unwrap_or_default()
            .contains("X-Relay-Token"));
        assert!(health.last_error_unix.is_some());

        record_remote_ok();
        let health = remote_health().unwrap();
        assert!(health.last_ok_unix.is_some());
        // The verdict follows the last request, while the record of what went
        // wrong stays available for diagnostics.
        assert_eq!(health.last_verdict, Some(VERDICT_OK));
        // A later success must not erase the record of what went wrong.
        assert_eq!(health.last_status, Some(401));
    }

    #[test]
    fn a_recovery_publishes_a_clean_snapshot_but_stays_quiet_after_that() {
        // A fresh process: the first success has to be published, otherwise the
        // WebUI keeps showing an outage inherited from the previous run.
        let empty = RemoteHealth::default();
        assert!(ok_snapshot_is_news(&empty));

        // Failure then success: the recovery is news, because the panel is
        // still claiming the relay is down.
        let after_failure = RemoteHealth {
            last_ok_unix: None,
            last_error_unix: Some(1_000),
            last_verdict: Some(VERDICT_ERROR),
            ..RemoteHealth::default()
        };
        assert!(ok_snapshot_is_news(&after_failure));

        // Already healthy: a success is not news, and rewriting the file on
        // every request would be pure write amplification.
        let healthy = RemoteHealth {
            last_ok_unix: Some(2_000),
            last_error_unix: Some(1_000),
            last_verdict: Some(VERDICT_OK),
            ..RemoteHealth::default()
        };
        assert!(!ok_snapshot_is_news(&healthy));

        // Success before any failure: the snapshot is already accurate.
        let never_failed = RemoteHealth {
            last_ok_unix: Some(2_000),
            last_error_unix: None,
            last_verdict: Some(VERDICT_OK),
            ..RemoteHealth::default()
        };
        assert!(!ok_snapshot_is_news(&never_failed));

        // Interleaved: a failure newer than the last success is still pending,
        // so the next success must clear it.
        let failure_is_newer = RemoteHealth {
            last_ok_unix: Some(1_000),
            last_error_unix: Some(2_000),
            last_verdict: Some(VERDICT_ERROR),
            ..RemoteHealth::default()
        };
        assert!(ok_snapshot_is_news(&failure_is_newer));

        // Same second: the clocks tie, so only the verdict can order the two
        // events. A stale error must not survive the next good request.
        let tied = RemoteHealth {
            last_ok_unix: Some(1_000),
            last_error_unix: Some(1_000),
            last_verdict: Some(VERDICT_ERROR),
            ..RemoteHealth::default()
        };
        assert!(ok_snapshot_is_news(&tied));
        let recovered = RemoteHealth {
            last_ok_unix: Some(1_300),
            last_error_unix: Some(1_000),
            last_verdict: Some(VERDICT_OK),
            ..RemoteHealth::default()
        };
        assert!(!ok_snapshot_is_news(&recovered));
    }
}
