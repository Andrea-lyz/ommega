//! relay daemon for ommegaclient-b.
//!
//! This binary is the "new B-side" agent that talks to the existing
//! relay_server (ommega-old) using its B-side protocol:
//!
//!   * `GET  /api/b/poll/?device_id=..&machine_id=..&timeout=N` (X-Relay-Token)
//!     -> 200: {task_id, task_type, payload, target_device_id}
//!     -> 204: no task (long poll timed out)
//!   * `POST /api/b/result/` body {task_id, result, device_id}
//!     -> {status: ok}
//!
//! When an `attest` task arrives, the A-side supplies (inside `payload`):
//!   * `challenge`                    : base64, the real-time attestation nonce
//!   * `attestation_application_id`  : base64, DER-encoded `AttestationApplicationId`
//!     (this is the "appid / tag 709" the A-side wants)
//!
//! The relay daemon mints a certificate chain via the *real* on-device
//! hardware TEE (see `ommegaclient_b::keymaster::attest_proxy`) with that appid
//! embedded, then uploads `{cert_chain: [base64, ...]}` back to the server,
//! which forwards it to the A-side.
//!
//! Configuration is read from the config file `/data/adb/ommega/relay.conf`
//! (KEY=VALUE lines), falling back to environment variables:
//!   OMMEGA_RELAY_SERVER      base URL, e.g. https://example.com:8443
//!   OMMEGA_RELAY_DEVICE_ID   device id (required)
//!   OMMEGA_RELAY_MACHINE_ID  machine id (optional)
//!   OMMEGA_RELAY_TOKEN       relay B-side token (X-Relay-Token)
//!   OMMEGA_RELAY_LOG_ENABLED   file log on/off (default true)
//!   OMMEGA_RELAY_LOG_LEVEL     file log level: off|error|warn|info|debug|trace (default debug)
//!   OMMEGA_RELAY_LOGCAT_ENABLED logcat on/off (default true)
//!   OMMEGA_RELAY_LOGCAT_LEVEL   logcat level: off|error|warn|info|debug|trace (default info)
//!
//! Logging is read *before* the rest of the config is validated, so a broken
//! `relay.conf` still honours its log settings while reporting the error.
//!
//! The config is hot-reloaded at runtime: a background thread watches
//! `relay.conf` for modification and the `restart.all` marker, and updates the
//! live config in place (no process restart needed). The service
//! (`template/service.sh`) starts the relay directly (killing stale instances
//! first); there is no daemon wrapper.
//!
//! Both `http://` and `https://` are supported. The relay_server runs over
//! HTTPS with a self-signed certificate, so any server certificate is accepted.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};
use x509_cert::der::Decode as _;

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use kmr_wire::keymint::{
    DateTime, Digest as KmDigest, EcCurve as KmEcCurve, ErrorCode as KmErrorCode,
    KeyPurpose as KmKeyPurpose, PaddingMode as KmPadding,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};

use ommegaclient_b::keymaster::attest_proxy::{check_app_id_der, SYSTEM_KEYMINT_STRONGBOX};
use ommegaclient_b::keymaster::relay_tee::keymint_error;
use ommegaclient_b::keymaster::tee_ops::{self, KeyAlgorithm, KeySpec};

const POLL_TIMEOUT_SEC: u32 = 20;
const TEE_WORKERS: usize = 3;
const CONNECT_TIMEOUT_MS: u64 = 3000;
const READ_TIMEOUT_MS: u64 = 30_000;
const CONF_PATH: &str = "/data/adb/ommega/relay.conf";
const STATE_DIR: &str = "/data/adb/ommega";
const INSTANCE_LOCK_PATH: &str = "/data/adb/ommega/relay.lock";
const RESTART_MARKER: &str = "/data/adb/ommega/restart.all";
const RELOAD_POLL_MS: u64 = 1000;
/// Backoff floor and ceiling after a failed poll.
///
/// The old loop retried every poll failure after a flat 1s. Against an HTTP 401
/// that burns the server's per-address failed-auth budget (40/hour) in under a
/// minute, after which the server answers "too many invalid requests" and the
/// device is locked out for the rest of the window — turning a brief server-side
/// outage into an hour of downtime. Rejections now back off.
const POLL_BACKOFF_MIN_MS: u64 = 1000;
const POLL_BACKOFF_MAX_MS: u64 = 15_000;
/// A rejected credential is not going to be accepted a second later. The
/// ceiling is chosen so that steady-state retries stay well under the server's
/// 40-per-hour invalid-request budget: at 5 minutes apart that is 12 an hour,
/// so a wrong or expired token can never lock the address out by itself.
/// Editing the config resets this, so a corrected token is picked up at once.
const AUTH_BACKOFF_MIN_MS: u64 = 5_000;
const AUTH_BACKOFF_MAX_MS: u64 = 300_000;
/// Fallback when a throttling response carries no `Retry-After`.
const THROTTLE_BACKOFF_MS: u64 = 30_000;
const MODULE_PROP: &str = "/data/adb/modules/ommegaclient_b/module.prop";

fn acquire_instance_lock() -> Result<File> {
    std::fs::create_dir_all(STATE_DIR).context("create relay state directory")?;
    std::fs::set_permissions(STATE_DIR, std::fs::Permissions::from_mode(0o700))
        .context("chmod relay state directory")?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(INSTANCE_LOCK_PATH)
        .context("open relay instance lock")?;
    std::fs::set_permissions(INSTANCE_LOCK_PATH, std::fs::Permissions::from_mode(0o600))
        .context("chmod relay instance lock")?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error()).context("another relay instance is running");
    }
    file.set_len(0).context("truncate relay instance lock")?;
    writeln!(file, "{}", std::process::id()).context("write relay instance pid")?;
    Ok(file)
}

