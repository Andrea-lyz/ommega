//! Token authentication and simple in-memory rate limiting.
//!
//! Mirrors Django's `_check_token` + `_db_token_valid`:
//!   1. static env token (RELAY_TOKEN) matches exactly -> allowed
//!   2. otherwise, a DB-issued "card" token (ApiToken) is checked:
//!        - role must match the endpoint's expected role (a/b), if any
//!        - activated on first use (activation-based expiry)
//!        - `expires_at = activated_at + duration_seconds`
//!   3. if neither env token nor any DB token is configured, allow (back-compat)
//!
//! A database error is never an answer. `check_token` reports
//! [`TokenCheck::Unavailable`] so the caller can return 503 instead of 401: a
//! MySQL blip must not tell a client that its token is invalid, and must not
//! count against the failed-auth budget that exists to stop credential probing.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::db::Db;

#[derive(Clone)]
pub struct AuthState {
    pub relay_token: String,
    pub admin_user: String,
    pub admin_password: String,
    pub admin_extra: String,
    db: Option<Arc<Db>>,
    rate: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    relay_rate: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    invalid_rate: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    login_rate: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    sessions: Arc<Mutex<HashMap<String, Instant>>>,
    /// Last time `record_token_use` wrote a row, keyed by token + IP.
    token_use_seen: Arc<Mutex<HashMap<String, Instant>>>,
    pub rate_limit_requests: u64,
    /// Limit for the relay data plane (A attest/sign/decrypt/agree/profile and
    /// B poll/result). These carry real work for an already-authenticated
    /// device, so they get their own, far larger budget than the public
    /// card/query endpoints.
    pub relay_rate_limit_requests: u64,
    pub invalid_rate_limit_requests: u64,
    pub rate_limit_window: Duration,
    // IP allow/deny list (applies to A/B-side endpoints only).
    ip_filter_enabled: Arc<AtomicBool>,
    /// true = whitelist mode (only listed IPs allowed), false = blacklist mode.
    ip_filter_whitelist: Arc<AtomicBool>,
    ip_filter_list: Arc<Mutex<HashSet<String>>>,
    /// When true (default), a valid token is always required. When false,
    /// anonymous access is allowed when neither a static token nor any DB
    /// token is configured (backward-compatibility fallback).
    pub auth_required: bool,
}

/// Admin session TTL: 12 hours of inactivity.
const SESSION_TTL: Duration = Duration::from_secs(12 * 3600);

/// Default budget for relay data-plane requests per token per window.
///
/// A B device long-polls every 20s (180/h idle) and spends one poll plus one
/// result per task, so a busy hour runs into the thousands. The old shared
/// 800/h budget throttled ordinary operation and, because a throttled
/// `b/result` discards a finished task, surfaced on the A side as an
/// attestation failure. This is a runaway-loop backstop, not a quota.
const DEFAULT_RELAY_RATE_LIMIT_REQUESTS: u64 = 20_000;

/// Separator for the token + IP throttle key: a control character that
/// cannot occur in either half.
const TOKEN_USE_KEY_SEP: char = '\u{1f}';

/// Minimum spacing between `token_usage_log` writes for one token + IP pair.
const TOKEN_USE_WRITE_INTERVAL: Duration = Duration::from_secs(60);

/// Outcome of authenticating a request.
///
/// `Invalid` is a verdict about the credential; `Unavailable` means the server
/// could not reach the store that holds the verdict. Collapsing the two into
/// `false` is what let a MySQL outage present as "missing or invalid
/// X-Relay-Token" and, through the failed-auth limiter, escalate into an
/// hour-long IP block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenCheck {
    Valid,
    Invalid,
    Unavailable(String),
}

impl AuthState {
    pub fn new(
        relay_token: String,
        rate_limit_requests: u64,
        rate_limit_window_secs: u64,
        auth_required: bool,
    ) -> Self {
        Self {
            relay_token,
            admin_user: String::new(),
            admin_password: String::new(),
            admin_extra: String::new(),
            db: None,
            rate: Arc::new(Mutex::new(HashMap::new())),
            relay_rate: Arc::new(Mutex::new(HashMap::new())),
            invalid_rate: Arc::new(Mutex::new(HashMap::new())),
            login_rate: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            token_use_seen: Arc::new(Mutex::new(HashMap::new())),
            rate_limit_requests,
            relay_rate_limit_requests: DEFAULT_RELAY_RATE_LIMIT_REQUESTS,
            // Invalid (failed-auth) requests get a much tighter limit.
            invalid_rate_limit_requests: 40,
            rate_limit_window: Duration::from_secs(rate_limit_window_secs),
            ip_filter_enabled: Arc::new(AtomicBool::new(false)),
            ip_filter_whitelist: Arc::new(AtomicBool::new(false)),
            ip_filter_list: Arc::new(Mutex::new(HashSet::new())),
            auth_required,
        }
    }

