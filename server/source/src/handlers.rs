//! HTTP handlers for all relay endpoints.
//!
//! Mirrors `relay_server/apps/relay_api/views.py`. Two fulfilment modes:
//!   - physical (default): A-side creates a task, B-side polls & returns result
//!   - server_keybox:      A-side requests are intercepted and fulfilled locally

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

use crate::auth::AuthState;
use crate::config::Config;
use crate::db::Db;
use crate::fulfill::Fulfill;
use crate::queue::TaskStore;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub auth: Arc<AuthState>,
    pub store: Arc<TaskStore>,
    pub fulfill: Arc<Fulfill>,
    pub db: Option<Arc<Db>>,
    pub geo: Option<Arc<crate::geo::Ip2Region>>,
}

fn token_from_headers(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-relay-token")
        .and_then(|v| v.to_str().ok())
        .or_else(|| headers.get("x-api-token").and_then(|v| v.to_str().ok()))
}

pub(crate) fn client_ip(headers: &HeaderMap) -> String {
    // Prefer X-Real-IP injected by the inject_client_ip middleware, which is the
    // actual TCP socket address. X-Forwarded-For is client-supplied and trivially
    // spoofable, so it must never take precedence — otherwise anyone can claim a
    // whitelisted IP and bypass the IP allow/deny filter. It is kept only as a
    // fallback for reverse-proxy deployments that do not propagate X-Real-IP.
    if let Some(v) = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return v;
    }
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn json_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn keymint_error_code(result: &Value) -> Option<i32> {
    result.get("error")?.as_str()?;
    let code = i32::try_from(result.get("keymint_error_code")?.as_i64()?).ok()?;
    (code < 0).then_some(code)
}

fn task_error_response(message: &str, code: Option<i32>) -> Response {
    let mut result = json!({ "error": message });
    if let Some(code) = code {
        result["keymint_error_code"] = json!(code);
    }
    (StatusCode::INTERNAL_SERVER_ERROR, Json(result)).into_response()
}

fn auth_fail() -> Response {
    json_err(
        StatusCode::UNAUTHORIZED,
        "unauthorized: missing or invalid X-Relay-Token",
    )
}

/// Authenticate + rate-limit a request. Returns Ok(token) or an error response.
///
/// `role`: `Some("a")` for A-side endpoints, `Some("b")` for B-side, `None` for
/// role-agnostic endpoints (ping/health/admin status).
fn check_auth(
    state: &AppState,
    headers: &HeaderMap,
    role: Option<&str>,
) -> Result<String, Response> {
    let token = token_from_headers(headers).unwrap_or("").to_string();
    let ip = client_ip(headers);

    // IP allow/deny filter (A/B-side only; admin uses its own session auth).
    if !state.auth.ip_allowed(&ip) {
        return Err(json_err(
            StatusCode::FORBIDDEN,
            "access denied by IP filter",
        ));
    }

    // Authenticate first. Failed auth counts against the (much tighter)
    // invalid-request limit, keyed by client IP.
    if !state.auth.check_token(Some(&token), role, &ip) {
        if !state.auth.allow_invalid(&ip) {
            return Err(json_err(
                StatusCode::TOO_MANY_REQUESTS,
                "too many invalid requests",
            ));
        }
        return Err(auth_fail());
    }

    // Valid auth: rate limit by token (or IP when no token).
    let rl_key = if token.is_empty() { ip } else { token.clone() };
    if !state.auth.allow(&rl_key) {
        return Err(json_err(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded",
        ));
    }
    Ok(token)
}

/// Physical-mode fulfilment: enqueue a task for the exact requested B device
/// and wait for the result. Device substitution is deliberately forbidden.
async fn try_b_device_layer(
    state: &AppState,
    task_type: &str,
    body: &Value,
    device_id: &str,
) -> Option<Value> {
    if !state.store.is_device_online(device_id).await {
        return Some(json!({ "error": format!("B-side device {device_id} is not online") }));
    }
    let request_id = body.get("request_id").and_then(Value::as_str);
    let task_id = state
        .store
        .create_task(task_type, body.clone(), device_id, request_id)
        .await;
    let timeout = Duration::from_secs(state.cfg.wait_result_timeout_secs);
    match state.store.wait_for_result(&task_id, timeout).await {
        Some(mut result) => {
            if let Some(obj) = result.as_object_mut() {
                obj.insert("task_id".to_string(), json!(task_id));
            }
            Some(result)
        }
        None => Some(json!({
            "error": "task timeout: no B-side result",
            "task_id": task_id,
        })),
    }
}