/// Keep the KernelSU/Magisk module status (module.prop description) in sync
/// with the relay's real state. Best effort: failures are silently ignored
/// (e.g. module dir absent when running from a manual copy).
fn update_module_status(status: &str) {
    let Ok(contents) = std::fs::read_to_string(MODULE_PROP) else {
        return;
    };
    let mut out = String::new();
    let mut changed = false;
    for line in contents.lines() {
        if line.starts_with("description=") {
            out.push_str("description=");
            out.push_str(status);
            out.push('\n');
            changed = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if changed {
        let _ = std::fs::write(MODULE_PROP, out);
    }
}

#[derive(Clone, Debug)]
struct RelayConfig {
    server: String,
    device_id: String,
    machine_id: String,
    token: String,
}

impl RelayConfig {
    fn validate(&self) -> Result<()> {
        if self.server.is_empty() {
            return Err(anyhow!("OMMEGA_RELAY_SERVER is empty"));
        }
        if self.device_id.is_empty() {
            return Err(anyhow!("OMMEGA_RELAY_DEVICE_ID is empty"));
        }
        if self.token.is_empty() {
            return Err(anyhow!("OMMEGA_RELAY_TOKEN is empty"));
        }
        Ok(())
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.trim().is_empty())
}

/// File mtime (seconds) if present, else None.
fn file_mtime(path: &str) -> Option<u64> {
    let md = std::fs::metadata(path).ok()?;
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

/// Load config from `/data/adb/ommega/relay.conf` (KEY=VALUE lines).
fn load_config_from_file() -> Result<RelayConfig> {
    let raw = std::fs::read_to_string(CONF_PATH).with_context(|| format!("read {CONF_PATH}"))?;
    let mut m: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim().to_string();
            if !k.is_empty() {
                m.insert(k, v);
            }
        }
    }
    let server = m
        .get("OMMEGA_RELAY_SERVER")
        .cloned()
        .context("OMMEGA_RELAY_SERVER missing in relay.conf")?;
    let device_id = m
        .get("OMMEGA_RELAY_DEVICE_ID")
        .cloned()
        .context("OMMEGA_RELAY_DEVICE_ID missing in relay.conf")?;
    let token = m
        .get("OMMEGA_RELAY_TOKEN")
        .cloned()
        .context("OMMEGA_RELAY_TOKEN missing in relay.conf")?;
    let machine_id = m
        .get("OMMEGA_RELAY_MACHINE_ID")
        .cloned()
        .unwrap_or_default();
    let server = server.trim_end_matches('/').to_string();
    Ok(RelayConfig {
        server,
        device_id,
        machine_id,
        token,
    })
}

fn parse_log_level(v: &str) -> Option<log::LevelFilter> {
    Some(match v.trim().to_ascii_lowercase().as_str() {
        "off" => log::LevelFilter::Off,
        "error" => log::LevelFilter::Error,
        "warn" | "warning" => log::LevelFilter::Warn,
        "info" => log::LevelFilter::Info,
        "debug" => log::LevelFilter::Debug,
        "trace" => log::LevelFilter::Trace,
        _ => return None,
    })
}

/// Extract the `OMMEGA_RELAY_LOG_*` and `OMMEGA_RELAY_LOGCAT_*` keys from raw
/// relay.conf content. Absent keys fall back to the defaults (file log on
/// debug, logcat on info) so an existing relay.conf without them keeps its
/// previous behaviour.
fn parse_log_config(raw: &str) -> (bool, log::LevelFilter, bool, log::LevelFilter) {
    let mut file_enabled = true;
    let mut file_level = log::LevelFilter::Debug;
    let mut logcat_enabled = true;
    let mut logcat_level = log::LevelFilter::Info;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim();
            if k == "OMMEGA_RELAY_LOG_ENABLED" {
                file_enabled = v.eq_ignore_ascii_case("true") || v == "1";
            } else if k == "OMMEGA_RELAY_LOG_LEVEL" {
                if let Some(lv) = parse_log_level(v) {
                    file_level = lv;
                }
            } else if k == "OMMEGA_RELAY_LOGCAT_ENABLED" {
                logcat_enabled = v.eq_ignore_ascii_case("true") || v == "1";
            } else if k == "OMMEGA_RELAY_LOGCAT_LEVEL" {
                if let Some(lv) = parse_log_level(v) {
                    logcat_level = lv;
                }
            }
        }
    }
    (file_enabled, file_level, logcat_enabled, logcat_level)
}

/// Read the logging switches *before* the full RelayConfig is loaded/validated,
/// so a broken relay.conf still honours its log settings while reporting the
/// error. Order: relay.conf -> environment -> defaults (file log on debug,
/// logcat on info).
fn preload_log_config() -> (bool, log::LevelFilter, bool, log::LevelFilter) {
    if let Ok(raw) = std::fs::read_to_string(CONF_PATH) {
        return parse_log_config(&raw);
    }
    let file_enabled = env("OMMEGA_RELAY_LOG_ENABLED")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(true);
    let file_level = env("OMMEGA_RELAY_LOG_LEVEL")
        .and_then(|v| parse_log_level(&v))
        .unwrap_or(log::LevelFilter::Debug);
    let logcat_enabled = env("OMMEGA_RELAY_LOGCAT_ENABLED")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(true);
    let logcat_level = env("OMMEGA_RELAY_LOGCAT_LEVEL")
        .and_then(|v| parse_log_level(&v))
        .unwrap_or(log::LevelFilter::Info);
    (file_enabled, file_level, logcat_enabled, logcat_level)
}

/// Prefer the config file; fall back to environment variables (the wrapper may
/// supply them directly). Returns the chosen source for logging.
fn load_config() -> Result<(RelayConfig, &'static str)> {
    if let Ok(cfg) = load_config_from_file() {
        cfg.validate()?;
        return Ok((cfg, "file"));
    }
    let server = env("OMMEGA_RELAY_SERVER")
        .context("OMMEGA_RELAY_SERVER not set and relay.conf unreadable")?;
    let device_id = env("OMMEGA_RELAY_DEVICE_ID")
        .context("OMMEGA_RELAY_DEVICE_ID not set and relay.conf unreadable")?;
    let token = env("OMMEGA_RELAY_TOKEN")
        .context("OMMEGA_RELAY_TOKEN not set and relay.conf unreadable")?;
    let machine_id = env("OMMEGA_RELAY_MACHINE_ID").unwrap_or_default();
    let server = server.trim_end_matches('/').to_string();
    let cfg = RelayConfig {
        server,
        device_id,
        machine_id,
        token,
    };
    cfg.validate()?;
    Ok((cfg, "env"))
}

// ---------------------------------------------------------------------------
// HTTP client (reqwest-based with rustls).
//
// Uses reqwest's blocking client with rustls TLS backend.  The relay_server
// uses a self-signed certificate, so certificate verification is disabled.
// reqwest provides built-in connection pooling, keep-alive, and chunked
// transfer encoding support — all things the old hand-rolled client lacked.
//
// The client is rebuilt on config hot-reload so that a server URL change
// does not leave stale pooled connections pointing at the old address.
// ---------------------------------------------------------------------------