    /// Attach the DB handle (set once after `Db::open`).
    pub fn with_db(mut self, db: Option<Arc<Db>>) -> Self {
        self.db = db;
        self
    }

    /// Set admin credentials (user/password/extra accounts).
    pub fn with_admin_credentials(
        mut self,
        user: &str,
        password: &str,
        extra: &str,
    ) -> Self {
        self.admin_user = user.to_string();
        self.admin_password = password.to_string();
        self.admin_extra = extra.to_string();
        self
    }

    /// Verify admin credentials. Returns true when user/password match either
    /// the primary account or one of the `user:pass` entries in `admin_extra`.
    pub fn verify_admin(&self, user: &str, password: &str) -> bool {
        if self.admin_user.is_empty() {
            return false;
        }
        if user == self.admin_user && password == self.admin_password {
            return true;
        }
        for entry in self.admin_extra.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            if let Some((u, p)) = entry.split_once(':') {
                if user == u.trim() && password == p.trim() {
                    return true;
                }
            }
        }
        false
    }

    /// Create a new admin session, returning the session id (opaque token).
    pub fn create_session(&self) -> String {
        use base64::Engine;
        use rand::RngCore;
        let mut rng = rand::rngs::OsRng;
        let mut b = [0u8; 32];
        rng.fill_bytes(&mut b);
        let sid = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(b)
            .to_string();
        let mut sessions = crate::util::mu(&self.sessions);
        // Opportunistically prune expired sessions.
        let now = Instant::now();
        sessions.retain(|_, exp| now.duration_since(*exp) < SESSION_TTL);
        sessions.insert(sid.clone(), now);
        sid
    }

    /// Check whether a session id is valid (and refresh its TTL).
    pub fn check_session(&self, sid: &str) -> bool {
        if sid.is_empty() {
            return false;
        }
        let mut sessions = crate::util::mu(&self.sessions);
        let now = Instant::now();
        sessions.retain(|_, exp| now.duration_since(*exp) < SESSION_TTL);
        match sessions.get_mut(sid) {
            Some(exp) => {
                *exp = now;
                true
            }
            None => false,
        }
    }

    /// Invalidate a session id.
    pub fn drop_session(&self, sid: &str) {
        let mut sessions = crate::util::mu(&self.sessions);
        sessions.remove(sid);
    }

    // ---- IP allow/deny list (A/B-side only) ----

    pub fn ip_filter_enabled(&self) -> bool {
        self.ip_filter_enabled.load(Ordering::Relaxed)
    }

    pub fn ip_filter_is_whitelist(&self) -> bool {
        self.ip_filter_whitelist.load(Ordering::Relaxed)
    }

    pub fn set_ip_filter_enabled(&self, v: bool) {
        self.ip_filter_enabled.store(v, Ordering::Relaxed);
    }

    pub fn set_ip_filter_whitelist(&self, v: bool) {
        self.ip_filter_whitelist.store(v, Ordering::Relaxed);
    }

    pub fn ip_filter_list(&self) -> Vec<String> {
        let mut list: Vec<String> = crate::util::mu(&self.ip_filter_list).iter().cloned().collect();
        list.sort();
        list
    }

    pub fn add_ip(&self, ip: &str) -> bool {
        let ip = ip.trim();
        if ip.is_empty() {
            return false;
        }
        crate::util::mu(&self.ip_filter_list).insert(ip.to_string())
    }

    pub fn remove_ip(&self, ip: &str) -> bool {
        crate::util::mu(&self.ip_filter_list).remove(ip)
    }

    /// Whether the given client IP is allowed through the filter. Returns true
    /// when the filter is disabled, or when the IP passes the current mode.
    pub fn ip_allowed(&self, ip: &str) -> bool {
        if !self.ip_filter_enabled.load(Ordering::Relaxed) {
            return true;
        }
        let list = crate::util::mu(&self.ip_filter_list);
        let contained = list.contains(ip);
        if self.ip_filter_whitelist.load(Ordering::Relaxed) {
            contained // whitelist: only listed IPs allowed
        } else {
            !contained // blacklist: listed IPs denied
        }
    }

    /// Set the invalid (failed-auth) rate limit.
    pub fn with_invalid_rate_limit(mut self, limit: u64) -> Self {
        self.invalid_rate_limit_requests = limit;
        self
    }

    /// Returns true if the header value matches the configured static token.
    pub fn check_static_token(&self, header: Option<&str>) -> bool {
        if self.relay_token.is_empty() {
            return false;
        }
        match header {
            Some(v) => v == self.relay_token,
            None => false,
        }
    }

    /// Full auth check for an endpoint with an optional required role.
    ///
    /// `role`: `Some("a")` for A-side endpoints, `Some("b")` for B-side,
    /// `None` for role-agnostic endpoints (ping/health). Also activates the
    /// token on first use and records the client IP.
    ///
    /// Returns [`TokenCheck::Unavailable`] when the token store cannot be
    /// reached. Callers must map that to 503, never to 401.
    pub fn check_token(&self, header: Option<&str>, role: Option<&str>, ip: &str) -> TokenCheck {
        let token = header.unwrap_or("");

        // 1) static env token (back-compat)
        if self.check_static_token(Some(token)) {
            return TokenCheck::Valid;
        }

        // 2) DB card token
        if let Some(db) = &self.db {
            if !token.is_empty() {
                match db.get_api_token(token) {
                    Ok(Some(row)) => {
                        // role check
                        if let Some(required) = role {
                            if row.role != required {
                                return TokenCheck::Invalid;
                            }
                        }
                        // activated / expiry (activation-based)
                        if !row.enabled {
                            return TokenCheck::Invalid;
                        }
                        // activate on first use
                        if row.activated_at.is_empty() {
                            if let Err(e) = db.activate_token_if_needed(token) {
                                return TokenCheck::Unavailable(format!(
                                    "token activation failed: {e}"
                                ));
                            }
                        }
                        // Record usage (IP + last-used). Throttled per token+IP:
                        // this is three statements in a transaction, and running
                        // it on every poll made the relay its own DB load. A
                        // write failure is not fatal here, because the
                        // credential is already known good, so it only warns.
                        if self.should_record_token_use(token, ip) {
                            if let Err(e) = db.record_token_use(token, ip) {
                                tracing::warn!(
                                    "check_token: record_token_use failed token={} ip={} err={e}",
                                    token,
                                    ip
                                );
                            }
                        }
                        // Expiry is computed from the row already in hand. The
                        // previous code re-queried, which doubled the per-request
                        // DB cost and added a second path where an error became
                        // "invalid token".
                        if token_row_is_valid(&row) {
                            return TokenCheck::Valid;
                        }
                        return TokenCheck::Invalid;
                    }
                    Ok(None) => { /* unknown token: fall through */ }
                    Err(e) => {
                        return TokenCheck::Unavailable(format!("token lookup failed: {e}"));
                    }
                }
            }
        }

        // 3) anonymous fallback: only allowed when auth_required is explicitly
        //    disabled AND no static token AND no DB token is configured.
        if !self.auth_required && self.relay_token.is_empty() {
            let has_db_token = match self.db.as_ref().map(|db| db.has_any_token()) {
                Some(Ok(has)) => has,
                Some(Err(e)) => {
                    return TokenCheck::Unavailable(format!("token inventory failed: {e}"));
                }
                None => false,
            };
            if !has_db_token {
                return TokenCheck::Valid;
            }
        }

        TokenCheck::Invalid
    }

    /// True when enough time has passed to write another `token_usage_log` row
    /// for this token and IP. A previously unseen IP always writes immediately
    /// so the admin IP history stays complete.
    fn should_record_token_use(&self, token: &str, ip: &str) -> bool {
        let key = format!("{token}{TOKEN_USE_KEY_SEP}{ip}");
        let now = Instant::now();
        let mut map = crate::util::mu(&self.token_use_seen);
        match map.get(&key) {
            Some(last) if now.duration_since(*last) < TOKEN_USE_WRITE_INTERVAL => false,
            _ => {
                map.insert(key, now);
                // Bound the map so a long-lived process cannot grow it without
                // limit across many tokens and client addresses.
                if map.len() > 4096 {
                    map.retain(|_, last| now.duration_since(*last) < TOKEN_USE_WRITE_INTERVAL * 10);
                }
                true
            }
        }
    }

    /// Set the relay data-plane rate limit (0 disables it).
    pub fn with_relay_rate_limit(mut self, limit: u64) -> Self {
        self.relay_rate_limit_requests = limit;
        self
    }

    /// Sliding-window rate limit keyed by token (or client IP when no token).
    /// Returns true if the request is allowed.
    pub fn allow(&self, key: &str) -> bool {
        self.allow_with_limit(&self.rate, key, self.rate_limit_requests)
    }

    /// Rate limit for the relay data plane, on its own budget and its own
    /// bucket so attestation traffic and public card queries cannot starve
    /// each other. A limit of 0 disables it.
    pub fn allow_relay(&self, key: &str) -> bool {
        if self.relay_rate_limit_requests == 0 {
            return true;
        }
        self.allow_with_limit(&self.relay_rate, key, self.relay_rate_limit_requests)
    }

    /// Seconds until the oldest request in this bucket leaves the window, i.e.
    /// the earliest moment a throttled caller can succeed. Always at least 1 so
    /// a `Retry-After: 0` never invites an immediate retry.
    pub fn retry_after_secs(&self, key: &str, relay: bool) -> u64 {
        let bucket = if relay { &self.relay_rate } else { &self.rate };
        let map = crate::util::mu(bucket);
        let Some(entries) = map.get(key) else {
            return 1;
        };
        let now = Instant::now();
        entries
            .iter()
            .map(|t| {
                self.rate_limit_window
                    .saturating_sub(now.duration_since(*t))
                    .as_secs()
            })
            .min()
            .unwrap_or(0)
            .max(1)
    }

    /// Sliding-window rate limit for invalid (failed-auth) requests, keyed by
    /// client IP. Uses a much tighter limit than valid requests.
    pub fn allow_invalid(&self, key: &str) -> bool {
        self.allow_with_limit(&self.invalid_rate, key, self.invalid_rate_limit_requests)
    }

    /// Sliding-window rate limit for admin login attempts, keyed by username+IP.
    /// Returns true if the attempt is allowed. Limits to 5 attempts per minute
    /// (independent of the general request rate limit).
    pub fn allow_login_attempt(&self, key: &str) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs(60);
        let mut map = crate::util::mu(&self.login_rate);
        let bucket = map.entry(key.to_string()).or_default();
        bucket.retain(|t| now.duration_since(*t) < window);
        if bucket.len() as u64 >= 5 {
            false
        } else {
            bucket.push(now);
            true
        }
    }

    fn allow_with_limit(
        &self,
        rate: &Arc<Mutex<HashMap<String, Vec<Instant>>>>,
        key: &str,
        limit: u64,
    ) -> bool {
        let now = Instant::now();
        let mut map = crate::util::mu(rate);
        let bucket = map.entry(key.to_string()).or_default();
        bucket.retain(|t| now.duration_since(*t) < self.rate_limit_window);
        if bucket.len() as u64 >= limit {
            false
        } else {
            bucket.push(now);
            true
        }
    }
}

