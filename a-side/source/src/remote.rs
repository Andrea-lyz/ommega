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

//! Remote TEE relay client (A-side).
//!
//! When `[remote].enabled` is set in `config.toml`, attestation / sign / decrypt
//! for A-side keys are forwarded to the relay_server (and from there to a B-side
//! real hardware TEE).  This module uses `reqwest::blocking::Client` to talk to
//! the relay_server API, mirroring the protocol used by the B-side relay agent
//! (`ommegaclient-b`).
//!
//! The relay_server uses a self-signed certificate by default, so
//! `tls_insecure` (default true) accepts any server certificate — matching the
//! B-side client behaviour.
//!
//! Two long-lived reqwest clients (insecure / verify) are kept in `OnceLock`s
//! and selected per request based on the current `tls_insecure` config value.
//! Each client has its own internal connection pool, so concurrent requests
//! are not serialised through a single Mutex — fixing the previous single-
//! connection pool bottleneck.  reqwest also natively supports chunked transfer
//! encoding and HTTP keep-alive, both of which the hand-written client did not.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use reqwest::blocking::Client;
use reqwest::Method;
use serde_json::{json, Value};

use crate::config;

pub(crate) mod attestation_metrics;
mod backend;
mod health;
pub use backend::RemoteRelayBackend;
use health::{record_remote_failure, record_remote_ok};
pub use health::{remote_health, RemoteHealth, REMOTE_HEALTH_PATH};

const CONNECT_TIMEOUT_MS: u64 = 3000;
const READ_TIMEOUT_MS: u64 = 30_000;
const SIGN_REQUEST_BUDGET: Duration = Duration::from_secs(10);
/// Upper bound honoured from a server `Retry-After`. A keystore call is
/// blocking an app, so we cap the wait well below the HAL watchdog rather than
/// obeying an arbitrarily large value.
const MAX_RETRY_AFTER_SECS: u64 = 3;

/// A relay-server client.  All configuration is read from `config().remote`.
pub struct RemoteRelay;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);
/// Cached B-side identity profile.
///
/// Deliberately not a `OnceLock`: it has to be clearable. While it was
/// write-once, saving the WebUI config could not refresh it, so the
/// "uncheck enable remote, save, re-check, save" ritual users fall back on
/// genuinely did nothing to the running daemon.
static REMOTE_IDENTITY_PROFILE: RwLock<Option<RemoteIdentityProfile>> = RwLock::new(None);
/// Why the relay last refused us, if it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteRejection {
    /// 401/403: the server did not accept the token.
    Unauthorized,
    /// 429/503: the server is throttling or its backend is unavailable.
    Throttled,
    /// Any other non-2xx response.
    Other,
}

impl RemoteRejection {
    fn classify(status: u16) -> Option<Self> {
        match status {
            200..=299 => None,
            401 | 403 => Some(Self::Unauthorized),
            408 | 429 | 503 => Some(Self::Throttled),
            _ => Some(Self::Other),
        }
    }

    /// Whether the same request is worth sending again shortly.
    fn is_retryable(self) -> bool {
        matches!(self, Self::Throttled)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Throttled => "throttled",
            Self::Other => "error",
        }
    }
}

/// Drop the cached identity profile so the next attestation re-reads it from
/// the relay. Called when the runtime config is applied.
pub fn invalidate_identity_profile() {
    // A different relay route must never leave the previous B's application view active.
    if let Err(error) = std::fs::remove_file(kmr_common::cpu_mask::PROFILE_PATH) {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!("could not invalidate CPU property snapshot: {error}");
        }
    }
    if let Ok(mut guard) = REMOTE_IDENTITY_PROFILE.write() {
        if guard.take().is_some() {
            log::info!("remote identity profile cache cleared after config change");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteIdentityProfile {
    pub interface_version: i32,
    pub interface_hash: String,
    pub profile_version: i32,
    pub hardware_version: i32,
    pub security_level: i32,
    pub keymint_name: String,
    pub keymint_author: String,
    pub has_strongbox: bool,
    /// Operation limit reported by the B-side implementation, when present.
    pub max_operations: Option<usize>,
}

fn new_request_id(kind: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{kind}-{:x}-{now:x}-{counter:x}", std::process::id())
}

#[derive(Debug)]
struct RelayKeyMintError {
    code: kmr_wire::keymint::ErrorCode,
    message: String,
}

impl std::fmt::Display for RelayKeyMintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (KeyMint {})", self.message, self.code as i32)
    }
}

impl std::error::Error for RelayKeyMintError {}

fn relay_business_error(response: &Value) -> Option<RelayKeyMintError> {
    let result = response.get("result").unwrap_or(response);
    let error = result.get("error")?;
    let message = error.as_str();
    let code = result
        .get("keymint_error_code")
        .filter(|_| message.is_some())
        .and_then(Value::as_i64)
        .and_then(|code| i32::try_from(code).ok())
        .filter(|code| *code < 0)
        .and_then(|code| kmr_wire::keymint::ErrorCode::try_from(code).ok())
        .unwrap_or(kmr_wire::keymint::ErrorCode::UnknownError);
    Some(RelayKeyMintError {
        code,
        message: message.unwrap_or("invalid relay error").to_string(),
    })
}