/// Shared reqwest blocking client.  Wrapped in `RwLock<Option<Arc<...>>>` so
/// the config watcher can drop it (forcing a rebuild) without blocking
/// in-flight requests (the old `Arc` stays alive until its last user drops it).
static HTTP_CLIENT: RwLock<Option<Arc<Client>>> = RwLock::new(None);

fn build_http_client() -> Result<Client> {
    Client::builder()
        .danger_accept_invalid_certs(true)
        .connect_timeout(Duration::from_millis(CONNECT_TIMEOUT_MS))
        .timeout(Duration::from_millis(READ_TIMEOUT_MS))
        .build()
        .context("build reqwest client")
}

/// Returns the shared HTTP client, building it on first use.
fn get_http_client() -> Result<Arc<Client>> {
    // Fast path: read lock, return if present.
    if let Ok(guard) = HTTP_CLIENT.read() {
        if let Some(client) = guard.as_ref() {
            return Ok(client.clone());
        }
    }
    // Slow path: write lock, build if still absent.
    let mut guard = HTTP_CLIENT
        .write()
        .map_err(|_| anyhow!("HTTP client lock poisoned"))?;
    if let Some(client) = guard.as_ref() {
        return Ok(client.clone());
    }
    let client = Arc::new(build_http_client()?);
    *guard = Some(client.clone());
    Ok(client)
}

/// Drops the shared HTTP client so the next request rebuilds it.
/// Called from the config watcher when the server URL changes.
fn reset_http_client() {
    if let Ok(mut guard) = HTTP_CLIENT.write() {
        *guard = None;
        log::info!("HTTP client reset (connection pool cleared)");
    }
}

/// A completed HTTP exchange.
///
/// `retry_after` carries the server's own backoff hint, which the relay honours
/// instead of guessing; a throttled client that keeps retrying at a fixed
/// interval only deepens the throttle.
struct HttpResponse {
    status: u16,
    body: Vec<u8>,
    retry_after: Option<Duration>,
}

/// Parse a `Retry-After` header. Only the delta-seconds form is accepted; the
/// HTTP-date form is rare here and a bad parse must not become a zero wait.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let raw = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let secs: u64 = raw.trim().parse().ok()?;
    // Clamp: a hostile or broken value must not park the relay for hours.
    Some(Duration::from_secs(secs.clamp(1, 300)))
}

fn http_request(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<HttpResponse> {
    let client = get_http_client()?;
    let mut req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        other => client.request(
            reqwest::Method::from_bytes(other.as_bytes())
                .map_err(|_| anyhow!("invalid HTTP method: {other}"))?,
            url,
        ),
    };
    for (k, v) in headers {
        req = req.header(k, v);
    }
    if let Some(b) = body {
        req = req.body(b.to_vec());
    }
    let t0 = std::time::Instant::now();
    let resp = req
        .send()
        .with_context(|| format!("http {method} {url} failed"))?;
    let status = resp.status().as_u16();
    let retry_after = parse_retry_after(resp.headers());
    let bytes = resp
        .bytes()
        .with_context(|| format!("http {method} {url} read body failed"))?;
    let read_ms = t0.elapsed().as_millis();
    log::debug!(
        "http {} {} -> status={} {} bytes in {}ms, body_head: {:?}",
        method,
        url,
        status,
        bytes.len(),
        read_ms,
        String::from_utf8_lossy(&bytes[..bytes.len().min(120)])
    );
    Ok(HttpResponse {
        status,
        body: bytes.to_vec(),
        retry_after,
    })
}

// ---------------------------------------------------------------------------
// Relay protocol helpers.
// ---------------------------------------------------------------------------

/// How the poll loop should wait after a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollFailure {
    /// Network error, timeout, or a server-side 5xx: likely transient.
    Transient,
    /// The server rejected our credential (401/403). Retrying at speed only
    /// spends the failed-auth budget that then blocks the whole address.
    Rejected,
    /// The server asked us to slow down (429/503), with its own hint when given.
    Throttled(Option<Duration>),
}

struct PollError {
    error: anyhow::Error,
    failure: PollFailure,
}

impl PollError {
    fn transient(error: anyhow::Error) -> Self {
        Self {
            error,
            failure: PollFailure::Transient,
        }
    }
}