/// Whether the A-side request explicitly asked for StrongBox (security_level=2).
fn is_strongbox_request(body: &Value) -> bool {
    body.get("device_attest_context")
        .and_then(|c| c.get("attestation_security_level"))
        .and_then(Value::as_i64)
        .or_else(|| {
            body.get("attestation_security_level")
                .and_then(Value::as_i64)
        })
        .unwrap_or(1)
        == 2
}

/// Rewrite the request's security level to a plain TEE request (level 1).
/// Both the `device_attest_context` entry and a top-level entry are rewritten
/// (b-app reads either), so every B-side relay interprets the downgrade.
fn demote_to_tee(body: &Value) -> Value {
    let mut b = body.clone();
    if let Some(ctx) = b.get_mut("device_attest_context") {
        if ctx.get("attestation_security_level").is_some() {
            ctx["attestation_security_level"] = json!(1);
        }
    }
    if b.get("attestation_security_level").is_some() {
        b["attestation_security_level"] = json!(1);
    }
    if let Some(request_id) = b.get("request_id").and_then(Value::as_str) {
        b["request_id"] = json!(format!("{request_id}-tee"));
    }
    b
}

/// Mark a successful StrongBox robustness retry with the security level that
/// actually minted the certificate chain.  A-side clients use both fields to
/// distinguish this explicit downgrade from an arbitrary level mismatch.
fn mark_strongbox_demotion(result: &mut Value) -> bool {
    let Some(obj) = result.as_object_mut() else {
        return false;
    };
    obj.insert("strongbox_demoted".to_string(), json!(true));
    obj.insert("effective_security_level".to_string(), json!(1));
    true
}

/// An attest result without a usable cert chain is treated as a failure so the
/// robustness demotion (or the next layer) gets a chance instead of forwarding
/// an empty chain to the A-side (which would silently fall back locally).
fn attest_chain_empty(task_type: &str, v: &Value) -> bool {
    if task_type != "attest" {
        return false;
    }
    match v.get("cert_chain") {
        None => true,
        Some(Value::Array(a)) => a.is_empty(),
        Some(_) => true,
    }
}