fn decode_relay_response(path: &str, status: u16, body: &[u8]) -> Result<Value> {
    if !(200..300).contains(&status) {
        // The server reports HAL failures using HTTP 500. Authentication,
        // rate-limit and transport failures must not become HAL errors.
        if status == 500 {
            if let Ok(response) = serde_json::from_slice::<Value>(body) {
                if let Some(error) = relay_business_error(&response) {
                    return Err(
                        anyhow::Error::new(error).context(format!("remote {path} HTTP {status}"))
                    );
                }
            }
        }
        return Err(anyhow!(
            "remote {path} HTTP {status}: {}",
            String::from_utf8_lossy(body)
                .chars()
                .take(256)
                .collect::<String>()
        ));
    }
    if body.is_empty() {
        return Err(anyhow!("remote {path} returned an empty success response"));
    }
    let response = serde_json::from_slice(body)
        .map_err(|e| anyhow!("relay returned malformed JSON (status {status}): {e}"))?;
    if let Some(error) = relay_business_error(&response) {
        return Err(anyhow::Error::new(error).context(format!("remote {path}")));
    }
    Ok(response)
}

fn remote_operation_error(error: anyhow::Error, operation: &str) -> kmr_common::Error {
    let code = error
        .downcast_ref::<RelayKeyMintError>()
        .map(|failure| failure.code)
        .unwrap_or(kmr_wire::keymint::ErrorCode::UnknownError);
    kmr_common::km_verr!(code, "remote {operation}: {error:#}")
}

// ── reqwest client management ──────────────────────────────────────────

/// Returns a reqwest blocking client configured for the current `tls_insecure`
/// setting.  Two clients (insecure / verify) are lazily initialised and cached
/// in `OnceLock`s — each carries its own connection pool, so flipping
/// `tls_insecure` at runtime just picks the other pool.
fn get_client() -> Result<Client> {
    let insecure = config::config()
        .read()
        .map(|c| c.remote.tls_insecure)
        .unwrap_or(true);

    if insecure {
        static INSECURE_CLIENT: OnceLock<Client> = OnceLock::new();
        Ok(INSECURE_CLIENT.get_or_init(|| build_client(true)).clone())
    } else {
        static VERIFY_CLIENT: OnceLock<Client> = OnceLock::new();
        Ok(VERIFY_CLIENT.get_or_init(|| build_client(false)).clone())
    }
}

fn build_client(insecure: bool) -> Client {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_millis(CONNECT_TIMEOUT_MS))
        .timeout(Duration::from_millis(READ_TIMEOUT_MS))
        .user_agent("ommegaclient-a/1.3");

    if insecure {
        builder = builder.danger_accept_invalid_certs(true);
    }

    builder
        .build()
        .expect("failed to build reqwest blocking client")
}

/// Minimal HTTP(S) request helper backed by reqwest.
///
/// Returns `(status, body)`.  HTTP error responses (4xx / 5xx) are returned as
/// `Ok` with the corresponding status code — only transport-level failures
/// produce `Err`.  This matches the original hand-written client contract so
/// callers (`post_json` and its retry logic) behave identically.
/// Parse a `Retry-After` delta-seconds header, clamped so a bad or hostile
/// value cannot stall a keystore call.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let raw = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let secs: u64 = raw.trim().parse().ok()?;
    Some(Duration::from_secs(secs.clamp(1, MAX_RETRY_AFTER_SECS)))
}

fn attempt_timeout(deadline: Instant, now: Instant) -> Result<Duration> {
    let remaining = deadline.saturating_duration_since(now);
    if remaining.is_zero() {
        return Err(anyhow!("remote request budget exhausted"));
    }
    Ok(remaining.min(Duration::from_millis(READ_TIMEOUT_MS)))
}