fn poll_tasks(
    cfg: &RelayConfig,
) -> std::result::Result<Option<(String, String, Value)>, PollError> {
    let url = format!(
        "{}/api/b/poll/?device_id={}&machine_id={}&timeout={}",
        cfg.server, cfg.device_id, cfg.machine_id, POLL_TIMEOUT_SEC
    );
    let headers = vec![("X-Relay-Token".to_string(), cfg.token.clone())];
    let response = http_request("GET", &url, &headers, None)
        .with_context(|| "b/poll failed")
        .map_err(PollError::transient)?;
    let HttpResponse {
        status,
        body,
        retry_after,
    } = response;
    log::debug!("b/poll status={status} body_len={}", body.len());
    match status {
        204 => Ok(None),
        200 => {
            let parse = |body: &[u8]| -> Result<(String, String, Value)> {
                let v: Value = serde_json::from_slice(body).with_context(|| "b/poll bad json")?;
                let task_id = v
                    .get("task_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("b/poll missing task_id"))?
                    .to_string();
                let task_type = v
                    .get("task_type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let payload = v.get("payload").cloned().unwrap_or(Value::Null);
                Ok((task_id, task_type, payload))
            };
            parse(&body).map(Some).map_err(PollError::transient)
        }
        other => Err(PollError {
            error: anyhow!("b/poll unexpected status {other}"),
            failure: poll_failure_for(other, retry_after),
        }),
    }
}

/// Classify a poll response status into a wait strategy.
fn poll_failure_for(status: u16, retry_after: Option<Duration>) -> PollFailure {
    match status {
        401 | 403 => PollFailure::Rejected,
        408 | 429 | 503 => PollFailure::Throttled(retry_after),
        _ => PollFailure::Transient,
    }
}

/// Next wait after a failed poll, given the failure kind and the current
/// transient/rejected backoff levels. Returns the wait and the updated levels.
fn next_poll_backoff(
    failure: PollFailure,
    transient_ms: u64,
    rejected_ms: u64,
) -> (Duration, u64, u64) {
    match failure {
        PollFailure::Transient => {
            let wait = transient_ms.clamp(POLL_BACKOFF_MIN_MS, POLL_BACKOFF_MAX_MS);
            (
                Duration::from_millis(wait),
                (wait * 2).min(POLL_BACKOFF_MAX_MS),
                rejected_ms,
            )
        }
        PollFailure::Rejected => {
            let wait = rejected_ms.clamp(AUTH_BACKOFF_MIN_MS, AUTH_BACKOFF_MAX_MS);
            (
                Duration::from_millis(wait),
                transient_ms,
                (wait * 2).min(AUTH_BACKOFF_MAX_MS),
            )
        }
        PollFailure::Throttled(hint) => {
            let wait = hint.unwrap_or(Duration::from_millis(THROTTLE_BACKOFF_MS));
            (wait, transient_ms, rejected_ms)
        }
    }
}

/// True when a result-upload status will not improve on retry.
///
/// 429 (rate limited) and 408 (request timeout) are excluded, and 5xx never
/// reaches here: the work is already done and the server is only asking us to
/// wait. Treating those as permanent threw away a finished TEE result, which
/// the A side then reported as an attestation failure even though nothing on
/// either device was actually wrong.
fn result_status_is_permanent(status: u16) -> bool {
    (400..500).contains(&status) && status != 429 && status != 408
}

/// POST the task result to the server, retrying transient failures
/// (network errors, 5xx, and throttling) with exponential backoff so a task is
/// not lost to a single glitch. Permanent 4xx rejections (bad token, unknown
/// task) are not retried.
fn post_result(cfg: &RelayConfig, task_id: &str, result: &Value) -> Result<()> {
    const MAX_ATTEMPTS: u32 = 6;
    let url = format!("{}/api/b/result/", cfg.server);
    let body = json!({
        "task_id": task_id,
        "result": result,
        "device_id": cfg.device_id,
    });
    let headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("X-Relay-Token".to_string(), cfg.token.clone()),
    ];
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        // The server's own `Retry-After`, when it sent one, wins over our
        // exponential guess.
        let retry_hint = match http_request(
            "POST",
            &url,
            &headers,
            Some(body.to_string().as_bytes()),
        ) {
            Ok(HttpResponse { status: 200, .. }) => {
                log::info!("b/result task={task_id} accepted (HTTP 200)");
                return Ok(());
            }
            Ok(HttpResponse {
                status,
                retry_after,
                ..
            }) => {
                if result_status_is_permanent(status) {
                    log::warn!("b/result task={task_id} rejected: HTTP {status}");
                    return Err(anyhow!("b/result rejected with HTTP {status}"));
                }
                log::warn!(
                    "b/result task={task_id} HTTP {status} (attempt {attempt}/{MAX_ATTEMPTS})"
                );
                last_err = Some(anyhow!("b/result unexpected status {status}"));
                retry_after
            }
            Err(e) => {
                log::warn!(
                    "b/result task={task_id} network error: {e:#} (attempt {attempt}/{MAX_ATTEMPTS})"
                );
                last_err = Some(e);
                None
            }
        };
        if attempt < MAX_ATTEMPTS {
            let backoff = retry_hint.unwrap_or(Duration::from_secs(1 << (attempt - 1)));
            std::thread::sleep(backoff);
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("b/result failed after {MAX_ATTEMPTS} attempts")))
}

// ---------------------------------------------------------------------------
// Task handlers.
// ---------------------------------------------------------------------------

fn b64_decode(v: &Value) -> Result<Vec<u8>> {
    let s = v
        .as_str()
        .ok_or_else(|| anyhow!("expected base64 string, got {v}"))?;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| anyhow!("bad base64: {e}"))
}

/// Attestation inputs pulled from a task payload:
/// `(attestation_application_id, challenge)`.
type AttestationContext = (Vec<u8>, Vec<u8>);

fn extract_attestation_context(payload: &Value) -> Result<AttestationContext> {
    // `attestation_application_id` may live at the top level or nested under
    // `device_attest_context` (the relay_server accepts both).
    let nested = payload.get("device_attest_context");
    let app_id = payload
        .get("attestation_application_id")
        .or_else(|| nested.and_then(|n| n.get("attestation_application_id")))
        .ok_or_else(|| anyhow!("payload missing attestation_application_id (tag 709)"))?;
    let challenge = payload
        .get("challenge")
        .ok_or_else(|| anyhow!("payload missing challenge"))?;

    let app_id_der = b64_decode(app_id).with_context(|| "decode attestation_application_id")?;
    let challenge = b64_decode(challenge).with_context(|| "decode challenge")?;
    Ok((app_id_der, challenge))
}

/// Parses the A-side requested key parameters from a task payload into a
/// [`tee_ops::KeySpec`]. Fields may live at the top level or nested under
/// `device_attest_context`; absent fields keep the KeySpec defaults (EC P-256,
/// SHA-256, etc.).
fn parse_key_spec(payload: &Value) -> Result<KeySpec> {
    let nested = payload.get("device_attest_context");
    let get =
        |k: &str| -> Option<&Value> { payload.get(k).or_else(|| nested.and_then(|n| n.get(k))) };
    let get_i64 = |k: &str| get(k).and_then(Value::as_i64);
    let get_arr = |k: &str| -> Vec<&Value> {
        get(k)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .collect()
    };

    // Algorithm family: prefer the explicit KeyMint `key_algorithm` int (raw
    // AIDL enum: 1 = RSA, 3 = EC); fall back to the top-level JCA `algorithm`
    // string. Reject unknown algorithms loudly instead of silently minting a
    // mismatched EC key.
    let algorithm = match get_i64("key_algorithm") {
        Some(1) => KeyAlgorithm::Rsa2048,
        Some(3) => KeyAlgorithm::EcP256,
        Some(other) => {
            return Err(anyhow!("unsupported key_algorithm: {other}"));
        }
        None => key_algorithm(payload),
    };

    let collect_enum = |vals: Vec<&Value>| -> Vec<i32> {
        vals.iter()
            .filter_map(|v| v.as_i64())
            .map(|n| n as i32)
            .collect()
    };
    let mgf_digest = parse_mgf_digest(get("mgf_digest"))?;

    Ok(KeySpec {
        algorithm,
        ec_curve: get_i64("ec_curve").and_then(|v| KmEcCurve::try_from(v as i32).ok()),
        key_size: get_i64("key_size").map(|v| v as u32),
        purposes: collect_enum(get_arr("purpose"))
            .iter()
            .filter_map(|&n| KmKeyPurpose::try_from(n).ok())
            .collect(),
        digests: collect_enum(get_arr("digest"))
            .iter()
            .filter_map(|&n| KmDigest::try_from(n).ok())
            .collect(),
        mgf_digest,
        paddings: collect_enum(get_arr("padding"))
            .iter()
            .filter_map(|&n| KmPadding::try_from(n).ok())
            .collect(),
        rsa_public_exponent: get_i64("rsa_public_exponent").map(|v| v as u64),
        cert_subject_der: get("certificate_subject")
            .map(|v| b64_decode(v).with_context(|| "decode certificate_subject"))
            .transpose()?,
        cert_not_before: get_i64("certificate_not_before_ms")
            .map(|ms| DateTime { ms_since_epoch: ms }),
        cert_not_after: get_i64("certificate_not_after_ms")
            .map(|ms| DateTime { ms_since_epoch: ms }),
        // `certificate_serial` (A-side CERTIFICATE_SERIAL tag) is optional;
        // when present the real TEE mints the leaf with that serial instead of
        // a random 16-byte value.
        cert_serial: get("certificate_serial")
            .map(|v| b64_decode(v).with_context(|| "decode certificate_serial"))
            .transpose()?,
    })
}