fn result_shape_valid(task_type: &str, value: &Value) -> bool {
    match task_type {
        "profile" => {
            value
                .get("interface_version")
                .and_then(Value::as_i64)
                .is_some()
                && value
                    .get("interface_hash")
                    .and_then(Value::as_str)
                    .is_some()
                && value
                    .get("profile_version")
                    .and_then(Value::as_i64)
                    .is_some()
                && value
                    .get("hardware_version")
                    .and_then(Value::as_i64)
                    .is_some()
                && value
                    .get("security_level")
                    .and_then(Value::as_i64)
                    .is_some()
                && value
                    .get("has_strongbox")
                    .and_then(Value::as_bool)
                    .is_some()
                && value
                    .get("keymint_name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| !name.trim().is_empty())
                && value
                    .get("keymint_author")
                    .and_then(Value::as_str)
                    .is_some_and(|author| author.is_empty() || !author.trim().is_empty())
        }
        "attest" => !attest_chain_empty(task_type, value),
        "sign" => value
            .get("signature")
            .or_else(|| value.get("data"))
            .and_then(Value::as_str)
            .is_some(),
        "decrypt" => value.get("data").and_then(Value::as_str).is_some(),
        "agree" => {
            use base64::Engine as _;
            value.get("error").is_none()
                && value
                    .get("data")
                    .and_then(Value::as_str)
                    .filter(|data| data.len() <= 88)
                    .and_then(|data| base64::engine::general_purpose::STANDARD.decode(data).ok())
                    .is_some_and(|data| matches!(data.len(), 28 | 32 | 48 | 66))
        }
        _ => false,
    }
}

/// Layer ② — server keybox (stored identity) local fulfilment.
/// Synchronous version that takes &Fulfill directly, for use inside spawn_blocking.
fn try_keybox_layer_sync(
    fulfill: &crate::fulfill::Fulfill,
    task_type: &str,
    body: &Value,
    device_id: &str,
) -> Option<Value> {
    match task_type {
        "attest" => fulfill.try_handle_attest(device_id, body),
        "sign" => fulfill.try_handle_sign(device_id, body),
        "decrypt" => fulfill.try_handle_decrypt(device_id, body),
        "agree" => Some(json!({
            "error": "server_keybox does not hold an EC agreement private key",
            "keymint_error_code": -100,
        })),
        _ => None,
    }
}

/// Shared logic for A-side task endpoints.
///
/// Each mode is strict: `physical` uses only the requested B device and
/// `server_keybox` uses only the stored identity. Transparently switching the
/// backend changes the key that owns an alias and breaks device continuity.
async fn run_a_side_task(state: &AppState, task_type: &str, body: &Value) -> Response {
    let device_id = body
        .get("device_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if device_id.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "device_id required");
    }
    if body
        .get("request_id")
        .and_then(Value::as_str)
        .is_some_and(|request_id| request_id.len() > 160)
    {
        return json_err(StatusCode::BAD_REQUEST, "request_id too long");
    }
    let ctx = body
        .get("device_attest_context")
        .cloned()
        .unwrap_or(Value::Null);
    let ctx_short = match &ctx {
        Value::Object(m) => {
            let mut s = String::new();
            for (k, v) in m {
                if k == "attestation_application_id" {
                    s.push_str(&format!(
                        "{k}=<appid-len:{}> ",
                        v.as_str().map(|x| x.len()).unwrap_or(0)
                    ));
                } else if k == "certificate_subject" {
                    s.push_str(&format!(
                        "{k}=<b64-len:{}> ",
                        v.as_str().map(|x| x.len()).unwrap_or(0)
                    ));
                } else {
                    s.push_str(&format!("{k}={v} "));
                }
            }
            s
        }
        _ => format!("{ctx}"),
    };
    tracing::info!(
        "run_a_side_task: type={task_type} device={device_id} alias={} ctx=[{ctx_short}]",
        body.get("alias").and_then(|v| v.as_str()).unwrap_or("")
    );

    let serverbox = state.fulfill.is_enabled();

    // StrongBox follows the selected strict backend. In physical mode the B
    // device must provide it, unless the separately controlled robustness mode
    // explicitly requests an honest retry as TEE.
    let order: &[&str] = if task_type == "profile" {
        // The remote identity must describe the physical B-side HAL even when
        // the server-keybox fulfilment mode is enabled for attestation.
        &["b"]
    } else if serverbox {
        &["keybox"]
    } else {
        &["b"]
    };

    let mut last_error: Option<String> = None;
    let mut last_keymint_error_code = None;
    for &layer in order {
        let result = match layer {
            "b" => try_b_device_layer(state, task_type, body, &device_id).await,
            "keybox" => {
                let fulfill = state.fulfill.clone();
                let tt = task_type.to_string();
                let b = body.clone();
                let did = device_id.clone();
                match tokio::task::spawn_blocking(move || {
                    try_keybox_layer_sync(&fulfill, &tt, &b, &did)
                })
                .await
                {
                    Ok(v) => v,
                    Err(e) => Some(json!({ "error": format!("spawn_blocking join error: {e}") })),
                }
            }
            _ => None,
        };
        match result {
            Some(v) if v.get("error").is_none() && result_shape_valid(task_type, &v) => {
                tracing::info!(
                    "run_a_side_task: type={task_type} layer={layer} result_keys={:?} has_cert_chain={}",
                    v.as_object().map(|m| m.keys().cloned().collect::<Vec<String>>()),
                    v.get("cert_chain").is_some(),
                );
                return Json(v).into_response();
            }
            Some(v) => {
                last_keymint_error_code = keymint_error_code(&v);
                let msg = if v.get("error").is_none() && !result_shape_valid(task_type, &v) {
                    format!("invalid {task_type} result shape")
                } else {
                    v.get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_string()
                };
                tracing::info!("run_a_side_task: type={task_type} layer={layer} failed: {msg}");
                // StrongBox robustness mode: a StrongBox attest that the B
                // device cannot fulfil (capability error — not supported /
                // keys not provisioned / HAL absent — or no cert chain at
                // all) is transparently retried as a TEE request on the B
                // side — the Android-standard silent fallback. The B side
                // tags the downgraded chain TRUSTED_ENVIRONMENT, so this is
                // an honest degradation, never a mislabelled StrongBox.
                // When robustness mode is off, return the capability error.
                if layer == "b"
                    && task_type == "attest"
                    && crate::strongbox::is_robust()
                    && is_strongbox_request(body)
                {
                    let demoted = demote_to_tee(body);
                    if let Some(mut dv) =
                        try_b_device_layer(state, task_type, &demoted, &device_id).await
                    {
                        if dv.get("error").is_none() && result_shape_valid(task_type, &dv) {
                            if !mark_strongbox_demotion(&mut dv) {
                                last_error = Some(
                                    "strongbox demotion returned a non-object result".to_string(),
                                );
                                continue;
                            }
                            tracing::info!(
                                "run_a_side_task: type={task_type} layer=b strongbox-robust demoted to TEE ok"
                            );
                            return Json(dv).into_response();
                        }
                        let dmsg =
                            if dv.get("error").is_none() && !result_shape_valid(task_type, &dv) {
                                "invalid attest result shape".to_string()
                            } else {
                                dv.get("error")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown error")
                                    .to_string()
                            };
                        tracing::info!(
                            "run_a_side_task: type={task_type} layer=b strongbox demotion retry failed: {dmsg}"
                        );
                    }
                }
                last_error = Some(msg);
            }
            None => {
                last_keymint_error_code = None;
                tracing::info!("run_a_side_task: type={task_type} layer={layer} not applicable");
                last_error = Some(format!("layer {layer} produced no result"));
            }
        }
    }

    task_error_response(
        &format!(
            "all fulfilment layers failed for device {device_id}: {}",
            last_error.unwrap_or_else(|| "unknown".to_string())
        ),
        last_keymint_error_code,
    )
}

// ---------------------------------------------------------------------------
// Basic endpoints
// ---------------------------------------------------------------------------

pub async fn ping() -> &'static str {
    "pong"
}

