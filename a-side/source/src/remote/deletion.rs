//! Persistent remote-blob reference registry and the asynchronous remote
//! session deletion worker.
//!
//! keystore2 calls `deleteKey(blob)` when a key (or a superseded upgrade blob)
//! is collected. The TA cannot decrypt the blob to recover the remote alias,
//! so the TA instead reports the SHA-256 digest of every serialized remote
//! keyblob it emits or sees (`register`) and of every blob passed to
//! `deleteKey` (`release`). A remote session is only scheduled for deletion
//! once the *last* blob digest referencing an alias is released, which keeps a
//! superseded upgrade blob from tearing down the still-live remote session.
//!
//! Pending deletions live in SQLite so they survive process restarts; a
//! `ommega-remote-gc` worker drains them with exponential backoff and drops
//! entries older than 30 days. Every failure mode fails toward not deleting:
//! registry errors are logged and swallowed, and a row is only removed after
//! the relay confirms the delete, after the request was permanently rejected,
//! or after the row expires.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use log::{info, warn};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

/// Name of the persistent registry database, inside the standard product data
/// directory.
const DATABASE_NAME: &str = "remote-blobs.db";
/// Name of the deletion worker thread (visible in `ps -T`).
const WORKER_THREAD_NAME: &str = "ommega-remote-gc";
/// Periodic wake-up for the worker; also caps the wait until the next due
/// retry.
const WAKE_INTERVAL: Duration = Duration::from_secs(60);
/// Shortest sleep while deletion is enabled, so a due row cannot spin the worker.
const MIN_WAIT: Duration = Duration::from_secs(1);
/// Base of the exponential retry backoff.
const BASE_BACKOFF_MS: i64 = 60_000;
/// Maximum backoff between remote deletion attempts.
const MAX_BACKOFF_MS: i64 = 6 * 60 * 60 * 1000;
/// How long a pending deletion is retried before it is dropped. The remote
/// session stays on B afterwards; failing toward not deleting is safer than a
/// stale delete against a recreated alias.
const PENDING_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;

fn database_path() -> std::path::PathBuf {
    crate::global::db_root_path().join(DATABASE_NAME)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Delay before the next attempt after `attempts` recorded failures
/// (`attempts >= 1`): `min(60s * 2^attempts, 6h)`.
fn retry_delay_ms(attempts: u32) -> i64 {
    BASE_BACKOFF_MS
        .saturating_mul(1i64 << attempts.min(20))
        .min(MAX_BACKOFF_MS)
}

/// Outcome of one remote deletion attempt, classified so the worker knows
/// whether the pending row can be dropped, retried, or abandoned.
#[derive(Debug, PartialEq, Eq)]
enum DeleteOutcome {
    /// The relay answered with a `deleted` flag; either way the pending row
    /// has been served.
    Done { deleted: bool },
    /// The request itself was rejected (validation); retrying the identical
    /// request cannot succeed, so the row is dropped without deleting.
    Permanent(String),
    /// Any other failure: transport, unavailable relay, an old server or
    /// B relay that does not know the task type, or a malformed response.
    Retry(String),
}

/// Performs the actual remote delete. Behind a trait so the worker loop is
/// testable without the network.
trait SessionDeleter: Send + Sync {
    fn delete_session(&self, alias: &str, device_id: &str) -> DeleteOutcome;
}

/// Sends the delete through the ordinary relay HTTP client.
struct RelaySessionDeleter;

impl SessionDeleter for RelaySessionDeleter {
    fn delete_session(&self, alias: &str, device_id: &str) -> DeleteOutcome {
        classify_response(&super::RemoteRelay::delete_session(alias, device_id))
    }
}

/// Classify a `RemoteRelay::delete_session` result into a [`DeleteOutcome`].
/// Kept separate so it is unit-testable without the network.
fn classify_response(response: &Result<Option<Value>>) -> DeleteOutcome {
    match response {
        Ok(Some(value)) => match value.get("deleted").and_then(Value::as_bool) {
            Some(deleted) => DeleteOutcome::Done { deleted },
            None => {
                DeleteOutcome::Retry(format!("delete response missing 'deleted' flag: {value}"))
            }
        },
        Ok(None) => DeleteOutcome::Retry("remote delete transport unavailable".into()),
        Err(e) => {
            let text = format!("{e:#}");
            // HTTP 400 means the server validated and rejected the request
            // shape; resending the identical request can never succeed.
            if text.contains("HTTP 400") {
                DeleteOutcome::Permanent(text)
            } else {
                DeleteOutcome::Retry(text)
            }
        }
    }
}

/// One row of `pending_deletes`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingDelete {
    alias: String,
    device_id: String,
    enqueued_ms: i64,
    attempts: u32,
    next_attempt_ms: i64,
}