fn parse_mgf_digest(value: Option<&Value>) -> Result<Option<KmDigest>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let raw = value
        .as_i64()
        .and_then(|raw| i32::try_from(raw).ok())
        .ok_or_else(|| {
            keymint_error(
                KmErrorCode::UnsupportedMgfDigest,
                format!("invalid mgf_digest value: {value}"),
            )
        })?;
    let digest = KmDigest::try_from(raw).map_err(|_| {
        keymint_error(
            KmErrorCode::UnsupportedMgfDigest,
            format!("unsupported mgf_digest value: {raw}"),
        )
    })?;
    if digest == KmDigest::None {
        return Err(keymint_error(
            KmErrorCode::UnsupportedMgfDigest,
            "OAEP MGF digest cannot be NONE",
        ));
    }
    Ok(Some(digest))
}

fn b64(v: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(v)
}

fn cert_chain_json(chain: &[Vec<u8>]) -> Vec<Value> {
    chain.iter().map(|der| Value::String(b64(der))).collect()
}

/// Logs every certificate in a chain: index, DER length and the parsed
/// subject/issuer when x509_cert can decode it (helps debugging what the real
/// TEE actually minted vs what the server expects).
fn log_cert_chain(tag: &str, chain: &[Vec<u8>]) {
    if chain.is_empty() {
        log::info!("cert_chain[{tag}]: EMPTY");
        return;
    }
    let mut lines = Vec::new();
    for (i, der) in chain.iter().enumerate() {
        let parsed = x509_cert::Certificate::from_der(der).ok().map(|c| {
            let tbs = c.tbs_certificate();
            format!("subject={} issuer={}", tbs.subject(), tbs.issuer())
        });
        match parsed {
            Some(info) => lines.push(format!("#{i} {}B {info}", der.len())),
            None => lines.push(format!("#{i} {}B (unparsable)", der.len())),
        }
    }
    log::debug!(
        "cert_chain[{tag}]: {} certs :: {}",
        chain.len(),
        lines.join(" | ")
    );
}

/// Picks a signing key algorithm from the payload (defaults to EC P-256).
fn key_algorithm(payload: &Value) -> KeyAlgorithm {
    let algo = payload
        .get("algorithm")
        .and_then(Value::as_str)
        .unwrap_or("");
    let up = algo.to_uppercase();
    if up.contains("RSA") {
        KeyAlgorithm::Rsa2048
    } else {
        KeyAlgorithm::EcP256
    }
}