pub async fn health(State(state): State<AppState>) -> Response {
    let counts = state.store.counts().await;
    let devices = state.store.get_connected_devices().await;
    // `machine_id` is deliberately omitted (same as the public status page):
    // health is unauthenticated and machine ids are not meant to be public.
    let device_list: Vec<Value> = devices
        .iter()
        .map(|d| {
            json!({
                "device_id": d.device_id,
                "last_seen_ms": d.last_seen_ms,
            })
        })
        .collect();
    Json(json!({
        "status": "ok",
        "mode": if state.fulfill.is_enabled() { "server_keybox" } else { "physical" },
        "tasks": {
            "pending": counts.pending,
            "assigned": counts.assigned,
            "completed": counts.completed,
            "failed": counts.failed,
        },
        "connected_devices": device_list,
        "server_time_ms": chrono::Utc::now().timestamp_millis(),
    }))
    .into_response()
}

pub async fn cert_chain_dump(State(state): State<AppState>, headers: HeaderMap) -> Response {
    // Diagnostic: dump stored server identity chain (admin diagnostic). Requires
    // a valid A/B token so the keybox identity is not exposed to unauthenticated
    // callers.
    if let Err(r) = check_auth(&state, &headers, None) {
        return r;
    }
    let Some(db) = state.db.clone() else {
        return json_err(StatusCode::NOT_FOUND, "no db");
    };
    let result = tokio::task::spawn_blocking(move || db.get_active_device_identity()).await;
    match result {
        Ok(Ok(Some(id))) => Json(json!({
            "device_id": id.device_id,
            "algorithm": id.algorithm,
            "active": id.active,
            "certificate_chain_pem": id.certificate_chain_pem,
        }))
        .into_response(),
        Ok(Ok(None)) => json_err(StatusCode::NOT_FOUND, "no active server identity"),
        Ok(Err(e)) => json_err(StatusCode::INTERNAL_SERVER_ERROR, &format!("db error: {e}")),
        Err(e) => json_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("join error: {e}"),
        ),
    }
}

// ---------------------------------------------------------------------------
// A-side task endpoints
// ---------------------------------------------------------------------------

pub async fn attest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = check_auth(&state, &headers, Some("a")) {
        return r;
    }
    run_a_side_task(&state, "attest", &body).await
}

pub async fn profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = check_auth(&state, &headers, Some("a")) {
        return r;
    }
    run_a_side_task(&state, "profile", &body).await
}