fn http_request(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<(u16, Vec<u8>, Option<Duration>)> {
    let client = get_client()?;
    let method = method
        .parse::<Method>()
        .map_err(|e| anyhow!("invalid HTTP method {method}: {e}"))?;

    let mut req = client.request(method, url).timeout(timeout);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    if let Some(b) = body {
        req = req.body(b.to_vec());
    }

    let resp = req
        .send()
        .with_context(|| format!("HTTP request to {url}"))?;

    let status = resp.status().as_u16();
    let retry_after = parse_retry_after(resp.headers());
    let body_bytes = resp
        .bytes()
        .with_context(|| format!("reading response body from {url}"))?
        .to_vec();

    Ok((status, body_bytes, retry_after))
}

// ── RemoteRelay public API ─────────────────────────────────────────────

impl RemoteRelay {
    // Copy only the requested value while the lock is held. HTTP requests run
    // after the guard has been released.
    fn remote<T>(select: impl FnOnce(&config::RemoteConfig) -> Result<T>) -> Result<T> {
        let guard = config::config()
            .read()
            .map_err(|_| anyhow!("config lock poisoned"))?;
        select(&guard.remote)
    }

    fn base_url() -> Result<String> {
        Self::remote(|r| {
            if !r.enabled {
                return Err(anyhow!("remote relay not enabled"));
            }
            if r.url.is_empty() {
                return Err(anyhow!("remote url not configured"));
            }
            Ok(r.url.trim_end_matches('/').to_string())
        })
    }

    fn token() -> Result<String> {
        Self::remote(|r| {
            if r.token.is_empty() {
                return Err(anyhow!("remote token not configured"));
            }
            Ok(r.token.clone())
        })
    }

    fn device_id() -> Result<String> {
        Self::remote(|r| {
            if r.device_id.is_empty() {
                return Err(anyhow!("remote device_id not configured"));
            }
            Ok(r.device_id.clone())
        })
    }

    /// POST a JSON body to a relay endpoint. Returns `Ok(None)` only when both
    /// transport attempts fail. HTTP/protocol errors are hard failures and must
    /// not be confused with an unavailable relay, because doing so silently
    /// changes the attestation backend.
    fn post_json(path: &str, body: &Value) -> Result<Option<Value>> {
        let url = format!("{}{}", Self::base_url()?, path);
        let body_str = serde_json::to_string(body)?;
        let headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-Relay-Token".to_string(), Self::token()?),
        ];
        // All attempts and backoff share one budget. A signature is on an
        // app's request path; attestation retains its longer existing budget.
        let budget = if path == "/api/sign/" {
            SIGN_REQUEST_BUDGET
        } else {
            Duration::from_millis(2 * READ_TIMEOUT_MS) + Duration::from_secs(MAX_RETRY_AFTER_SECS)
        };
        let deadline = Instant::now() + budget;
        let send = || {
            http_request(
                "POST",
                &url,
                &headers,
                Some(body_str.as_bytes()),
                attempt_timeout(deadline, Instant::now())?,
            )
        };
        // Transient transport failures (connect timeout, network jitter, TLS
        // handshake) are retried once before giving up. Without the retry a
        // single dropped attempt falls back to the local software keybox and
        // briefly emits a self-signed chain.
        //
        // Most HTTP error responses are NOT retried — the server answered, and
        // its body carries the real result. The exception is an explicit
        // "slow down" (429/503): the request was never processed, so resending
        // it once after the server's own `Retry-After` recovers the call
        // instead of failing an attestation over a moment of throttling.
        let (status, resp) = match send() {
            Ok((status, body, retry_after)) => match RemoteRejection::classify(status) {
                Some(rejection) if rejection.is_retryable() => {
                    let wait = retry_after
                        .unwrap_or(Duration::from_millis(500))
                        .min(Duration::from_secs(MAX_RETRY_AFTER_SECS));
                    if wait >= deadline.saturating_duration_since(Instant::now()) {
                        record_remote_failure(
                            Some(status),
                            rejection.as_str(),
                            "request budget exhausted during backoff",
                        );
                        return decode_relay_response(path, status, &body).map(Some);
                    }
                    log::warn!(
                        "remote {path} throttled (HTTP {status}); retrying once in {}ms",
                        wait.as_millis()
                    );
                    std::thread::sleep(wait);
                    match send() {
                        Ok((status, body, _)) => (status, body),
                        Err(second) => {
                            log::warn!("remote {path} unavailable after retry: {second:#}");
                            record_remote_failure(None, "transport", &second.to_string());
                            return Ok(None);
                        }
                    }
                }
                _ => (status, body),
            },
            Err(first) => {
                log::warn!("remote {path} transport error, retrying once: {first:#}");
                match send() {
                    Ok((status, body, _)) => (status, body),
                    Err(second) => {
                        log::warn!("remote {path} unavailable after retry: {second:#}");
                        record_remote_failure(None, "transport", &second.to_string());
                        return Ok(None);
                    }
                }
            }
        };
        // Record what the server actually said, so the WebUI can report
        // "rejected by server" rather than leaving the user to guess.
        match RemoteRejection::classify(status) {
            None => record_remote_ok(),
            Some(rejection) => {
                let detail = String::from_utf8_lossy(&resp)
                    .chars()
                    .take(200)
                    .collect::<String>();
                log::warn!(
                    "remote {path} rejected by server: HTTP {status} ({})",
                    rejection.as_str()
                );
                record_remote_failure(Some(status), rejection.as_str(), &detail);
            }
        }
        // A 2xx response that is not JSON is a server/protocol error, not
        // "remote unavailable" — surface it loudly instead of silently falling
        // back to the local software keybox (which would emit a self-signed
        // chain and hide the real failure).
        decode_relay_response(path, status, &resp).map(Some)
    }

    fn fetch_identity_profile() -> Result<RemoteIdentityProfile> {
        let body = json!({
            "request_id": new_request_id("profile"),
            "device_id": Self::device_id()?,
        });
        let response = Self::post_json("/api/profile/", &body)?
            .ok_or_else(|| anyhow!("remote profile unavailable after transport retry"))?;
        let result = response.get("result").unwrap_or(&response);
        if let Some(error) = result.get("error").and_then(Value::as_str) {
            return Err(anyhow!("remote profile failed: {error}"));
        }
        let profile = parse_identity_profile(result)?;
        // Piggyback on the existing profile exchange; app startup has no network or HAL work.
        let app_profile = result
            .get("app_identity")
            .cloned()
            .ok_or_else(|| anyhow!("B has not supplied an exp6 property profile"))
            .and_then(|value| {
                serde_json::from_value::<kmr_common::cpu_mask::Profile>(value).map_err(Into::into)
            })
            .and_then(|value| value.text().map_err(|error| anyhow!(error)));
        let saved = app_profile.and_then(|contents| {
            let path = std::path::Path::new(kmr_common::cpu_mask::PROFILE_PATH);
            let previous = match std::fs::read(path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            kmr_common::runtime::fs::atomic_compare_replace_preserving_metadata(
                path,
                previous.as_deref(),
                contents.as_bytes(),
                0o600,
                kmr_common::consts::KEYSTORE_UID,
                kmr_common::consts::KEYSTORE_GID,
            )
            .context("cache B property snapshot")
        });
        if let Err(error) = saved {
            let _ = std::fs::remove_file(kmr_common::cpu_mask::PROFILE_PATH);
            log::warn!("CPU property view unavailable: {error:#}");
        }
        Ok(profile)
    }

    /// Forward an attestation request.  `challenge` is the caller's nonce.
    pub fn attest(
        challenge: &[u8],
        alias: &str,
        app_id_der: &[u8],
        params: &kmr_ta::device::RemoteAttestParams,
        cert_serial: Option<&[u8]>,
    ) -> Result<Option<(Value, Duration)>> {
        let mut ctx = serde_json::Map::new();
        if let Some(issuer) = &params.attestation_key {
            ctx.insert(
                "attestation_key".to_string(),
                json!({
                    "alias": issuer.alias,
                    "issuer_subject_name": base64_encode(&issuer.issuer_subject_name),
                }),
            );
        }
        ctx.insert(
            "attestation_application_id".to_string(),
            Value::String(base64_encode(app_id_der)),
        );
        // Use the device's real verified-boot key/hash from the resolved trust
        // config (client-a forwards `AndroidDeviceUtils.bootKey/bootHash`).
        let (vb_key, vb_hash) = Self::verified_boot_values();
        ctx.insert(
            "verified_boot_key".to_string(),
            Value::String(base64_encode(&vb_key)),
        );
        ctx.insert(
            "verified_boot_hash".to_string(),
            Value::String(base64_encode(&vb_hash)),
        );
        // Forward the resolved boot state instead of a hardcoded verified/locked
        // device: an unavailable root of trust is reported as unverified/unlocked,
        // and the relay-minted record must match the local one.
        let (device_locked, verified_boot_state) = match config::config().read() {
            Ok(g) => (
                g.trust.device_locked,
                if g.trust.verified_boot_state { 0 } else { 2 },
            ),
            Err(_) => (true, 0),
        };
        ctx.insert("device_locked".to_string(), Value::Bool(device_locked));
        ctx.insert(
            "verified_boot_state".to_string(),
            Value::from(verified_boot_state),
        );
        ctx.insert(
            "creation_datetime_ms".to_string(),
            Value::from(Self::now_ms()),
        );
        // Forward the app's key-generation parameters (mirroring client-a) so
        // the B-side TEE mints a key matching the request, not an EC-P256 default.
        if let Some(algo) = params.key_algorithm {
            ctx.insert("key_algorithm".to_string(), Value::from(algo));
        }
        if let Some(size) = params.key_size {
            ctx.insert("key_size".to_string(), Value::from(size));
        }
        if let Some(curve) = params.ec_curve {
            ctx.insert("ec_curve".to_string(), Value::from(curve));
        }
        if !params.purpose.is_empty() {
            ctx.insert(
                "purpose".to_string(),
                Value::Array(params.purpose.iter().copied().map(Value::from).collect()),
            );
        }
        if !params.digest.is_empty() {
            ctx.insert(
                "digest".to_string(),
                Value::Array(params.digest.iter().copied().map(Value::from).collect()),
            );
        }
        if !params.padding.is_empty() {
            ctx.insert(
                "padding".to_string(),
                Value::Array(params.padding.iter().copied().map(Value::from).collect()),
            );
        }
        if let Some(mgf) = params.mgf_digest {
            ctx.insert("mgf_digest".to_string(), Value::from(mgf));
        }
        if let Some(exponent) = params.rsa_public_exponent {
            ctx.insert("rsa_public_exponent".to_string(), Value::from(exponent));
        }
        if let Some(subject) = &params.certificate_subject {
            ctx.insert(
                "certificate_subject".to_string(),
                Value::String(base64_encode(subject)),
            );
        }
        if let Some(not_before) = params.certificate_not_before_ms {
            ctx.insert(
                "certificate_not_before_ms".to_string(),
                Value::from(not_before),
            );
        }
        if let Some(not_after) = params.certificate_not_after_ms {
            ctx.insert(
                "certificate_not_after_ms".to_string(),
                Value::from(not_after),
            );
        }
        if let Some(serial) = cert_serial {
            ctx.insert(
                "certificate_serial".to_string(),
                Value::String(base64_encode(serial)),
            );
        }
        // Forward the requesting security level (1 = TEE, 2 = StrongBox) so the
        // relay tags the attestation extension the same way the A-side reported
        // it. Without this a STRONGBOX request minted remotely is mislabelled as
        // TEE (the relay's `attestation_security_level` default is 1).
        if let Some(security_level) = params.security_level {
            ctx.insert(
                "attestation_security_level".to_string(),
                Value::from(i64::from(security_level)),
            );
        }
        // Carry the original policy. A enforces authentication locally; B
        // retains the independent unlock constraint but cannot validate A's HAT.
        if !params.user_auth.is_empty() {
            ctx.insert(
                "user_auth".to_string(),
                Value::Array(
                    params
                        .user_auth
                        .iter()
                        .map(|(tag, value)| json!([*tag, *value]))
                        .collect(),
                ),
            );
        }
        if let Some(properties) = encode_device_properties(&params.device_properties) {
            ctx.insert("device_properties".to_string(), properties);
        }
        let profile = remote_identity_profile()?;
        ctx.insert(
            "attest_record_version".to_string(),
            Value::from(profile.profile_version),
        );
        ctx.insert(
            "keymint_record_version".to_string(),
            Value::from(profile.profile_version),
        );
        // Forward the device's OS version + security patch level so the relay
        // can emit KM_TAG_OS_VERSION (705) / KM_TAG_OS_PATCH_LEVEL (706) in the
        // teeEnforced authorization list. Software attestations that omit these
        // two tags are flagged as tampered by self-check apps (e.g. 密钥认证
        // 1.7's `checkTagOrderMisordered` requires 704/705/706 all present).
        let os_version = match config::config().read() {
            Ok(g) => (g.trust.os_version.max(0) as u32) * 10000,
            Err(_) => {
                (kmr_common::android_version::android_major_version().unwrap_or(16) as u32) * 10000
            }
        };
        if os_version > 0 {
            ctx.insert("os_version".to_string(), Value::from(os_version));
        }
        let os_patch_level = match config::config().read() {
            Ok(g) => patch_level_to_yyyymm(&g.trust.os_patchlevel),
            Err(_) => None,
        }
        .or_else(|| {
            crate::plat::resetprop::read_string_property("ro.build.version.security_patch")
                .as_deref()
                .and_then(patch_level_to_yyyymm)
        });
        if let Some(patch) = os_patch_level {
            ctx.insert("os_patch_level".to_string(), Value::from(patch));
        }
        // KeyMint 3.0+ also carries per-partition patch levels
        // (KM_TAG_VENDOR_PATCH_LEVEL 707 / KM_TAG_BOOT_PATCH_LEVEL 708).
        // Real TEE attestations include them; omitting them makes the server
        // keybox layer fail STRONG integrity checks that expect them.
        // Prefer the device's real partition patch props; the resolved config
        // `[trust]` values can be stale/malformed (e.g. a bogus "2026-30").
        let vendor_patch_level =
            crate::plat::resetprop::read_string_property("ro.vendor.build.security_patch")
                .as_deref()
                .and_then(patch_level_to_yyyymm)
                .or_else(|| match config::config().read() {
                    Ok(g) => patch_level_to_yyyymm(&g.trust.vendor_patchlevel),
                    Err(_) => None,
                })
                .or(os_patch_level);
        if let Some(patch) = vendor_patch_level {
            ctx.insert("vendor_patch_level".to_string(), Value::from(patch));
        }
        let boot_patch_level = crate::plat::vbmeta::boot_patch_property()
            .as_deref()
            .and_then(patch_level_to_yyyymm)
            .or_else(|| match config::config().read() {
                Ok(g) => patch_level_to_yyyymm(&g.trust.boot_patchlevel),
                Err(_) => None,
            })
            .or(os_patch_level);
        if let Some(patch) = boot_patch_level {
            ctx.insert("boot_patch_level".to_string(), Value::from(patch));
        }
        // The B-side reads `device_attest_context` (nested form) for the
        // appid and optional serial; the full key params are passed through.
        let body = json!({
            "request_id": new_request_id("attest"),
            "challenge": base64_encode(challenge),
            "alias": alias,
            "device_id": Self::device_id()?,
            "device_attest_context": Value::Object(ctx),
        });
        let started = Instant::now();
        Self::post_json("/api/attest/", &body)
            .map(|result| result.map(|value| (value, started.elapsed())))
    }

    /// Millis since epoch (used for `creation_datetime_ms`).
    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    /// Resolved verified-boot key/hash from `config().trust` (32 bytes each).
    /// Falls back to zeros if the config lock is unavailable (never fatal).
    fn verified_boot_values() -> ([u8; 32], [u8; 32]) {
        match config::config().read() {
            Ok(g) => (g.trust.vb_key, g.trust.vb_hash),
            Err(_) => ([0u8; 32], [0u8; 32]),
        }
    }

    /// Forward a sign request for a remote key.
    pub fn sign(alias: &str, data: &[u8], algorithm: &str) -> Result<Option<Value>> {
        let body = json!({
            "request_id": new_request_id("sign"),
            "alias": alias,
            "data": base64_encode(data),
            "algorithm": algorithm,
            "device_id": Self::device_id()?,
        });
        Self::post_json("/api/sign/", &body)
    }

    /// Forward a decrypt request for a remote key.
    pub fn decrypt(alias: &str, data: &[u8], algorithm: &str) -> Result<Option<Value>> {
        let body = json!({
            "request_id": new_request_id("decrypt"),
            "alias": alias,
            "data": base64_encode(data),
            "algorithm": algorithm,
            "device_id": Self::device_id()?,
        });
        Self::post_json("/api/decrypt/", &body)
    }

    /// Forward an EC key-agreement request for a remote key.
    pub fn agree_key(alias: &str, peer_public_key: &[u8]) -> Result<Option<Value>> {
        let body = json!({
            "request_id": new_request_id("agree"),
            "alias": alias,
            "peer_public_key": base64_encode(peer_public_key),
            "device_id": Self::device_id()?,
        });
        Self::post_json("/api/agree/", &body)
    }
}

/// Keep public attestation IDs byte-exact across the JSON relay boundary.
fn encode_device_properties(properties: &[(u32, Vec<u8>)]) -> Option<Value> {
    if properties.is_empty() {
        return None;
    }
    Some(Value::Array(
        properties
            .iter()
            .map(|(tag, bytes)| json!([tag, base64_encode(bytes)]))
            .collect(),
    ))
}

fn base64_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Convert a "YYYY-MM-DD" patch level string to the YYYYMM integer the relay
/// expects for KM_TAG_OS_PATCH_LEVEL (706).
fn patch_level_to_yyyymm(value: &str) -> Option<u32> {
    let s = value.trim();
    // Accept both "YYYY-MM-DD" (or "YYYY-MM") and bare "YYYYMM".
    if s.len() >= 6 {
        let (year, month) = if s.as_bytes().get(4) == Some(&b'-') {
            (s[0..4].parse::<u32>().ok()?, s[5..7].parse::<u32>().ok()?)
        } else {
            (s[0..4].parse::<u32>().ok()?, s[4..6].parse::<u32>().ok()?)
        };
        if (1..=12).contains(&month) {
            return Some(year * 100 + month);
        }
    }
    None
}

/// Reject the relay's StrongBox-to-TEE "robustness" marker.
///
/// A physical KeyMint never serves a request that selected the StrongBox
/// security level with TEE-minted material: the framework asks keystore2 for
/// the StrongBox instance and a device without one fails the operation
/// (`HARDWARE_TYPE_UNAVAILABLE`).  Accepting the marker would hand back a chain
/// whose attestation security level is TRUSTED_ENVIRONMENT while
/// `KeyMetadata.keySecurityLevel` still reports STRONGBOX, so the two
/// app-visible security-level sources would disagree.
fn reject_strongbox_demotion(
    result: &Value,
    requested_security_level: Option<i32>,
) -> Result<(), kmr_common::Error> {
    let demoted = match result.get("strongbox_demoted") {
        None => return Ok(()),
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(kmr_common::km_err!(
                VerificationFailed,
                "remote strongbox_demoted marker is not a boolean"
            ));
        }
    };
    if !demoted {
        return Ok(());
    }
    if requested_security_level != Some(kmr_wire::keymint::SecurityLevel::Strongbox as i32) {
        return Err(kmr_common::km_err!(
            VerificationFailed,
            "remote StrongBox demotion marked for a non-StrongBox request"
        ));
    }
    let effective = result
        .get("effective_security_level")
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            kmr_common::km_err!(
                VerificationFailed,
                "remote StrongBox demotion missing effective security level"
            )
        })?;
    if effective != i64::from(kmr_wire::keymint::SecurityLevel::TrustedEnvironment as i32) {
        return Err(kmr_common::km_err!(
            VerificationFailed,
            "remote StrongBox demotion did not report TEE as the effective level"
        ));
    }
    Err(kmr_common::km_err!(
        HardwareTypeUnavailable,
        "remote identity cannot serve the requested StrongBox security level"
    ))
}