fn alias_of(payload: &Value, default: &str) -> String {
    payload
        .get("alias")
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn handle_generate_attest(_task_type: &str, payload: &Value) -> Result<Value> {
    let (app_id_der, challenge) = extract_attestation_context(payload)?;
    check_app_id_der(&app_id_der).with_context(|| {
        "attestation_application_id is not a valid AttestationApplicationId DER"
    })?;
    let alias = alias_of(payload, "attest");
    let spec = parse_key_spec(payload)?;
    // The requested purposes are forwarded unchanged (see
    // tee_ops::build_attestation_params). For an App Attest Key this mirrors
    // AOSP keystore2: a lone PURPOSE_ATTEST_KEY is accepted by the real TEE
    // (verified working on-device); such a key is later used only as an
    // `AttestationKey` when the A-side signs a child certificate.

    // The A-side forwards the requesting security level (1 = TEE, 2 = StrongBox)
    // in `device_attest_context.attestation_security_level`. A StrongBox request
    // is served by this B-side device's real `/strongbox` HAL when one exists;
    // otherwise we return an explicit error so the A-side falls back to its own
    // local software keybox (never silently mislabelling a TEE chain as StrongBox).
    let security_level = payload
        .get("device_attest_context")
        .and_then(|c| c.get("attestation_security_level"))
        .and_then(Value::as_i64)
        .unwrap_or(1);

    let session = if security_level == 2 {
        match tee_ops::generate_attest_key_on(
            SYSTEM_KEYMINT_STRONGBOX,
            &alias,
            &challenge,
            &app_id_der,
            &spec,
        ) {
            Ok(session) => session,
            Err(e) => {
                let err_str = format!("{e:#}");
                // Distinguish the root cause so operators can tell apart:
                //   - HAL not present (binder connect failed)
                //   - HAL present but attestation keys not provisioned (-74)
                //   - HAL present but hardware unavailable (-68)
                //   - Parameter/version incompatibility (other KeyMint errors)
                // AOSP keystore2 does not retry on -74 (it is a hard
                // failure that propagates to the caller); the three-tier
                // server fallback then handles recovery without ever
                // mislabelling a TEE chain as StrongBox.
                let reason = if err_str.contains("[km_error=-74]") {
                    "HAL exists but attestation keys not provisioned (factory provisioning issue)"
                } else if err_str.contains("[km_error=-68]") {
                    "HAL exists but hardware type unavailable"
                } else if err_str.contains("[km_error=") {
                    "HAL rejected key generation (possible parameter/version mismatch)"
                } else if err_str.contains("connect")
                    || err_str.contains("NameNotFound")
                    || err_str.contains("not found")
                {
                    "StrongBox HAL service not present on this device"
                } else {
                    "strongbox generateKey failed"
                };
                log::warn!("B-side StrongBox unavailable ({reason}): {err_str}");
                let mut result = ommegaclient_b::keymaster::relay_tee::relay_error_result(&e);
                result["error"] = json!(format!("strongbox not supported: {reason}"));
                return Ok(result);
            }
        }
    } else {
        tee_ops::generate_attest_key(&alias, &challenge, &app_id_der, &spec)?
    };
    let profile = tee_ops::identity_profile()?;
    if session.km_version != profile.hardware_version {
        anyhow::bail!(
            "KeyMint identity changed while minting: profile hardware={} session hardware={}",
            profile.hardware_version,
            session.km_version
        );
    }

    log_cert_chain("attest", &session.cert_chain);
    Ok(json!({
        "alias": alias,
        "cert_chain": cert_chain_json(&session.cert_chain),
        "public_key": b64(&tee_ops::get_public_key(&alias)?),
        "remote_profile": {
            "interface_version": profile.interface_version,
            "interface_hash": profile.interface_hash,
            "profile_version": profile.profile_version,
            "hardware_version": session.km_version,
            "security_level": security_level,
            "keymint_name": profile.keymint_name,
            "keymint_author": profile.keymint_author,
            "has_strongbox": profile.has_strongbox,
        },
    }))
}

fn handle_profile(_task_type: &str, _payload: &Value) -> Result<Value> {
    let profile = tee_ops::identity_profile()?;
    Ok(json!({
        "interface_version": profile.interface_version,
        "interface_hash": profile.interface_hash,
        "profile_version": profile.profile_version,
        "hardware_version": profile.hardware_version,
        "security_level": profile.security_level,
        "keymint_name": profile.keymint_name,
        "keymint_author": profile.keymint_author,
        "has_strongbox": profile.has_strongbox,
    }))
}

fn handle_sign(_task_type: &str, payload: &Value) -> Result<Value> {
    let alias = alias_of(payload, "attest");
    let algorithm = payload
        .get("algorithm")
        .and_then(Value::as_str)
        .unwrap_or("SHA256withECDSA")
        .to_string();
    let data = b64_decode(
        payload
            .get("data")
            .ok_or_else(|| anyhow!("payload missing data"))?,
    )?;
    let sig = tee_ops::sign(&alias, &data, &algorithm)?;
    Ok(json!({
        "alias": alias,
        "algorithm": algorithm,
        "data": b64(&sig),
    }))
}

fn handle_decrypt(_task_type: &str, payload: &Value) -> Result<Value> {
    let alias = alias_of(payload, "attest");
    let algorithm = payload
        .get("algorithm")
        .and_then(Value::as_str)
        .unwrap_or("RSA/ECB/PKCS1Padding")
        .to_string();
    let data = b64_decode(
        payload
            .get("data")
            .ok_or_else(|| anyhow!("payload missing data"))?,
    )?;
    let plain = tee_ops::decrypt(&alias, &data, &algorithm)?;
    Ok(json!({
        "alias": alias,
        "algorithm": algorithm,
        "data": b64(&plain),
    }))
}

fn handle_agree(_task_type: &str, payload: &Value) -> Result<Value> {
    let alias = payload
        .get("alias")
        .and_then(Value::as_str)
        .filter(|alias| !alias.trim().is_empty())
        .ok_or_else(|| {
            keymint_error(KmErrorCode::InvalidArgument, "agreement requires an alias")
        })?;
    let encoded = payload
        .get("peer_public_key")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            keymint_error(
                KmErrorCode::InvalidArgument,
                "agreement requires base64 peer_public_key",
            )
        })?;
    if encoded.len() > 220 {
        return Err(keymint_error(
            KmErrorCode::InvalidInputLength,
            "peer_public_key exceeds the maximum DER SubjectPublicKeyInfo size",
        ));
    }
    let peer_public_key = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| {
            keymint_error(
                KmErrorCode::InvalidArgument,
                "invalid peer_public_key base64",
            )
        })?;
    let secret = tee_ops::agree_key(alias, &peer_public_key)?;
    Ok(json!({
        "alias": alias,
        "data": b64(&secret),
    }))
}

fn handle_task(cfg: &RelayConfig, task_id: &str, task_type: &str, payload: &Value) -> Result<()> {
    let handler: fn(&str, &Value) -> Result<Value> = match task_type {
        "profile" => handle_profile,
        "attest" => handle_generate_attest,
        "sign" => handle_sign,
        "decrypt" => handle_decrypt,
        "agree" => handle_agree,
        other => {
            log::warn!("task {task_id} type={other} not supported, reporting failure");
            post_result(
                cfg,
                task_id,
                &json!({ "error": format!("unsupported task type: {other}") }),
            )?;
            return Ok(());
        }
    };

    let start = std::time::Instant::now();
    log::info!("processing task {task_id} type={task_type}");
    let result = match handler(task_type, payload) {
        Ok(v) => v,
        Err(e) => {
            log::error!("task {task_id} type={task_type} failed: {e:#}");
            ommegaclient_b::keymaster::relay_tee::relay_error_result(&e)
        }
    };
    let outcome = if result.get("error").is_some() {
        "failed"
    } else {
        "ok"
    };
    log::info!(
        "task {task_id} type={task_type} {outcome} in {:?}",
        start.elapsed()
    );
    post_result(cfg, task_id, &result)
}