pub async fn sign(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = check_auth(&state, &headers, Some("a")) {
        return r;
    }
    run_a_side_task(&state, "sign", &body).await
}

pub async fn decrypt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = check_auth(&state, &headers, Some("a")) {
        return r;
    }
    run_a_side_task(&state, "decrypt", &body).await
}

pub async fn agree(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = check_auth(&state, &headers, Some("a")) {
        return r;
    }
    run_a_side_task(&state, "agree", &body).await
}

// ---------------------------------------------------------------------------
// Client report (A-side diagnostics)
// ---------------------------------------------------------------------------

pub async fn client_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = check_auth(&state, &headers, Some("a")) {
        return r;
    }
    let Some(db) = state.db.clone() else {
        return Json(json!({ "status": "ok", "stored": false })).into_response();
    };
    let row = crate::db::ClientReportRow {
        device_id: body
            .get("device_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        level: body
            .get("level")
            .and_then(Value::as_str)
            .unwrap_or("info")
            .to_string(),
        code: body
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        message: body
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        detail_json: body
            .get("detail")
            .map(|d| d.to_string())
            .unwrap_or_else(|| "{}".to_string()),
        client_ip: client_ip(&headers),
        user_agent: headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let result = tokio::task::spawn_blocking(move || db.insert_client_report(&row)).await;
    match result {
        Ok(Ok(())) => Json(json!({ "status": "ok" })).into_response(),
        Ok(Err(e)) => json_err(StatusCode::INTERNAL_SERVER_ERROR, &format!("db error: {e}")),
        Err(e) => json_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("join error: {e}"),
        ),
    }
}

// ---------------------------------------------------------------------------
// B-side endpoints
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PollQuery {
    pub device_id: String,
    pub machine_id: Option<String>,
    pub timeout: Option<u64>,
}

pub async fn b_poll(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PollQuery>,
) -> Response {
    if let Err(r) = check_auth(&state, &headers, Some("b")) {
        return r;
    }
    if q.device_id.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "device_id required");
    }
    let machine_id = q.machine_id.unwrap_or_default();

    // Concurrency guard: reject if another machine is actively serving this device.
    if let Some(active) = state.store.get_active_machine_id(&q.device_id).await {
        if !machine_id.is_empty() && active != machine_id {
            return json_err(
                StatusCode::CONFLICT,
                "another machine is already serving this device",
            );
        }
    }

    let timeout_secs = q.timeout.unwrap_or(state.cfg.poll_timeout_secs);
    let timeout = Duration::from_secs(timeout_secs.min(120));

    match state
        .store
        .pop_for_b(&q.device_id, &machine_id, timeout)
        .await
    {
        Some(task) => Json(json!({
            "task_id": task.task_id,
            "task_type": task.task_type,
            "payload": task.payload,
            "target_device_id": task.target_device_id,
        }))
        .into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

pub async fn b_result(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = check_auth(&state, &headers, Some("b")) {
        return r;
    }
    let task_id = body
        .get("task_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let result = body.get("result").cloned().unwrap_or(Value::Null);
    let device_id = body
        .get("device_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if task_id.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "task_id required");
    }
    match state
        .store
        .complete_task(&task_id, result, &device_id)
        .await
    {
        Ok(()) => Json(json!({ "status": "ok" })).into_response(),
        Err(error) if error == "task not found" => json_err(StatusCode::NOT_FOUND, &error),
        Err(error) => json_err(StatusCode::CONFLICT, &error),
    }
}

pub async fn b_upload_keybox_identity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = check_auth(&state, &headers, Some("b")) {
        return r;
    }
    let fulfill = state.fulfill.clone();
    let b = body.clone();
    let device_id = body
        .get("device_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let result = tokio::task::spawn_blocking(move || {
        fulfill.try_handle_b_upload_keybox_identity(&device_id, &b)
    })
    .await;
    match result {
        Ok(Some(v)) => {
            // Validation / parse failures are client errors: surface them as
            // 400 (with the specific reason) instead of a 200-with-error body.
            if let Some(err) = v.get("error").and_then(Value::as_str) {
                json_err(StatusCode::BAD_REQUEST, err)
            } else {
                Json(v).into_response()
            }
        }
        Ok(None) => json_err(
            StatusCode::BAD_REQUEST,
            "server_keybox mode required for identity upload",
        ),
        Err(e) => json_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("join error: {e}"),
        ),
    }
}