fn parse_identity_profile(value: &Value) -> Result<RemoteIdentityProfile> {
    let field_i32 = |name: &str| -> Result<i32> {
        value
            .get(name)
            .and_then(Value::as_i64)
            .and_then(|number| i32::try_from(number).ok())
            .ok_or_else(|| anyhow!("remote profile missing or invalid {name}"))
    };
    let field_string = |name: &str| -> Result<String> {
        value
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("remote profile missing or invalid {name}"))
    };

    let interface_version = field_i32("interface_version")?;
    let profile_version = field_i32("profile_version")?;
    let hardware_version = field_i32("hardware_version")?;
    if !(1..=5).contains(&interface_version) || profile_version != interface_version * 100 {
        return Err(anyhow!(
            "remote profile version mismatch: interface={interface_version} profile={profile_version}"
        ));
    }
    if !matches!(hardware_version, 100 | 200 | 300 | 400 | 500) {
        return Err(anyhow!(
            "remote profile returned invalid hardware version {hardware_version}"
        ));
    }
    let security_level = field_i32("security_level")?;
    if security_level != kmr_wire::keymint::SecurityLevel::TrustedEnvironment as i32 {
        return Err(anyhow!(
            "remote default KeyMint is not TEE: security_level={security_level}"
        ));
    }
    let interface_hash = field_string("interface_hash")?;
    let max_operations = match value.get("max_operations") {
        None | Some(Value::Null) => None,
        Some(Value::Number(number)) => {
            let limit = number
                .as_u64()
                .ok_or_else(|| anyhow!("remote profile returned a non-integer max_operations"))?;
            if limit == 0 || limit > kmr_ta::MAX_OPERATION_LIMIT as u64 {
                return Err(anyhow!(
                    "remote profile returned max_operations {limit} outside 1..={}",
                    kmr_ta::MAX_OPERATION_LIMIT
                ));
            }
            Some(limit as usize)
        }
        Some(_) => return Err(anyhow!("remote profile returned an invalid max_operations")),
    };

    if interface_hash.len() != 40 || !interface_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!("remote profile returned an invalid interface hash"));
    }

    Ok(RemoteIdentityProfile {
        max_operations,
        interface_version,
        interface_hash,
        profile_version,
        hardware_version,
        security_level,
        keymint_name: field_string("keymint_name")?,
        keymint_author: value
            .get("keymint_author")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| anyhow!("remote profile missing or invalid keymint_author"))?,
        has_strongbox: value
            .get("has_strongbox")
            .and_then(Value::as_bool)
            .ok_or_else(|| anyhow!("remote profile missing or invalid has_strongbox"))?,
    })
}