/// Check expiry of an already-activated token. Returns true if not expired.
/// A token with no `activated_at` is treated as not-yet-activated (valid),
/// which also covers the row this request just activated.
///
/// Pure: it reads the row the caller already fetched instead of issuing a
/// second query whose failure would have to be guessed at.
fn token_row_is_valid(row: &crate::db::ApiTokenRow) -> bool {
    if row.activated_at.is_empty() {
        return true; // not activated yet
    }
    // activated_at is stored as `YYYY-MM-DD HH:MM:SS` in Beijing time
    // (MySQL session time_zone = +08:00). Parse it and add duration_seconds.
    let act = match parse_beijing_datetime(&row.activated_at) {
        Some(t) => t,
        None => return false,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    now <= act + row.duration_seconds
}

/// Parse `YYYY-MM-DD HH:MM:SS` (Beijing time, +08:00) into a unix timestamp
/// (seconds). MySQL DATETIME columns are stored in local Beijing time.
fn parse_beijing_datetime(s: &str) -> Option<i64> {
    // Strip fractional seconds if present (e.g. `2026-08-14 12:00:00.123456`).
    let s = s.trim();
    let s = if let Some(dot) = s.find('.') {
        &s[..dot]
    } else {
        s
    };
    let parts: Vec<&str> = s.split([' ', ':', '-']).collect();
    if parts.len() < 6 {
        return None;
    }
    use chrono::TimeZone;
    let y: i32 = parts[0].parse().ok()?;
    let mo: u32 = parts[1].parse().ok()?;
    let d: u32 = parts[2].parse().ok()?;
    let h: u32 = parts[3].parse().ok()?;
    let mi: u32 = parts[4].parse().ok()?;
    let sec: u32 = parts[5].parse().ok()?;
    let offset = chrono::FixedOffset::east_opt(8 * 3600)?;
    let dt = offset.with_ymd_and_hms(y, mo, d, h, mi, sec).single()?;
    Some(dt.timestamp())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ApiTokenRow;

    fn row(activated_at: &str, duration_seconds: i64) -> ApiTokenRow {
        ApiTokenRow {
            id: 1,
            token: "t".into(),
            role: "a".into(),
            duration_seconds,
            note: String::new(),
            enabled: true,
            activated_at: activated_at.into(),
            created_at: String::new(),
            last_ip: String::new(),
            last_used_at: String::new(),
        }
    }

    /// A database that cannot be reached. Every query fails, which is exactly
    /// the outage the three-state check exists to distinguish from a bad token.
    fn unreachable_db() -> Arc<Db> {
        Arc::new(Db::open_lazy("mysql://ommega@127.0.0.1:1/unreachable"))
    }

    #[test]
    fn not_yet_activated_tokens_are_valid_and_expiry_uses_the_fetched_row() {
        assert!(token_row_is_valid(&row("", 60)));
        assert!(!token_row_is_valid(&row("2000-01-01 00:00:00", 60)));
        assert!(token_row_is_valid(&row("2999-01-01 00:00:00", 60)));
        // An unparseable timestamp is a real data problem, not an outage.
        assert!(!token_row_is_valid(&row("not-a-date", 60)));
    }

    #[test]
    fn a_dead_database_is_unavailable_rather_than_invalid() {
        let auth = AuthState::new(String::new(), 100, 60, true).with_db(Some(unreachable_db()));
        let check = auth.check_token(Some("some-card-token"), Some("a"), "10.0.0.1");
        assert!(
            matches!(check, TokenCheck::Unavailable(_)),
            "a MySQL outage must not read as a bad credential, got {check:?}"
        );
    }

    #[test]
    fn the_static_token_still_works_while_the_database_is_down() {
        let auth =
            AuthState::new("static-token".into(), 100, 60, true).with_db(Some(unreachable_db()));
        assert_eq!(
            auth.check_token(Some("static-token"), Some("a"), "10.0.0.1"),
            TokenCheck::Valid
        );
        // A wrong token with the DB down is still unknown, not invalid.
        assert!(matches!(
            auth.check_token(Some("wrong"), Some("a"), "10.0.0.1"),
            TokenCheck::Unavailable(_)
        ));
    }

    #[test]
    fn a_missing_token_is_invalid_without_a_database() {
        let auth = AuthState::new("static-token".into(), 100, 60, true);
        assert_eq!(
            auth.check_token(None, Some("a"), "10.0.0.1"),
            TokenCheck::Invalid
        );
    }

    #[test]
    fn the_relay_budget_is_separate_from_the_standard_one() {
        let auth = AuthState::new("t".into(), 1, 60, true).with_relay_rate_limit(3);
        assert!(auth.allow("token"));
        assert!(!auth.allow("token"), "standard budget is 1");
        // Spending the standard budget must not spend the relay budget.
        assert!(auth.allow_relay("token"));
        assert!(auth.allow_relay("token"));
        assert!(auth.allow_relay("token"));
        assert!(!auth.allow_relay("token"), "relay budget is 3");
    }

    #[test]
    fn a_zero_relay_limit_disables_relay_throttling() {
        let auth = AuthState::new("t".into(), 1, 60, true).with_relay_rate_limit(0);
        for _ in 0..1000 {
            assert!(auth.allow_relay("token"));
        }
    }

    #[test]
    fn retry_after_is_never_zero_and_never_exceeds_the_window() {
        let auth = AuthState::new("t".into(), 1, 60, true).with_relay_rate_limit(1);
        assert_eq!(auth.retry_after_secs("cold", true), 1);
        assert!(auth.allow_relay("token"));
        let wait = auth.retry_after_secs("token", true);
        assert!((1..=60).contains(&wait), "unexpected retry-after {wait}");
    }

    #[test]
    fn token_usage_writes_are_throttled_per_token_and_ip() {
        let auth = AuthState::new("t".into(), 100, 60, true);
        assert!(auth.should_record_token_use("tok", "1.1.1.1"));
        assert!(
            !auth.should_record_token_use("tok", "1.1.1.1"),
            "a second poll within the interval must not write again"
        );
        assert!(
            auth.should_record_token_use("tok", "2.2.2.2"),
            "a new client address must always be recorded"
        );
    }
}