pub async fn b_revoke_server_identity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = check_auth(&state, &headers, Some("b")) {
        return r;
    }
    let Some(db) = state.db.clone() else {
        return json_err(StatusCode::NOT_FOUND, "no db");
    };
    let device_id = body
        .get("device_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let result =
        tokio::task::spawn_blocking(move || db.set_device_identity_active(&device_id, false)).await;
    match result {
        Ok(Ok(())) => Json(json!({ "status": "ok" })).into_response(),
        Ok(Err(e)) => json_err(StatusCode::INTERNAL_SERVER_ERROR, &format!("db error: {e}")),
        Err(e) => json_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("join error: {e}"),
        ),
    }
}

pub async fn admin_cancel_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    // This is an admin operation: require a valid admin session (X-Relay-Session),
    // not just any A/B token.
    let sid = headers
        .get("x-relay-session")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !state.auth.check_session(&sid) {
        return json_err(
            StatusCode::UNAUTHORIZED,
            "unauthorized: missing or invalid session",
        );
    }
    let task_id = body
        .get("task_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if task_id.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "task_id required");
    }
    match state.store.cancel_task(&task_id).await {
        Ok(()) => Json(json!({ "status": "ok" })).into_response(),
        Err(_) => json_err(StatusCode::NOT_FOUND, "task not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agreement_state(server_keybox: bool) -> AppState {
        AppState {
            cfg: Arc::new(Config {
                wait_result_timeout_secs: 2,
                ..Config::default()
            }),
            auth: Arc::new(AuthState::new("synthetic-token".into(), 100, 60, true)),
            store: TaskStore::new(60, 60, 100, 60),
            fulfill: Fulfill::new(server_keybox, None),
            db: None,
            geo: None,
        }
    }

    fn agreement_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-relay-token", "synthetic-token".parse().unwrap());
        headers
    }

    #[tokio::test]
    async fn agreement_round_trip_uses_only_requested_b_and_keeps_hal_errors() {
        use base64::Engine as _;
        for result in [
            json!({"data": base64::engine::general_purpose::STANDARD.encode([0; 32])}),
            json!({"error": "wrong purpose", "keymint_error_code": -3}),
            json!({"error": "wrong curve", "keymint_error_code": -38}),
        ] {
            let state = agreement_state(false);
            state
                .store
                .pop_for_b("synthetic-b", "mock", Duration::ZERO)
                .await;
            let body = json!({"device_id": "synthetic-b", "alias": "key", "request_id": "agree-1", "peer_public_key": "synthetic-spki"});
            let worker = async {
                let task = state
                    .store
                    .pop_for_b("synthetic-b", "mock", Duration::from_secs(1))
                    .await
                    .unwrap();
                assert_eq!(task.task_type, "agree");
                assert_eq!(task.payload, body);
                state
                    .store
                    .complete_task(&task.task_id, result.clone(), "synthetic-b")
                    .await
                    .unwrap();
            };
            let (response, ()) = tokio::join!(
                agree(
                    State(state.clone()),
                    agreement_headers(),
                    Json(body.clone())
                ),
                worker
            );
            assert_eq!(
                response.status(),
                if result.get("error").is_some() {
                    StatusCode::INTERNAL_SERVER_ERROR
                } else {
                    StatusCode::OK
                }
            );
            let bytes = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let response: Value = serde_json::from_slice(&bytes).unwrap();
            if let Some(code) = result.get("keymint_error_code") {
                assert_eq!(&response["keymint_error_code"], code);
            } else {
                assert_eq!(response["data"], result["data"]);
            }
            assert_eq!(state.store.counts().await.pending, 0);
        }
    }

    #[tokio::test]
    async fn agreement_keybox_and_auth_failure_never_enqueue_b_work() {
        let state = agreement_state(true);
        let body = json!({"device_id": "synthetic-b", "alias": "key", "peer_public_key": "synthetic-spki"});
        let denied = agree(State(state.clone()), HeaderMap::new(), Json(body.clone())).await;
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        let response = agree(State(state.clone()), agreement_headers(), Json(body)).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let response: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response["keymint_error_code"], -100);
        assert!(state.store.list_tasks(10).await.is_empty());
    }

    #[tokio::test]
    async fn hal_failure_keeps_http_status_and_numeric_code() {
        let result = json!({ "error": "finish rejected", "keymint_error_code": -30 });
        let response = task_error_response("task failed", keymint_error_code(&result));
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            body,
            json!({ "error": "task failed", "keymint_error_code": -30 })
        );
    }

    #[test]
    fn legacy_and_malformed_results_have_no_numeric_hal_code() {
        for result in [
            json!({ "error": "legacy [km_error=-30]" }),
            json!({ "signature": "AQ==", "keymint_error_code": -30 }),
            json!({ "error": "bad", "keymint_error_code": 0 }),
            json!({ "error": "bad", "keymint_error_code": 1 }),
            json!({ "error": "bad", "keymint_error_code": "-30" }),
            json!({ "error": "bad", "keymint_error_code": -2147483649_i64 }),
        ] {
            assert!(keymint_error_code(&result).is_none());
        }
    }

    #[test]
    fn validates_remote_profile_result_shape() {
        assert!(result_shape_valid(
            "profile",
            &json!({
                "interface_version": 2,
                "interface_hash": "207c9f218b9b9e4e74ff5232eb16511eca9d7d2e",
                "profile_version": 200,
                "hardware_version": 400,
                "security_level": 1,
                "keymint_name": "Keymint HAL: 4",
                "keymint_author": "Qualcomm",
                "has_strongbox": false,
            })
        ));
        assert!(!result_shape_valid(
            "profile",
            &json!({ "hardware_version": 200 })
        ));

        for field in ["keymint_name", "keymint_author"] {
            let mut profile = json!({
                "interface_version": 2,
                "interface_hash": "207c9f218b9b9e4e74ff5232eb16511eca9d7d2e",
                "profile_version": 200,
                "hardware_version": 400,
                "security_level": 1,
                "keymint_name": "Keymint HAL: 4",
                "keymint_author": "Qualcomm",
                "has_strongbox": false,
            });
            profile.as_object_mut().unwrap().remove(field);
            assert!(!result_shape_valid("profile", &profile));
        }

        let mut profile = json!({
            "interface_version": 2,
            "interface_hash": "207c9f218b9b9e4e74ff5232eb16511eca9d7d2e",
            "profile_version": 200,
            "hardware_version": 400,
            "security_level": 1,
            "keymint_name": "Keymint HAL: 4",
            "keymint_author": "",
            "has_strongbox": false,
        });
        assert!(result_shape_valid("profile", &profile));

        profile["keymint_author"] = json!("   ");
        assert!(!result_shape_valid("profile", &profile));
        profile["keymint_author"] = Value::Null;
        assert!(!result_shape_valid("profile", &profile));
        profile["keymint_author"] = json!(42);
        assert!(!result_shape_valid("profile", &profile));
        profile["keymint_author"] = json!("");
        profile["keymint_name"] = json!("   ");
        assert!(!result_shape_valid("profile", &profile));
    }

    #[test]
    fn validates_agree_result_shape_without_confusing_errors_for_success() {
        use base64::Engine as _;
        for size in [28, 32, 48, 66] {
            assert!(result_shape_valid(
                "agree",
                &json!({
                    "data": base64::engine::general_purpose::STANDARD.encode(vec![0; size])
                })
            ));
        }
        for data in ["", "AQID", "?"] {
            assert!(!result_shape_valid("agree", &json!({"data": data})));
        }
        assert!(!result_shape_valid(
            "agree",
            &json!({
                "data": base64::engine::general_purpose::STANDARD.encode([0; 32]),
                "error": "failed", "keymint_error_code": -38
            })
        ));
        assert!(!result_shape_valid(
            "agree",
            &json!({ "signature": "AQID" })
        ));
        assert!(!result_shape_valid("agree", &json!({ "data": 7 })));
    }

    #[test]
    fn strongbox_demotion_marks_effective_tee_level() {
        let mut result = json!({ "cert_chain": ["leaf"] });

        assert!(mark_strongbox_demotion(&mut result));
        assert_eq!(result["strongbox_demoted"], json!(true));
        assert_eq!(result["effective_security_level"], json!(1));
    }

    #[test]
    fn strongbox_demotion_rejects_non_object_result() {
        let mut result = json!(["leaf"]);

        assert!(!mark_strongbox_demotion(&mut result));
    }
}