/// Background thread: hot-reload the config when `relay.conf` changes or the
/// `restart.all` marker appears. The live config is updated in place via the
/// shared `RwLock`, so the poll loop keeps running and never conflicts with the
/// wrapper (the wrapper does not kill the relay on config changes).
fn spawn_config_watcher(shared: Arc<RwLock<RelayConfig>>, last_mtime: u64) {
    thread::spawn(move || {
        let mut last = last_mtime;
        loop {
            thread::sleep(Duration::from_millis(RELOAD_POLL_MS));

            let restart_requested = std::path::Path::new(RESTART_MARKER).exists();
            let changed = file_mtime(CONF_PATH).is_some_and(|m| m != last);

            if !restart_requested && !changed {
                continue;
            }
            // Re-read; if it fails (e.g. transient), keep the previous config.
            let reloaded = match load_config() {
                Ok((cfg, source)) => Some((cfg, source)),
                Err(e) => {
                    log::warn!("config reload failed, keeping previous: {e:#}");
                    None
                }
            };
            if let Some((cfg, source)) = reloaded {
                if let Ok(mut guard) = shared.write() {
                    log::info!(
                        "config hot-reloaded from {source}: server={} device={}",
                        cfg.server,
                        cfg.device_id
                    );
                    *guard = cfg;
                    // Drop the HTTP client so the next request rebuilds it
                    // with fresh connections to the (possibly new) server.
                    reset_http_client();
                }
                if let Some(m) = file_mtime(CONF_PATH) {
                    last = m;
                }
            }
            // Clear the restart marker so a single touch triggers one reload.
            let _ = std::fs::remove_file(RESTART_MARKER);
        }
    });
}

struct TeeWork {
    cfg: RelayConfig,
    task_id: String,
    task_type: String,
    payload: Value,
}