pub(crate) fn remote_identity_profile() -> Result<RemoteIdentityProfile> {
    if let Ok(guard) = REMOTE_IDENTITY_PROFILE.read() {
        if let Some(profile) = guard.as_ref() {
            return Ok(profile.clone());
        }
    }
    let profile = RemoteRelay::fetch_identity_profile()?;
    if let Ok(mut guard) = REMOTE_IDENTITY_PROFILE.write() {
        *guard = Some(profile.clone());
    }
    Ok(profile)
}

/// Convenience: `true` if remote relay is enabled in config.
pub fn remote_enabled() -> bool {
    match config::config().read() {
        Ok(g) => g.remote.enabled && !g.remote.url.is_empty(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kmr_wire::keymint::SecurityLevel;

    #[test]
    fn signature_retry_and_backoff_cannot_reset_the_total_budget() {
        let started = Instant::now();
        let deadline = started + SIGN_REQUEST_BUDGET;
        assert_eq!(
            attempt_timeout(deadline, started).unwrap(),
            Duration::from_secs(10)
        );
        // Three seconds of transport plus three seconds of backoff leave
        // four seconds for the next attempt, not another complete timeout.
        assert_eq!(
            attempt_timeout(deadline, started + Duration::from_secs(6)).unwrap(),
            Duration::from_secs(4)
        );
        assert!(attempt_timeout(deadline, started + Duration::from_secs(10)).is_err());
        assert_eq!(
            attempt_timeout(started + Duration::from_secs(63), started).unwrap(),
            Duration::from_secs(30)
        );
    }

    fn response_error_code(
        path: &str,
        operation: &str,
        status: u16,
        body: &Value,
    ) -> kmr_wire::keymint::ErrorCode {
        let error =
            decode_relay_response(path, status, &serde_json::to_vec(body).unwrap()).unwrap_err();
        match remote_operation_error(error, operation).kind() {
            kmr_common::ErrorKind::Hal(code, _) => *code,
            kind => panic!("unexpected error kind: {kind:?}"),
        }
    }

    #[test]
    fn structured_hal_errors_survive_http_and_contexts() {
        for code in [-3, -26, -30, -62, -66, -68, -74, -78] {
            let failure = json!({ "error": "HAL rejected operation", "keymint_error_code": code });
            for status in [200, 500] {
                for body in [&failure, &json!({ "result": failure })] {
                    assert_eq!(
                        response_error_code("/api/sign/", "sign", status, body) as i32,
                        code
                    );
                }
            }
        }

        let unsupported = json!({
            "error": "server_keybox does not support agreement",
            "keymint_error_code": -100,
        });
        assert_eq!(
            response_error_code("/api/agree/", "agree", 500, &unsupported),
            kmr_wire::keymint::ErrorCode::Unimplemented
        );
        let old_server = remote_operation_error(
            decode_relay_response("/api/agree/", 404, b"not found").unwrap_err(),
            "agree",
        );
        assert!(matches!(
            old_server.kind(),
            kmr_common::ErrorKind::Hal(kmr_wire::keymint::ErrorCode::UnknownError, _)
        ));
    }

    #[test]
    fn legacy_malformed_and_non_hal_failures_remain_unknown() {
        use kmr_wire::keymint::ErrorCode;
        for body in [
            json!({ "error": "legacy [km_error=-30]" }),
            json!({ "error": "invalid", "keymint_error_code": 0 }),
            json!({ "error": "invalid", "keymint_error_code": 1 }),
            json!({ "error": "invalid", "keymint_error_code": -999999 }),
            json!({ "error": "invalid", "keymint_error_code": "-30" }),
            json!({ "error": "invalid", "keymint_error_code": -2147483649_i64 }),
            json!({ "error": null, "keymint_error_code": -30 }),
        ] {
            assert_eq!(
                response_error_code("/api/sign/", "sign", 500, &body),
                ErrorCode::UnknownError
            );
        }
        let body = json!({ "error": "access denied", "keymint_error_code": -30 });
        for status in [401, 403, 429, 502, 503] {
            assert_eq!(
                response_error_code("/api/sign/", "sign", status, &body),
                ErrorCode::UnknownError
            );
        }
        for body in [b"".as_slice(), b"not JSON"] {
            assert!(decode_relay_response("/api/sign/", 200, body).is_err());
        }
    }

    #[test]
    fn server_rejections_are_classified_so_the_ui_can_explain_them() {
        // A rejected token is the server's verdict: retrying will not help, and
        // the user needs to be told it is a token problem rather than left
        // toggling "enable remote" in the WebUI.
        for status in [401, 403] {
            let rejection = RemoteRejection::classify(status).unwrap();
            assert_eq!(rejection, RemoteRejection::Unauthorized);
            assert!(!rejection.is_retryable());
            assert_eq!(rejection.as_str(), "unauthorized");
        }
        // Throttling is temporary and the request never ran, so one retry is
        // both safe and worth doing.
        for status in [408, 429, 503] {
            let rejection = RemoteRejection::classify(status).unwrap();
            assert_eq!(rejection, RemoteRejection::Throttled);
            assert!(rejection.is_retryable());
            assert_eq!(rejection.as_str(), "throttled");
        }
        for status in [400, 404, 500, 502] {
            let rejection = RemoteRejection::classify(status).unwrap();
            assert_eq!(rejection, RemoteRejection::Other);
            assert!(!rejection.is_retryable());
        }
        for status in [200, 201, 204, 299] {
            assert_eq!(RemoteRejection::classify(status), None);
        }
    }

    #[test]
    fn a_long_retry_after_cannot_stall_a_keystore_call() {
        // The header is advisory; a keystore call is blocking an app, so an
        // outsized value must be clamped rather than obeyed.
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "86400".parse().unwrap());
        assert_eq!(
            parse_retry_after(&headers),
            Some(Duration::from_secs(MAX_RETRY_AFTER_SECS))
        );

        headers.insert(reqwest::header::RETRY_AFTER, "2".parse().unwrap());
        assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(2)));

        // A zero or malformed value must not become an immediate retry.
        headers.insert(reqwest::header::RETRY_AFTER, "0".parse().unwrap());
        assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(1)));
        headers.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(parse_retry_after(&headers), None);
        assert_eq!(parse_retry_after(&reqwest::header::HeaderMap::new()), None);
    }

    #[test]
    fn the_identity_profile_cache_can_be_cleared() {
        let profile = RemoteIdentityProfile {
            interface_version: 3,
            interface_hash: "hash".into(),
            profile_version: 300,
            hardware_version: 300,
            security_level: 1,
            keymint_name: "synthetic".into(),
            keymint_author: "synthetic".into(),
            has_strongbox: false,
            max_operations: None,
        };
        *REMOTE_IDENTITY_PROFILE.write().unwrap() = Some(profile);
        assert!(REMOTE_IDENTITY_PROFILE.read().unwrap().is_some());
        // Saving the config has to actually drop this, otherwise a corrected
        // token or server URL keeps serving the stale profile.
        invalidate_identity_profile();
        assert!(REMOTE_IDENTITY_PROFILE.read().unwrap().is_none());
    }

    #[test]
    fn successful_relay_payloads_are_unchanged() {
        for body in [
            json!({ "signature": "AQID" }),
            json!({ "data": "" }),
            json!({ "cert_chain": ["leaf", "issuer"] }),
            json!({ "result": { "signature": "AQID" } }),
        ] {
            assert_eq!(
                decode_relay_response("/api/sign/", 200, &serde_json::to_vec(&body).unwrap())
                    .unwrap(),
                body
            );
        }
    }

    #[test]
    fn device_property_context_is_absent_unless_requested_and_byte_exact() {
        assert_eq!(encode_device_properties(&[]), None);
        assert_eq!(
            encode_device_properties(&[
                (710, b"OnePlus".to_vec()),
                (717, vec![0, 255, 65]),
                (711, Vec::new()),
            ]),
            Some(json!([[710, "T25lUGx1cw=="], [717, "AP9B"], [711, ""]]))
        );
    }

    #[test]
    fn parses_coherent_remote_keymint_profile() {
        let profile = parse_identity_profile(&json!({
            "interface_version": 2,
            "interface_hash": "207c9f218b9b9e4e74ff5232eb16511eca9d7d2e",
            "profile_version": 200,
            "hardware_version": 400,
            "security_level": 1,
            "keymint_name": "QTI KeyMint",
            "keymint_author": "Qualcomm",
            "has_strongbox": false,
        }))
        .unwrap();

        assert_eq!(profile.interface_version, 2);
        assert_eq!(profile.profile_version, 200);
        assert_eq!(profile.hardware_version, 400);
        assert!(!profile.has_strongbox);
    }

    #[test]
    fn validates_the_remote_operation_limit() {
        let base = json!({
            "interface_version": 2,
            "interface_hash": "207c9f218b9b9e4e74ff5232eb16511eca9d7d2e",
            "profile_version": 200,
            "hardware_version": 400,
            "security_level": 1,
            "keymint_name": "QTI KeyMint",
            "keymint_author": "Qualcomm",
            "has_strongbox": false,
        });
        assert_eq!(parse_identity_profile(&base).unwrap().max_operations, None);

        let mut with_limit = base.clone();
        with_limit["max_operations"] = json!(32);
        assert_eq!(
            parse_identity_profile(&with_limit).unwrap().max_operations,
            Some(32)
        );

        for invalid in [json!(0), json!(1025), json!(-1), json!("many")] {
            let mut profile = base.clone();
            profile["max_operations"] = invalid;
            assert!(
                parse_identity_profile(&profile).is_err(),
                "invalid max_operations must be rejected"
            );
        }
    }

    #[test]
    fn rejects_mixed_remote_keymint_profile_versions() {
        let result = parse_identity_profile(&json!({
            "interface_version": 2,
            "interface_hash": "207c9f218b9b9e4e74ff5232eb16511eca9d7d2e",
            "profile_version": 300,
            "hardware_version": 400,
            "security_level": 1,
            "keymint_name": "QTI KeyMint",
            "keymint_author": "Qualcomm",
            "has_strongbox": false,
        }));

        assert!(result.is_err());
    }

    #[test]
    fn rejects_explicit_strongbox_to_tee_demotion() {
        let result = json!({
            "strongbox_demoted": true,
            "effective_security_level": 1,
        });

        assert!(reject_strongbox_demotion(&result, Some(SecurityLevel::Strongbox as i32)).is_err());
    }

    #[test]
    fn legacy_response_keeps_strict_requested_level() {
        assert!(
            reject_strongbox_demotion(&json!({}), Some(SecurityLevel::Strongbox as i32)).is_ok()
        );
    }

    #[test]
    fn rejects_demotion_for_non_strongbox_request() {
        let result = json!({
            "strongbox_demoted": true,
            "effective_security_level": 1,
        });

        assert!(
            reject_strongbox_demotion(&result, Some(SecurityLevel::TrustedEnvironment as i32))
                .is_err()
        );
    }

    #[test]
    fn rejects_demotion_without_effective_tee_level() {
        let result = json!({
            "strongbox_demoted": true,
            "effective_security_level": 2,
        });

        assert!(reject_strongbox_demotion(&result, Some(SecurityLevel::Strongbox as i32)).is_err());
    }
}