fn pending_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingDelete> {
    Ok(PendingDelete {
        alias: row.get(0)?,
        device_id: row.get(1)?,
        enqueued_ms: row.get(2)?,
        attempts: row.get(3)?,
        next_attempt_ms: row.get(4)?,
    })
}

/// The SQLite-backed digest -> (alias, device_id) registry plus the durable
/// pending-delete queue. All methods are local-only and never block on the
/// network.
struct Registry {
    conn: Connection,
    /// Digests known to have a `blob_refs` row. Skips the DB write on repeat
    /// `begin()` registrations; kept in sync by `register`/`release`.
    known: HashSet<[u8; 32]>,
}

impl Registry {
    fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_millis(250))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS blob_refs(
                digest BLOB PRIMARY KEY NOT NULL,
                alias TEXT NOT NULL,
                device_id TEXT NOT NULL,
                created_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS blob_refs_alias ON blob_refs(alias);
            CREATE TABLE IF NOT EXISTS pending_deletes(
                alias TEXT NOT NULL,
                device_id TEXT NOT NULL,
                enqueued_ms INTEGER NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                next_attempt_ms INTEGER NOT NULL,
                last_error TEXT,
                PRIMARY KEY(alias, device_id)
            );",
        )?;
        Ok(Self {
            conn,
            known: HashSet::new(),
        })
    }

    /// Record `digest -> (alias, device_id)` and cancel any pending deletion
    /// for that alias/device (a live reference always wins).
    fn register(
        &mut self,
        alias: &str,
        device_id: &str,
        digest: &[u8; 32],
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        if self.known.contains(digest) {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO blob_refs(digest, alias, device_id, created_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![digest.as_slice(), alias, device_id, now_ms],
        )?;
        tx.execute(
            "DELETE FROM pending_deletes WHERE alias = ?1 AND device_id = ?2",
            params![alias, device_id],
        )?;
        tx.commit()?;
        self.known.insert(*digest);
        Ok(())
    }

    /// Remove the `digest` reference. Returns `true` when this was the last
    /// reference to its alias and a pending deletion was queued.
    fn release(&mut self, digest: &[u8; 32], now_ms: i64) -> rusqlite::Result<bool> {
        self.known.remove(digest);
        let tx = self.conn.transaction()?;
        let removed: Option<(String, String)> = tx
            .query_row(
                "SELECT alias, device_id FROM blob_refs WHERE digest = ?1",
                params![digest.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((alias, device_id)) = removed else {
            // Unknown digest: the blob was never registered as remote (local
            // key, or the registry never saw it). Nothing to do.
            return Ok(false);
        };
        tx.execute(
            "DELETE FROM blob_refs WHERE digest = ?1",
            params![digest.as_slice()],
        )?;
        let remaining: i64 = tx.query_row(
            "SELECT COUNT(*) FROM blob_refs WHERE alias = ?1 AND device_id = ?2",
            params![alias, device_id],
            |row| row.get(0),
        )?;
        let queued = remaining == 0;
        if queued {
            tx.execute(
                "INSERT INTO pending_deletes(alias, device_id, enqueued_ms, attempts, next_attempt_ms, last_error)
                 VALUES (?1, ?2, ?3, 0, ?3, NULL)
                 ON CONFLICT(alias, device_id) DO UPDATE SET
                     enqueued_ms = excluded.enqueued_ms,
                     attempts = 0,
                     next_attempt_ms = excluded.next_attempt_ms,
                     last_error = NULL",
                params![alias, device_id, now_ms],
            )?;
        }
        tx.commit()?;
        Ok(queued)
    }

    /// Device id an alias is already known under, whether through a live blob
    /// reference or a deletion that is still pending.
    fn device_for_alias(&self, alias: &str) -> rusqlite::Result<Option<String>> {
        if let Some(device_id) = self
            .conn
            .query_row(
                "SELECT device_id FROM blob_refs WHERE alias = ?1 LIMIT 1",
                params![alias],
                |row| row.get(0),
            )
            .optional()?
        {
            return Ok(Some(device_id));
        }
        self.conn
            .query_row(
                "SELECT device_id FROM pending_deletes WHERE alias = ?1 ORDER BY enqueued_ms DESC LIMIT 1",
                params![alias],
                |row| row.get(0),
            )
            .optional()
    }

    /// The next row the worker should handle: prefer an expired row (even if
    /// it is not due yet) so it can be dropped, else the earliest due row.
    /// The bool flag marks the expired-row case.
    fn next_work(&self, now_ms: i64) -> rusqlite::Result<Option<(PendingDelete, bool)>> {
        let expired: Option<PendingDelete> = self
            .conn
            .query_row(
                "SELECT alias, device_id, enqueued_ms, attempts, next_attempt_ms
                 FROM pending_deletes WHERE enqueued_ms <= ?1
                 ORDER BY enqueued_ms LIMIT 1",
                params![now_ms.saturating_sub(PENDING_TTL_MS)],
                pending_from_row,
            )
            .optional()?;
        if let Some(row) = expired {
            return Ok(Some((row, true)));
        }
        self.conn
            .query_row(
                "SELECT alias, device_id, enqueued_ms, attempts, next_attempt_ms
                 FROM pending_deletes WHERE next_attempt_ms <= ?1
                 ORDER BY next_attempt_ms LIMIT 1",
                params![now_ms],
                pending_from_row,
            )
            .optional()
            .map(|row| row.map(|r| (r, false)))
    }

    /// Drop a pending row after the deletion was served or the row expired.
    fn remove_pending(&self, alias: &str, device_id: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM pending_deletes WHERE alias = ?1 AND device_id = ?2",
            params![alias, device_id],
        )?;
        Ok(())
    }

    /// Record a failed attempt: bump `attempts`, back off `next_attempt_ms`,
    /// and keep the error for debugging.
    fn record_failure(
        &self,
        pending: &PendingDelete,
        now_ms: i64,
        error: &str,
    ) -> rusqlite::Result<()> {
        let attempts = pending.attempts.saturating_add(1);
        let next_attempt_ms = now_ms.saturating_add(retry_delay_ms(attempts));
        self.conn.execute(
            "UPDATE pending_deletes
             SET attempts = ?3, next_attempt_ms = ?4, last_error = ?5
             WHERE alias = ?1 AND device_id = ?2",
            params![
                pending.alias,
                pending.device_id,
                attempts,
                next_attempt_ms,
                error
            ],
        )?;
        Ok(())
    }

    /// Milliseconds until the earliest scheduled attempt (0 when one is due),
    /// or `None` when no pending work exists.
    fn next_wait_ms(&self) -> rusqlite::Result<Option<u64>> {
        let earliest: Option<i64> = self
            .conn
            .query_row(
                "SELECT MIN(next_attempt_ms) FROM pending_deletes",
                [],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(earliest.map(|ms| ms.saturating_sub(now_ms()).max(0) as u64))
    }

    #[cfg(test)]
    fn pending_rows(&self) -> rusqlite::Result<Vec<PendingDelete>> {
        let mut statement = self.conn.prepare(
            "SELECT alias, device_id, enqueued_ms, attempts, next_attempt_ms
             FROM pending_deletes ORDER BY alias, device_id",
        )?;
        let rows = statement.query_map([], pending_from_row)?;
        rows.collect()
    }
}

/// Shared registry plus the worker wake-up. The registry lives behind a
/// `Mutex` so TA-path `register`/`release` calls stay local and short; the
/// network delete runs outside the lock.
pub(crate) struct RemoteDeletion {
    inner: Mutex<Option<Registry>>,
    wake: Condvar,
    deleter: Arc<dyn SessionDeleter>,
}

impl RemoteDeletion {
    fn new(path: &Path, deleter: Arc<dyn SessionDeleter>) -> Self {
        let registry = match Registry::open(path) {
            Ok(registry) => Some(registry),
            Err(e) => {
                warn!(
                    "remote blob registry unavailable at {}: {e}",
                    path.display()
                );
                None
            }
        };
        Self {
            inner: Mutex::new(registry),
            wake: Condvar::new(),
            deleter,
        }
    }

    fn start(path: &Path) -> Arc<Self> {
        let deletion = Arc::new(Self::new(path, Arc::new(RelaySessionDeleter)));
        let worker = Arc::clone(&deletion);
        match thread::Builder::new()
            .name(WORKER_THREAD_NAME.into())
            .spawn(move || worker_loop(&worker))
        {
            Ok(_) => info!("remote key deletion worker started"),
            Err(e) => warn!("failed to start remote key deletion worker: {e}"),
        }
        deletion
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Registry>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Record that `digest` references `alias`. Registration is independent of
    /// the relay switch so an upgraded live blob keeps ownership of its session.
    fn register(&self, alias: &str, digest: &[u8; 32]) {
        let configured_device = super::RemoteRelay::device_id().ok();
        self.register_with(alias, digest, configured_device.as_deref());
    }

    /// Register with an already resolved configured device id. A device already
    /// associated with the alias wins over the configured fallback.
    fn register_with(&self, alias: &str, digest: &[u8; 32], configured_device: Option<&str>) {
        let mut guard = self.lock();
        let Some(registry) = guard.as_mut() else {
            return;
        };
        let known_device = match registry.device_for_alias(alias) {
            Ok(device_id) => device_id,
            Err(e) => {
                warn!("failed to inspect remote blob reference device: {e}");
                return;
            }
        };
        let Some(device_id) = known_device.or_else(|| configured_device.map(str::to_string)) else {
            return;
        };
        if let Err(e) = registry.register(alias, &device_id, digest, now_ms()) {
            warn!("failed to register remote blob reference: {e}");
        }
    }

    /// Release one blob reference; when it was the last one for its alias,
    /// queue the durable remote deletion and wake the worker.
    fn release(&self, digest: &[u8; 32]) {
        let queued = {
            let mut guard = self.lock();
            let Some(registry) = guard.as_mut() else {
                return;
            };
            match registry.release(digest, now_ms()) {
                Ok(queued) => queued,
                Err(e) => {
                    warn!("failed to release remote blob reference: {e}");
                    false
                }
            }
        };
        if queued {
            self.wake.notify_one();
        }
    }

    /// Drain pending rows one at a time. Network sends run without the
    /// registry lock so TA-path calls are never blocked by the relay.
    /// `enabled` reports whether the remote relay is currently configured;
    /// the worker passes the live config check, tests pass `|| true`.
    fn process_due(&self, enabled: &dyn Fn() -> bool) {
        loop {
            let work = {
                let guard = self.lock();
                let Some(registry) = guard.as_ref() else {
                    return;
                };
                match registry.next_work(now_ms()) {
                    Ok(work) => work,
                    Err(e) => {
                        warn!("remote delete queue read failed: {e}");
                        return;
                    }
                }
            };
            let Some((pending, expired)) = work else {
                break;
            };
            if expired {
                let guard = self.lock();
                if let Some(registry) = guard.as_ref() {
                    if let Err(e) = registry.remove_pending(&pending.alias, &pending.device_id) {
                        warn!("remote delete expiry drop failed: {e}");
                    }
                }
                warn!(
                    "dropping remote session delete for alias={} device={}: pending for over 30 days",
                    pending.alias, pending.device_id
                );
                continue;
            }
            if pending.alias.is_empty() || pending.device_id.is_empty() {
                let guard = self.lock();
                if let Some(registry) = guard.as_ref() {
                    let _ = registry.remove_pending(&pending.alias, &pending.device_id);
                }
                warn!(
                    "dropping malformed remote session delete for alias={} device={}",
                    pending.alias, pending.device_id
                );
                continue;
            }
            // The worker only acts while the remote relay is configured.
            if !enabled() {
                break;
            }
            let outcome = self
                .deleter
                .delete_session(&pending.alias, &pending.device_id);
            let guard = self.lock();
            let Some(registry) = guard.as_ref() else {
                return;
            };
            match outcome {
                DeleteOutcome::Done { deleted } => {
                    if let Err(e) = registry.remove_pending(&pending.alias, &pending.device_id) {
                        warn!("remote delete completion drop failed: {e}");
                    }
                    info!(
                        "remote session delete for alias={} device={} completed (existed={deleted})",
                        pending.alias, pending.device_id
                    );
                }
                DeleteOutcome::Permanent(error) => {
                    if let Err(e) = registry.remove_pending(&pending.alias, &pending.device_id) {
                        warn!("remote delete rejection drop failed: {e}");
                    }
                    warn!(
                        "remote session delete for alias={} device={} permanently rejected: {error}",
                        pending.alias, pending.device_id
                    );
                }
                DeleteOutcome::Retry(error) => {
                    let attempts = pending.attempts.saturating_add(1);
                    warn!(
                        "remote session delete for alias={} device={} failed \
                         (attempt {attempts}, retry in {}ms): {error}",
                        pending.alias,
                        pending.device_id,
                        retry_delay_ms(attempts)
                    );
                    if let Err(e) = registry.record_failure(&pending, now_ms(), &error) {
                        warn!("remote delete retry record failed: {e}");
                    }
                }
            }
        }
    }
}

/// Pick the worker sleep. A disabled relay never scans due rows, and an
/// enabled relay never waits less than one second even when work is due now.
fn worker_wait(enabled: bool, next_wait_ms: Option<u64>) -> Duration {
    if !enabled {
        return WAKE_INTERVAL;
    }
    next_wait_ms
        .map(Duration::from_millis)
        .unwrap_or(WAKE_INTERVAL)
        .clamp(MIN_WAIT, WAKE_INTERVAL)
}

fn worker_loop(deletion: &Arc<RemoteDeletion>) {
    loop {
        if super::remote_enabled() {
            deletion.process_due(&super::remote_enabled);
        }
        let enabled = super::remote_enabled();
        let guard = deletion.lock();
        let wait = worker_wait(
            enabled,
            guard
                .as_ref()
                .and_then(|registry| registry.next_wait_ms().ok().flatten()),
        );
        let _ = deletion
            .wake
            .wait_timeout(guard, wait)
            .unwrap_or_else(|e| e.into_inner());
    }
}

fn deletion() -> &'static Arc<RemoteDeletion> {
    static DELETION: OnceLock<Arc<RemoteDeletion>> = OnceLock::new();
    DELETION.get_or_init(|| RemoteDeletion::start(&database_path()))
}

/// Called once at startup: opens the registry database and spawns the
/// `ommega-remote-gc` worker thread.
pub fn spawn() {
    deletion();
}

/// `RemoteBackend::register_remote_blob` implementation entry point.
pub(crate) fn register_remote_blob(alias: &str, blob_digest: &[u8; 32]) {
    deletion().register(alias, blob_digest);
}

/// `RemoteBackend::release_remote_blob` implementation entry point.
pub(crate) fn release_remote_blob(blob_digest: &[u8; 32]) {
    deletion().release(blob_digest);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    struct ScriptedDeleter {
        outcomes: StdMutex<Vec<DeleteOutcome>>,
        calls: StdMutex<Vec<(String, String)>>,
    }

    impl ScriptedDeleter {
        fn new(outcomes: Vec<DeleteOutcome>) -> Self {
            Self {
                outcomes: StdMutex::new(outcomes),
                calls: StdMutex::new(Vec::new()),
            }
        }
    }

    impl SessionDeleter for ScriptedDeleter {
        fn delete_session(&self, alias: &str, device_id: &str) -> DeleteOutcome {
            self.calls
                .lock()
                .unwrap()
                .push((alias.to_string(), device_id.to_string()));
            self.outcomes
                .lock()
                .unwrap()
                .drain(..)
                .next()
                .unwrap_or(DeleteOutcome::Done { deleted: true })
        }
    }

    fn digest(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    fn remote_enabled_for_test() -> bool {
        true
    }

    #[test]
    fn last_release_queues_pending_and_reregister_cancels() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut registry = Registry::open(temp.path()).unwrap();
        registry.register("a", "dev-b", &digest(1), 100).unwrap();
        registry.register("a", "dev-b", &digest(2), 101).unwrap();
        assert!(
            !registry.release(&digest(1), 200).unwrap(),
            "second blob still references the alias"
        );
        assert!(registry.pending_rows().unwrap().is_empty());
        assert!(
            registry.release(&digest(2), 201).unwrap(),
            "last reference queues the remote delete"
        );
        assert_eq!(registry.pending_rows().unwrap()[0].alias, "a");
        // A new blob for the same alias cancels the pending delete.
        registry.register("a", "dev-b", &digest(3), 300).unwrap();
        assert!(registry.pending_rows().unwrap().is_empty());
        // Releasing that blob queues the delete again.
        assert!(registry.release(&digest(3), 400).unwrap());
        assert_eq!(registry.pending_rows().unwrap()[0].attempts, 0);
    }

    #[test]
    fn releasing_an_unknown_digest_is_a_noop() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut registry = Registry::open(temp.path()).unwrap();
        registry.register("a", "dev-b", &digest(1), 100).unwrap();
        assert!(!registry.release(&digest(9), 200).unwrap());
        assert!(!registry.release(&digest(9), 300).unwrap());
        // The still-registered digest keeps working afterwards.
        assert!(registry.release(&digest(1), 400).unwrap());
    }

    #[test]
    fn references_and_pending_work_survive_reopen() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        {
            let mut registry = Registry::open(temp.path()).unwrap();
            registry.register("a", "dev-b", &digest(1), 100).unwrap();
            registry.register("a", "dev-b", &digest(2), 101).unwrap();
            assert!(!registry.release(&digest(1), 150).unwrap());
        }
        {
            let mut registry = Registry::open(temp.path()).unwrap();
            assert!(registry.release(&digest(2), 200).unwrap());
            let rows = registry.pending_rows().unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].alias, "a");
            assert_eq!(rows[0].device_id, "dev-b");
        }
        {
            // The pending row persists until the worker serves it.
            let registry = Registry::open(temp.path()).unwrap();
            assert_eq!(registry.pending_rows().unwrap().len(), 1);
        }
    }

    #[test]
    fn delete_outcome_classification() {
        assert_eq!(
            classify_response(&Ok(Some(json!({"deleted": true})))),
            DeleteOutcome::Done { deleted: true }
        );
        assert_eq!(
            classify_response(&Ok(Some(json!({"deleted": false})))),
            DeleteOutcome::Done { deleted: false }
        );
        assert_eq!(
            classify_response(&Ok(None)),
            DeleteOutcome::Retry("remote delete transport unavailable".into())
        );
        match classify_response(&Ok(Some(json!({"unexpected": 1})))) {
            DeleteOutcome::Retry(_) => {}
            other => panic!("unexpected classification: {other:?}"),
        }
        match classify_response(&Err(anyhow::anyhow!(
            "remote /api/delete/ HTTP 400: bad request"
        ))) {
            DeleteOutcome::Permanent(_) => {}
            other => panic!("unexpected classification: {other:?}"),
        }
        match classify_response(&Err(anyhow::anyhow!(
            "remote /api/delete/ HTTP 404: not found"
        ))) {
            DeleteOutcome::Retry(_) => {}
            other => panic!("unexpected classification: {other:?}"),
        }
        match classify_response(&Err(anyhow::anyhow!("connection refused"))) {
            DeleteOutcome::Retry(_) => {}
            other => panic!("unexpected classification: {other:?}"),
        }
    }

    #[test]
    fn worker_retries_with_backoff_then_completes() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let path = temp.path().to_path_buf();
        {
            let mut registry = Registry::open(&path).unwrap();
            registry.register("a", "dev-b", &digest(1), 100).unwrap();
            // A real-clock enqueue time: older synthetic values would look
            // expired to `next_work`.
            assert!(registry.release(&digest(1), now_ms()).unwrap());
        }
        let deleter = Arc::new(ScriptedDeleter::new(vec![
            DeleteOutcome::Retry("B offline".into()),
            DeleteOutcome::Done { deleted: false },
        ]));
        let deletion = RemoteDeletion::new(&path, deleter);
        let before = now_ms();
        deletion.process_due(&remote_enabled_for_test);
        {
            let registry = Registry::open(&path).unwrap();
            let rows = registry.pending_rows().unwrap();
            assert_eq!(rows.len(), 1, "failed attempt keeps the pending row");
            assert_eq!(rows[0].attempts, 1);
            assert!(
                rows[0].next_attempt_ms >= before + retry_delay_ms(1),
                "first retry is backed off: next={} before={before}",
                rows[0].next_attempt_ms
            );
        }
        // Backed off: the row is not due any more, so another pass changes
        // nothing and consumes no scripted response.
        deletion.process_due(&remote_enabled_for_test);
        {
            let registry = Registry::open(&path).unwrap();
            assert_eq!(registry.pending_rows().unwrap().len(), 1);
            registry
                .conn
                .execute("UPDATE pending_deletes SET next_attempt_ms = 0", [])
                .unwrap();
        }
        // Due again: the scripted success removes the row even though the
        // remote side reported the session was already gone.
        deletion.process_due(&remote_enabled_for_test);
        {
            let registry = Registry::open(&path).unwrap();
            assert!(registry.pending_rows().unwrap().is_empty());
        }
    }

    #[test]
    fn worker_drops_permanently_rejected_and_expired_rows() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let path = temp.path().to_path_buf();
        {
            let mut registry = Registry::open(&path).unwrap();
            registry
                .register("rejected", "dev-b", &digest(1), 100)
                .unwrap();
            registry
                .register("expired", "dev-b", &digest(2), 101)
                .unwrap();
            let now = now_ms();
            assert!(registry.release(&digest(1), now).unwrap());
            assert!(registry.release(&digest(2), now).unwrap());
            // Age the second row past the 30-day TTL.
            registry
                .conn
                .execute(
                    "UPDATE pending_deletes
                     SET enqueued_ms = ?1, next_attempt_ms = ?1 + 60000
                     WHERE alias = 'expired'",
                    params![now_ms() - PENDING_TTL_MS - 1000],
                )
                .unwrap();
        }
        let deleter = Arc::new(ScriptedDeleter::new(vec![DeleteOutcome::Permanent(
            "HTTP 400".into(),
        )]));
        let deletion = RemoteDeletion::new(&path, deleter.clone());
        deletion.process_due(&remote_enabled_for_test);
        {
            let registry = Registry::open(&path).unwrap();
            assert!(
                registry.pending_rows().unwrap().is_empty(),
                "expired row dropped without a send; rejected row dropped on the permanent answer"
            );
            let calls = deleter.calls.lock().unwrap();
            assert_eq!(
                calls.as_slice(),
                &[("rejected".to_string(), "dev-b".to_string())],
                "only the live row is sent; expiry needs no network"
            );
        }
    }

    #[test]
    fn retry_delay_grows_exponentially_and_caps() {
        assert_eq!(retry_delay_ms(1), 120_000);
        assert_eq!(retry_delay_ms(2), 240_000);
        assert_eq!(retry_delay_ms(8), 15_360_000);
        assert_eq!(retry_delay_ms(9), MAX_BACKOFF_MS);
        assert_eq!(retry_delay_ms(100), MAX_BACKOFF_MS);
    }

    fn pending_rows_for(deletion: &RemoteDeletion) -> Vec<PendingDelete> {
        deletion
            .lock()
            .as_ref()
            .expect("test registry")
            .pending_rows()
            .expect("pending rows")
    }

    #[test]
    fn upgrade_while_remote_unconfigured_keeps_session() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let deletion = RemoteDeletion::new(temp.path(), Arc::new(ScriptedDeleter::new(vec![])));
        deletion.register_with("a", &digest(1), Some("dev-b"));
        deletion.register_with("a", &digest(2), None);
        deletion.release(&digest(1));
        assert!(pending_rows_for(&deletion).is_empty());
        deletion.release(&digest(2));
        let rows = pending_rows_for(&deletion);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].alias == "a" && rows[0].device_id == "dev-b");
    }

    #[test]
    fn live_blob_cancels_pending_delete_without_configured_device() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let deletion = RemoteDeletion::new(temp.path(), Arc::new(ScriptedDeleter::new(vec![])));
        deletion.register_with("a", &digest(1), Some("dev-b"));
        deletion.release(&digest(1));
        assert_eq!(pending_rows_for(&deletion).len(), 1);
        deletion.register_with("a", &digest(2), None);
        assert!(pending_rows_for(&deletion).is_empty());
        deletion.release(&digest(2));
        let rows = pending_rows_for(&deletion);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].device_id == "dev-b");
    }

    #[test]
    fn unknown_alias_without_configured_device_is_not_registered() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let deletion = RemoteDeletion::new(temp.path(), Arc::new(ScriptedDeleter::new(vec![])));
        deletion.register_with("a", &digest(1), None);
        deletion.release(&digest(1));
        assert!(pending_rows_for(&deletion).is_empty());
    }

    #[test]
    fn known_alias_device_wins_over_configured_device() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let deletion = RemoteDeletion::new(temp.path(), Arc::new(ScriptedDeleter::new(vec![])));
        deletion.register_with("a", &digest(1), Some("dev-b"));
        deletion.register_with("a", &digest(2), Some("dev-c"));
        deletion.release(&digest(1));
        assert!(pending_rows_for(&deletion).is_empty());
    }

    #[test]
    fn worker_wait_never_spins() {
        assert_eq!(worker_wait(false, Some(0)), WAKE_INTERVAL);
        assert_eq!(worker_wait(true, Some(0)), MIN_WAIT);
        assert_eq!(worker_wait(true, None), WAKE_INTERVAL);
        assert_eq!(worker_wait(true, Some(5_000)), Duration::from_secs(5));
        assert_eq!(worker_wait(true, Some(600_000)), WAKE_INTERVAL);
    }
}