fn spawn_tee_workers() -> mpsc::SyncSender<TeeWork> {
    let (tx, rx) = mpsc::sync_channel::<TeeWork>(0);
    let rx = Arc::new(Mutex::new(rx));
    for i in 0..TEE_WORKERS {
        let rx = rx.clone();
        thread::Builder::new()
            .name(format!("tee-{i}"))
            .spawn(move || loop {
                let work = {
                    let guard = match rx.lock() {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                    guard.recv()
                };
                match work {
                    Ok(work) => {
                        if let Err(e) =
                            handle_task(&work.cfg, &work.task_id, &work.task_type, &work.payload)
                        {
                            log::error!("handle_task failed: {e:#}");
                        }
                    }
                    Err(_) => return,
                }
            })
            .unwrap_or_else(|e| panic!("spawn TEE worker {i}: {e}"));
    }
    tx
}

fn run_poll_loop(shared: Arc<RwLock<RelayConfig>>, tx: mpsc::SyncSender<TeeWork>) {
    let mut transient_ms = POLL_BACKOFF_MIN_MS;
    let mut rejected_ms = AUTH_BACKOFF_MIN_MS;
    let mut endpoint = String::new();
    loop {
        let cfg = match shared.read() {
            Ok(g) => g.clone(),
            Err(_) => {
                log::error!("config lock poisoned");
                thread::sleep(Duration::from_millis(1000));
                continue;
            }
        };
        // A changed server or token is a new credential to try: clear the
        // backoff so a fix made in the WebUI applies on the next poll rather
        // than after the current (possibly minutes-long) wait.
        let current_endpoint = format!("{}|{}", cfg.server, cfg.token);
        if current_endpoint != endpoint {
            if !endpoint.is_empty() {
                log::info!("relay endpoint or token changed; clearing poll backoff");
            }
            endpoint = current_endpoint;
            transient_ms = POLL_BACKOFF_MIN_MS;
            rejected_ms = AUTH_BACKOFF_MIN_MS;
        }
        match poll_tasks(&cfg) {
            Ok(Some((task_id, task_type, payload))) => {
                transient_ms = POLL_BACKOFF_MIN_MS;
                rejected_ms = AUTH_BACKOFF_MIN_MS;
                log::info!("poll received task {task_id} type={task_type}");
                if tx
                    .send(TeeWork {
                        cfg,
                        task_id,
                        task_type,
                        payload,
                    })
                    .is_err()
                {
                    return;
                }
            }
            Ok(None) => {
                // Long poll timed out cleanly: the link is healthy.
                transient_ms = POLL_BACKOFF_MIN_MS;
                rejected_ms = AUTH_BACKOFF_MIN_MS;
            }
            Err(PollError { error, failure }) => {
                let (wait, next_transient, next_rejected) =
                    next_poll_backoff(failure, transient_ms, rejected_ms);
                transient_ms = next_transient;
                rejected_ms = next_rejected;
                match failure {
                    PollFailure::Rejected => log::warn!(
                        "poll rejected by server (check the relay token / card expiry): \
                         {error:#}; retrying in {}s",
                        wait.as_secs()
                    ),
                    PollFailure::Throttled(_) => log::warn!(
                        "poll throttled by server: {error:#}; retrying in {}s",
                        wait.as_secs()
                    ),
                    PollFailure::Transient => {
                        log::warn!("poll failed: {error:#}; retrying in {}ms", wait.as_millis())
                    }
                }
                thread::sleep(wait);
            }
        }
    }
}

fn main() {
    let (log_enabled, log_level, logcat_enabled, logcat_level) = preload_log_config();
    ommegaclient_b::logging::init_logger(log_enabled, log_level, logcat_enabled, logcat_level);
    let _instance_lock = match acquire_instance_lock() {
        Ok(lock) => lock,
        Err(error) => {
            log::error!("relay refused duplicate startup: {error:#}");
            std::process::exit(1);
        }
    };
    let (cfg, source) = match load_config() {
        Ok(c) => c,
        Err(e) => {
            log::error!("relay config error: {e:#}");
            update_module_status("Ommega Attestation Relay Module ❌ 启动失败");
            std::process::exit(1);
        }
    };
    let shared: Arc<RwLock<RelayConfig>> = Arc::new(RwLock::new(cfg));
    let last_mtime = file_mtime(CONF_PATH).unwrap_or(0);
    spawn_config_watcher(shared.clone(), last_mtime);
    // We drive the real hardware keymint (a binder HAL) directly, so a binder
    // process state must be up before we start serving tasks.
    let _ = rsbinder::ProcessState::init_default();
    {
        let g = shared.read().map(|g| g.clone()).unwrap_or(RelayConfig {
            server: String::new(),
            device_id: String::new(),
            machine_id: String::new(),
            token: String::new(),
        });
        log::info!(
            "relay daemon starting (config from {source}) server={} device={} machine={}",
            g.server,
            g.device_id,
            g.machine_id
        );
    }
    update_module_status("Ommega Attestation Relay Module ✅ 运行中");
    rsbinder::ProcessState::start_thread_pool();
    // Reload persisted TEE sessions so aliases from before a relay restart stay
    // usable (key blobs are self-contained and still valid for begin/finish).
    ommegaclient_b::keymaster::tee_ops::load_all_sessions();
    // One poll thread (this one) keeps a long-poll open while TEE_WORKERS run
    // generateKey, so Duck's parallel attested generateKey calls do not queue
    // behind a single HTTP round-trip after each TEE op.
    let tx = spawn_tee_workers();
    run_poll_loop(shared, tx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ommegaclient_b::keymaster::relay_tee::relay_error_result;

    fn assert_mgf_error(value: Value) {
        let error = parse_mgf_digest(Some(&value)).unwrap_err();
        assert_eq!(
            relay_error_result(&error)["keymint_error_code"],
            KmErrorCode::UnsupportedMgfDigest as i32
        );
    }

    #[test]
    fn a_rejected_credential_backs_off_instead_of_hammering() {
        // 40 invalid requests per hour is the server's per-address budget. At
        // the old flat 1s retry the relay spent it in 40 seconds and then sat
        // in "too many invalid requests" for the rest of the window.
        assert_eq!(poll_failure_for(401, None), PollFailure::Rejected);
        assert_eq!(poll_failure_for(403, None), PollFailure::Rejected);

        let mut rejected_ms = AUTH_BACKOFF_MIN_MS;
        let mut transient_ms = POLL_BACKOFF_MIN_MS;
        let mut total = Duration::ZERO;
        let mut attempts = 0;
        // One hour of continuous rejection must stay well inside the budget.
        while total < Duration::from_secs(3600) {
            let (wait, next_transient, next_rejected) =
                next_poll_backoff(PollFailure::Rejected, transient_ms, rejected_ms);
            assert!(
                wait >= Duration::from_millis(AUTH_BACKOFF_MIN_MS),
                "a rejection must never retry faster than the auth floor"
            );
            assert!(wait <= Duration::from_millis(AUTH_BACKOFF_MAX_MS));
            transient_ms = next_transient;
            rejected_ms = next_rejected;
            total += wait;
            attempts += 1;
        }
        assert!(
            attempts < 40,
            "{attempts} attempts in an hour would exhaust the server's 40-request \
             invalid budget and lock the device out"
        );
    }

    #[test]
    fn throttling_honours_the_servers_own_hint() {
        let hint = Duration::from_secs(12);
        assert_eq!(
            poll_failure_for(429, Some(hint)),
            PollFailure::Throttled(Some(hint))
        );
        let (wait, ..) = next_poll_backoff(
            PollFailure::Throttled(Some(hint)),
            POLL_BACKOFF_MIN_MS,
            AUTH_BACKOFF_MIN_MS,
        );
        assert_eq!(wait, hint);

        // No hint: fall back to a fixed, generous pause.
        assert_eq!(poll_failure_for(503, None), PollFailure::Throttled(None));
        let (wait, ..) = next_poll_backoff(
            PollFailure::Throttled(None),
            POLL_BACKOFF_MIN_MS,
            AUTH_BACKOFF_MIN_MS,
        );
        assert_eq!(wait, Duration::from_millis(THROTTLE_BACKOFF_MS));
    }

    #[test]
    fn transient_failures_grow_then_settle_at_the_ceiling() {
        assert_eq!(poll_failure_for(500, None), PollFailure::Transient);
        assert_eq!(poll_failure_for(502, None), PollFailure::Transient);

        let mut transient_ms = POLL_BACKOFF_MIN_MS;
        let rejected_ms = AUTH_BACKOFF_MIN_MS;
        let mut waits = Vec::new();
        for _ in 0..8 {
            let (wait, next_transient, _) =
                next_poll_backoff(PollFailure::Transient, transient_ms, rejected_ms);
            transient_ms = next_transient;
            waits.push(wait);
        }
        assert_eq!(waits[0], Duration::from_millis(POLL_BACKOFF_MIN_MS));
        assert!(waits[1] > waits[0], "backoff must grow");
        assert!(
            waits
                .iter()
                .all(|w| *w <= Duration::from_millis(POLL_BACKOFF_MAX_MS)),
            "backoff must stay bounded so a recovered server is noticed promptly"
        );
        assert_eq!(
            *waits.last().unwrap(),
            Duration::from_millis(POLL_BACKOFF_MAX_MS)
        );
    }

    #[test]
    fn a_throttled_result_upload_is_retried_not_discarded() {
        // The finished TEE work is already in hand; these only mean "wait".
        assert!(!result_status_is_permanent(429));
        assert!(!result_status_is_permanent(408));
        // A genuinely wrong request will not improve on retry.
        assert!(result_status_is_permanent(401));
        assert!(result_status_is_permanent(403));
        assert!(result_status_is_permanent(404));
        assert!(result_status_is_permanent(400));
        // 5xx is retried by the caller's own branch.
        assert!(!result_status_is_permanent(500));
        assert!(!result_status_is_permanent(503));
    }

    #[test]
    fn mgf_digest_is_exact_or_absent() {
        assert_eq!(parse_mgf_digest(None).unwrap(), None);
        assert_eq!(
            parse_mgf_digest(Some(&json!(KmDigest::Sha224 as i32))).unwrap(),
            Some(KmDigest::Sha224)
        );
        assert_mgf_error(json!(KmDigest::None as i32));
        assert_mgf_error(json!(99));
        assert_mgf_error(json!("SHA-256"));
        assert_mgf_error(json!(i64::MAX));
    }

    #[test]
    fn agree_rejects_oversized_peer_spki_with_keymint_code() {
        let payload = json!({
            "alias": "synthetic",
            "peer_public_key": b64(&[0; 165]),
        });
        let error = handle_agree("agree", &payload).unwrap_err();
        assert_eq!(
            relay_error_result(&error)["keymint_error_code"],
            KmErrorCode::InvalidInputLength as i32
        );
    }

    #[test]
    fn agree_rejects_missing_alias_and_malformed_input_before_session_or_hal() {
        for payload in [
            json!({"peer_public_key": "AA=="}),
            json!({"alias": " ", "peer_public_key": "AA=="}),
            json!({"alias": "synthetic"}),
            json!({"alias": "synthetic", "peer_public_key": 5}),
            json!({"alias": "synthetic", "peer_public_key": "?"}),
            json!({"alias": "synthetic", "peer_public_key": ""}),
            json!({"alias": "synthetic", "peer_public_key": "AA=="}),
        ] {
            let error = handle_agree("agree", &payload).unwrap_err();
            assert_eq!(
                relay_error_result(&error)["keymint_error_code"],
                KmErrorCode::InvalidArgument as i32
            );
        }
    }
}
