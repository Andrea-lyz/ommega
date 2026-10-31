//! In-memory task queue mirroring `relay_server/apps/relay_core/store.py`.
//!
//! A-side endpoints create tasks, wait for a B-side device to claim them
//! (`pop_for_b`), process them and report the result back (`complete_task`).
//! Timed-out assignments are reclaimed so they are not lost forever.

use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{watch, Mutex, Notify};

#[cfg(test)]
#[path = "queue/latency_tests.rs"]
mod latency_tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Assigned,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub task_id: String,
    pub task_type: String,
    pub request_id: Option<String>,
    pub payload: Value,
    pub target_device_id: String,
    pub assigned_device_id: Option<String>,
    pub assigned_at_ms: u64,
    pub result: Option<Value>,
    pub created_at_ms: u64,
    pub completed_at_ms: u64,
    pub status: TaskStatus,
    created_at: Instant,
    assigned_at: Option<Instant>,
}

impl Task {
    fn is_agreement(&self) -> bool {
        self.task_type == "agree"
            || (self.task_type == "attest"
                && self
                    .payload
                    .get("purpose")
                    .or_else(|| self.payload.get("device_attest_context")?.get("purpose"))
                    .and_then(Value::as_array)
                    .is_some_and(|purposes| purposes.iter().any(|p| p.as_i64() == Some(6))))
    }

    fn must_not_replay(&self) -> bool {
        self.is_agreement() || self.task_type == "soter"
    }

    pub fn status_str(&self) -> &'static str {
        match self.status {
            TaskStatus::Pending => "pending",
            TaskStatus::Assigned => "assigned",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceEntry {
    pub device_id: String,
    pub machine_id: String,
    pub last_seen_ms: u64,
    pub connected: bool,
    pub soter: Option<bool>,
    pub soter_sign: Option<bool>,
    pub soter_nosign: Option<bool>,
    /// Relay version from the heartbeat. `None` means an older relay.
    pub version: Option<String>,
    pub battery_percent: Option<u8>,
    pub battery_charging: Option<bool>,
    /// Server receipt time, not the relay's clock or a long-poll wake-up time.
    pub battery_updated_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeviceBattery {
    pub percent: Option<u8>,
    pub charging: Option<bool>,
}

impl DeviceBattery {
    /// Malformed optional metadata must not stop task polling or become zero.
    pub fn parse(percent: Option<&str>, charging: Option<&str>) -> Self {
        let percent = percent.map(str::trim).and_then(|raw| {
            if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            raw.parse::<u8>().ok().filter(|value| *value <= 100)
        });
        let charging = charging.and_then(|raw| match raw.trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        });
        Self { percent, charging }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceMetadata<'a> {
    pub caps: DeviceCaps,
    pub version: Option<&'a str>,
    pub battery: DeviceBattery,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeviceCaps {
    pub soter: Option<bool>,
    pub soter_sign: Option<bool>,
    pub soter_nosign: Option<bool>,
}

impl DeviceCaps {
    /// Missing caps means an old relay; an empty field explicitly reports no SOTER.
    pub fn parse(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Self::default();
        };
        let names: Vec<&str> = raw.split(',').map(str::trim).collect();
        let has = |name| names.contains(&name);
        Self {
            soter: Some(has("soter")),
            soter_sign: Some(has("soter_sign")),
            soter_nosign: Some(has("soter_nosign")),
        }
    }

    fn can_serve(self, task: &Task) -> bool {
        if task.task_type != "soter" {
            return true;
        }
        self.soter == Some(true)
            && !(soter_needs_sign(&task.payload) && self.soter_nosign == Some(true))
    }
}

fn soter_needs_sign(payload: &Value) -> bool {
    matches!(
        payload.get("op").and_then(Value::as_str),
        Some("init_sign" | "finish_sign")
    )
}

#[derive(Debug, Clone, Default)]
pub struct TaskCounts {
    pub pending: usize,
    pub assigned: usize,
    pub completed: usize,
    pub failed: usize,
}

/// How far back device activity is summed for load estimation. The queue in
/// `record_event_locked` is only pruned when a *new* event arrives, so a
/// device that went quiet keeps stale entries and the read side has to filter
/// them out.
const ACTIVITY_WINDOW_MS: u64 = 60_000;

#[derive(Default)]
struct Inner {
    tasks: HashMap<String, Task>,
    /// Only open tasks own senders. A published result stays readable in the
    /// receivers even when the terminal task is pruned before a waiter runs.
    result_senders: HashMap<String, watch::Sender<Option<Value>>>,
    /// A-side idempotency key (`task_type:request_id`) -> task id.
    request_index: HashMap<String, String>,
    /// Per-device pending queues: device_id -> FIFO of task_ids targeting it.
    pending_by_device: HashMap<String, VecDeque<String>>,
    /// Pending tasks with no target device (any device can claim them).
    pending_any: VecDeque<String>,
    /// Completed tasks ordered by completion time: (completed_at_ms, task_id).
    completed_queue: VecDeque<(u64, String)>,
    /// Failed tasks ordered by completion time: (failed_at_ms, task_id).
    failed_queue: VecDeque<(u64, String)>,
    devices: HashMap<String, DeviceEntry>,
    /// device_id -> (machine_id, last_seen_ms) that most recently served it (for concurrency check).
    active_machine: HashMap<String, (String, u64)>,
    /// per-device recent activity for load estimation: (timestamp_ms, weight).
    device_events: HashMap<String, VecDeque<(u64, u64)>>,
    /// Shared only by concurrent polls for the same device. Weak handles do not
    /// retain a cancelled HTTP poll or its waiter.
    poll_notifiers: HashMap<String, Weak<Notify>>,
}

pub struct TaskStore {
    inner: Mutex<Inner>,
    assignment_timeout: Duration,
    /// How long a pending task may wait before being marked as failed (timeout).
    pending_ttl: Duration,
    /// Maximum number of completed/failed tasks to retain (each category independently).
    completed_max: usize,
    /// How long completed/failed tasks are kept before being purged.
    completed_ttl: Duration,
}

impl TaskStore {
    pub fn new(
        assignment_timeout_secs: u64,
        pending_ttl_secs: u64,
        completed_max: usize,
        completed_ttl_secs: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
            assignment_timeout: Duration::from_secs(assignment_timeout_secs),
            pending_ttl: Duration::from_secs(pending_ttl_secs),
            completed_max,
            completed_ttl: Duration::from_secs(completed_ttl_secs),
        })
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Create a task and enqueue it. Returns the task_id.
    pub async fn create_task(
        &self,
        task_type: &str,
        payload: Value,
        target_device_id: &str,
        request_id: Option<&str>,
    ) -> String {
        let now = Self::now_ms();
        let mut inner = self.inner.lock().await;
        let request_id = request_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let request_key = request_id
            .as_ref()
            .map(|value| format!("{task_type}:{target_device_id}:{value}"));
        if let Some(key) = request_key.as_ref() {
            if let Some(existing) = inner.request_index.get(key).cloned() {
                if inner.tasks.contains_key(&existing) {
                    return existing;
                }
                inner.request_index.remove(key);
            }
        }
        let task_id = uuid::Uuid::new_v4().to_string();
        inner.tasks.insert(
            task_id.clone(),
            Task {
                task_id: task_id.clone(),
                task_type: task_type.to_string(),
                request_id,
                payload,
                target_device_id: target_device_id.to_string(),
                assigned_device_id: None,
                assigned_at_ms: 0,
                result: None,
                created_at_ms: now,
                completed_at_ms: 0,
                status: TaskStatus::Pending,
                created_at: Instant::now(),
                assigned_at: None,
            },
        );
        if let Some(key) = request_key {
            inner.request_index.insert(key, task_id.clone());
        }
        inner
            .result_senders
            .insert(task_id.clone(), watch::channel(None).0);
        // Enqueue into the per-device bucket or the wildcard queue.
        if target_device_id.is_empty() {
            inner.pending_any.push_back(task_id.clone());
        } else {
            inner
                .pending_by_device
                .entry(target_device_id.to_string())
                .or_default()
                .push_back(task_id.clone());
        }
        Self::notify_pending_locked(&inner, target_device_id);
        task_id
    }

    /// Targeted work wakes only that device; wildcard work may wake any poll.
    /// Call under the queue lock so notification and enqueue are one transition.
    fn notify_pending_locked(inner: &Inner, target_device_id: &str) {
        if target_device_id.is_empty() {
            for notify in inner.poll_notifiers.values().filter_map(Weak::upgrade) {
                notify.notify_waiters();
            }
        } else if let Some(notify) = inner
            .poll_notifiers
            .get(target_device_id)
            .and_then(Weak::upgrade)
        {
            notify.notify_waiters();
        }
    }

    fn remove_task_locked(inner: &mut Inner, task_id: &str) -> Option<Task> {
        let task = inner.tasks.remove(task_id)?;
        // The task left the table, so no result can be published for it any
        // more: close the waiter's channel instead of leaving it to the full
        // timeout. A cancelled task keeps the old "no early return" semantics
        // - see `wait_for_result`.
        inner.result_senders.remove(task_id);
        if let Some(request_id) = task.request_id.as_ref() {
            inner.request_index.remove(&format!(
                "{}:{}:{request_id}",
                task.task_type, task.target_device_id
            ));
        }
        Some(task)
    }

    /// Publish a task's terminal result under the transition lock, then drop
    /// the sender. `watch` keeps the value for receivers, so a waiter that
    /// subscribed before the transition sees it, and one that subscribes after
    /// sees it through [`TaskStore::wait_for_result`]'s initial check.
    fn publish_result_locked(inner: &mut Inner, task_id: &str) {
        if let Some(sender) = inner.result_senders.remove(task_id) {
            let result = inner
                .tasks
                .get(task_id)
                .and_then(|task| task.result.clone());
            sender.send_replace(result);
        }
    }

    /// Record a device event (must be called while holding `inner`).
    fn record_event_locked(inner: &mut Inner, device_id: &str, weight: u64) {
        let now = Self::now_ms();
        let q = inner
            .device_events
            .entry(device_id.to_string())
            .or_default();
        q.push_back((now, weight));
        while let Some((ts, _)) = q.front() {
            if now.saturating_sub(*ts) > ACTIVITY_WINDOW_MS {
                q.pop_front();
            } else {
                break;
            }
        }
    }

    /// Pop the next pending task matching this device, with long-poll semantics.
    /// Returns None after `timeout` elapsed with no match.
    pub async fn pop_for_b(
        &self,
        device_id: &str,
        machine_id: &str,
        timeout: Duration,
    ) -> Option<Task> {
        self.pop_for_b_with_caps(device_id, machine_id, timeout, DeviceCaps::default())
            .await
    }

    pub async fn pop_for_b_with_caps(
        &self,
        device_id: &str,
        machine_id: &str,
        timeout: Duration,
        caps: DeviceCaps,
    ) -> Option<Task> {
        self.pop_for_b_with_caps_and_version(device_id, machine_id, timeout, caps, None)
            .await
    }

    pub async fn pop_for_b_with_caps_and_version(
        &self,
        device_id: &str,
        machine_id: &str,
        timeout: Duration,
        caps: DeviceCaps,
        version: Option<&str>,
    ) -> Option<Task> {
        self.pop_for_b_with_metadata(
            device_id,
            machine_id,
            timeout,
            DeviceMetadata {
                caps,
                version,
                ..DeviceMetadata::default()
            },
        )
        .await
    }

    pub async fn pop_for_b_with_metadata(
        &self,
        device_id: &str,
        machine_id: &str,
        timeout: Duration,
        metadata: DeviceMetadata<'_>,
    ) -> Option<Task> {
        let DeviceMetadata {
            caps,
            version,
            battery,
        } = metadata;
        // Capture once per received poll, outside wait_until's repeated checks.
        // Queue notifications or a long-poll timeout are not new telemetry.
        let received_at_ms = Self::now_ms();
        let battery_updated_at_ms =
            (battery.percent.is_some() || battery.charging.is_some()).then_some(received_at_ms);
        let assignment_timeout = self.assignment_timeout;
        let pending_ttl = self.pending_ttl;
        let completed_ttl = self.completed_ttl;
        let completed_max = self.completed_max;
        let notify = {
            let mut inner = self.inner.lock().await;
            let slot = inner
                .poll_notifiers
                .entry(device_id.to_string())
                .or_default();
            match slot.upgrade() {
                Some(notify) => notify,
                None => {
                    let notify = Arc::new(Notify::new());
                    *slot = Arc::downgrade(&notify);
                    notify
                }
            }
        };
        self.wait_until(&notify, timeout, |inner| {
            let known_version = inner
                .devices
                .get(device_id)
                .and_then(|entry| entry.version.clone());
            inner.devices.insert(
                device_id.to_string(),
                DeviceEntry {
                    device_id: device_id.to_string(),
                    machine_id: machine_id.to_string(),
                    last_seen_ms: Self::now_ms(),
                    connected: true,
                    soter: caps.soter,
                    soter_sign: caps.soter_sign,
                    soter_nosign: caps.soter_nosign,
                    version: version.map(str::to_string).or(known_version),
                    // Do not carry stale battery data across unknown/legacy
                    // polls or a different machine serving the same device.
                    battery_percent: battery.percent,
                    battery_charging: battery.charging,
                    battery_updated_at_ms,
                },
            );
            if !machine_id.is_empty() {
                inner.active_machine.insert(
                    device_id.to_string(),
                    (machine_id.to_string(), Self::now_ms()),
                );
            }
            Self::reclaim_locked(inner, assignment_timeout);
            Self::expire_locked(inner, pending_ttl, completed_max, completed_ttl);
            Self::dequeue_locked(inner, device_id, caps).map(|task| {
                Self::record_event_locked(inner, device_id, 1);
                task
            })
        })
        .await
    }

    /// Enable-then-check-then-await on the device notifier. Timeout runs `check`
    /// once more so an enqueue that races the deadline is not dropped.
    async fn wait_until<T>(
        &self,
        notify: &Notify,
        timeout: Duration,
        mut check: impl FnMut(&mut Inner) -> Option<T>,
    ) -> Option<T> {
        let deadline = Instant::now() + timeout;
        loop {
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut inner = self.inner.lock().await;
                if let Some(v) = check(&mut inner) {
                    return Some(v);
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let mut inner = self.inner.lock().await;
                return check(&mut inner);
            }
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(remaining) => {
                    let mut inner = self.inner.lock().await;
                    return check(&mut inner);
                }
            }
        }
    }

    /// Try to dequeue a task matching this device from the FIFO.
    /// O(1): checks per-device queue first, then the wildcard queue.
    fn dequeue_locked(inner: &mut Inner, device_id: &str, caps: DeviceCaps) -> Option<Task> {
        // 0) Attest outranks older sign/decrypt work for the same device. Scan a
        // snapshot of the IDs so the queue and task table are not borrowed at the
        // same time; FIFO is preserved within the attest class.
        let attest_candidate = {
            let ids: Vec<String> = inner
                .pending_by_device
                .get(device_id)
                .map(|q| q.iter().cloned().collect())
                .unwrap_or_default();
            ids.into_iter().find(|id| {
                inner
                    .tasks
                    .get(id)
                    .is_some_and(|task| task.task_type == "attest" && caps.can_serve(task))
            })
        };
        if let Some(task_id) = attest_candidate {
            if let Some(q) = inner.pending_by_device.get_mut(device_id) {
                if let Some(pos) = q.iter().position(|id| id == &task_id) {
                    q.remove(pos);
                }
                if q.is_empty() {
                    inner.pending_by_device.remove(device_id);
                }
            }
            if let Some(task) = inner.tasks.get_mut(&task_id) {
                task.assigned_device_id = Some(device_id.to_string());
                task.assigned_at_ms = Self::now_ms();
                task.assigned_at = Some(Instant::now());
                tracing::debug!(
                    event = "relay_claim",
                    task_id = %task.task_id,
                    queue_us = task.created_at.elapsed().as_micros() as u64
                );
                task.status = TaskStatus::Assigned;
                return Some(task.clone());
            }
        }

        // 1) Try device-specific queue first.
        if let Some(q) = inner.pending_by_device.get_mut(device_id) {
            for _ in 0..q.len() {
                let Some(candidate_id) = q.pop_front() else {
                    break;
                };
                if let Some(t) = inner.tasks.get_mut(&candidate_id) {
                    if !caps.can_serve(t) {
                        q.push_back(candidate_id);
                        continue;
                    }
                    t.assigned_device_id = Some(device_id.to_string());
                    t.assigned_at_ms = Self::now_ms();
                    t.assigned_at = Some(Instant::now());
                    tracing::debug!(event = "relay_claim", task_id = %t.task_id,
                        queue_us = t.created_at.elapsed().as_micros() as u64);
                    t.status = TaskStatus::Assigned;
                    return Some(t.clone());
                }
                // Stale id (task no longer exists) — drop it.
            }
            // Keep tasks that this device cannot currently serve. A later
            // heartbeat may advertise a restored SOTER capability.
            if q.is_empty() {
                inner.pending_by_device.remove(device_id);
            }
        }

        // 2) Try wildcard (any-device) queue.
        for _ in 0..inner.pending_any.len() {
            let Some(candidate_id) = inner.pending_any.pop_front() else {
                break;
            };
            if let Some(t) = inner.tasks.get_mut(&candidate_id) {
                if !caps.can_serve(t) {
                    inner.pending_any.push_back(candidate_id);
                    continue;
                }
                t.assigned_device_id = Some(device_id.to_string());
                t.assigned_at_ms = Self::now_ms();
                t.assigned_at = Some(Instant::now());
                tracing::debug!(event = "relay_claim", task_id = %t.task_id,
                    queue_us = t.created_at.elapsed().as_micros() as u64);
                t.status = TaskStatus::Assigned;
                return Some(t.clone());
            }
            // Stale id (task no longer exists) — drop it.
        }

        None
    }

    /// Expire stale pending tasks and prune old completed/failed tasks.
    /// Must be called while holding `inner` lock.
    fn expire_locked(
        inner: &mut Inner,
        pending_ttl: Duration,
        completed_max: usize,
        completed_ttl: Duration,
    ) {
        let now = Self::now_ms();
        let pending_ttl_ms = pending_ttl.as_millis() as u64;
        let completed_ttl_ms = completed_ttl.as_millis() as u64;

        // 1) Expire pending tasks older than pending_ttl → mark as Failed.
        let expired_pending: Vec<String> = inner
            .tasks
            .iter()
            .filter(|(_, t)| {
                t.status == TaskStatus::Pending
                    && now.saturating_sub(t.created_at_ms) > pending_ttl_ms
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired_pending {
            if let Some(t) = inner.tasks.get_mut(id) {
                t.status = TaskStatus::Failed;
                t.result = Some(serde_json::json!({
                    "error": "task expired: pending TTL exceeded"
                }));
                t.completed_at_ms = now;
                inner.failed_queue.push_back((now, id.clone()));
            }
            Self::publish_result_locked(inner, id);
        }
        // Remove expired tasks from per-device and wildcard queues.
        if !expired_pending.is_empty() {
            let expired_set: std::collections::HashSet<&String> = expired_pending.iter().collect();
            for queue in inner.pending_by_device.values_mut() {
                queue.retain(|id| !expired_set.contains(id));
            }
            inner.pending_any.retain(|id| !expired_set.contains(id));
            // Clean up empty per-device queues.
            inner.pending_by_device.retain(|_, q| !q.is_empty());
        }

        // 2) Prune completed tasks by TTL (front of queue = oldest).
        while let Some(&(ts, _)) = inner.completed_queue.front() {
            if now.saturating_sub(ts) > completed_ttl_ms {
                if let Some((_, id)) = inner.completed_queue.pop_front() {
                    Self::remove_task_locked(inner, &id);
                }
            } else {
                break;
            }
        }

        // 3) Prune failed tasks by TTL.
        while let Some(&(ts, _)) = inner.failed_queue.front() {
            if now.saturating_sub(ts) > completed_ttl_ms {
                if let Some((_, id)) = inner.failed_queue.pop_front() {
                    Self::remove_task_locked(inner, &id);
                }
            } else {
                break;
            }
        }

        // 4) Prune completed tasks by max count.
        while inner.completed_queue.len() > completed_max {
            if let Some((_, id)) = inner.completed_queue.pop_front() {
                Self::remove_task_locked(inner, &id);
            }
        }

        // 5) Prune failed tasks by max count.
        while inner.failed_queue.len() > completed_max {
            if let Some((_, id)) = inner.failed_queue.pop_front() {
                Self::remove_task_locked(inner, &id);
            }
        }
    }

    /// Reclaim tasks assigned to devices that never returned a result in time.
    /// Skip assignees that are still long-polling: their generateKey is in
    /// flight, and putting the task back would let another worker dequeue the
    /// same id.
    fn reclaim_locked(inner: &mut Inner, assignment_timeout: Duration) {
        let now = Self::now_ms();
        let timeout_ms = assignment_timeout.as_millis() as u64;
        let stale: Vec<String> = inner
            .tasks
            .iter()
            .filter(|(_, t)| {
                t.status == TaskStatus::Assigned
                    && now.saturating_sub(t.assigned_at_ms) > timeout_ms
                    && !Self::assignee_still_connected(inner, t.assigned_device_id.as_deref(), now)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            let mut ended = false;
            let mut requeued_target = None;
            if let Some(t) = inner.tasks.get_mut(&id) {
                // A lost poller does not prove its secure-world call ended.
                // Never replay an agreement or SOTER operation that may still
                // be in flight in the real TEE.
                if t.must_not_replay() {
                    t.status = TaskStatus::Failed;
                    t.result = Some(serde_json::json!({
                        "error": "sensitive assignment lost; execution state unknown; not retried"
                    }));
                    t.completed_at_ms = now;
                    inner.failed_queue.push_back((now, id.clone()));
                    ended = true;
                } else {
                    t.status = TaskStatus::Pending;
                    t.assigned_device_id = None;
                    t.assigned_at = None;
                    requeued_target = Some(t.target_device_id.clone());
                    // Put back into the appropriate bucket.
                    if t.target_device_id.is_empty() {
                        inner.pending_any.push_back(id.clone());
                    } else {
                        inner
                            .pending_by_device
                            .entry(t.target_device_id.clone())
                            .or_default()
                            .push_back(id.clone());
                    }
                }
            }
            if ended {
                Self::publish_result_locked(inner, &id);
            }
            if let Some(target) = requeued_target {
                Self::notify_pending_locked(inner, &target);
            }
        }
    }

    fn assignee_still_connected(inner: &Inner, device_id: Option<&str>, now: u64) -> bool {
        let Some(id) = device_id else {
            return false;
        };
        inner
            .devices
            .get(id)
            .is_some_and(|d| now.saturating_sub(d.last_seen_ms) < 120_000)
    }

    /// Complete a task with a result reported by the B-side.
    /// Returns Ok(()) if the task existed, Err(msg) otherwise.
    pub async fn complete_task(
        &self,
        task_id: &str,
        result: Value,
        device_id: &str,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().await;
        let Some(task) = inner.tasks.get_mut(task_id) else {
            return Err("task not found".to_string());
        };
        if task.status != TaskStatus::Assigned {
            return Err(format!(
                "task is not assigned (status={})",
                task.status_str()
            ));
        }
        if device_id.is_empty() || task.assigned_device_id.as_deref() != Some(device_id) {
            return Err("task result came from the wrong device".to_string());
        }
        let now = Self::now_ms();
        let is_err = result.get("error").is_some();
        task.result = Some(result);
        task.status = if is_err {
            TaskStatus::Failed
        } else {
            TaskStatus::Completed
        };
        task.completed_at_ms = now;
        tracing::debug!(
            event = "relay_complete",
            task_id,
            assigned_us = task.assigned_at.map(|at| at.elapsed().as_micros() as u64),
            total_us = task.created_at.elapsed().as_micros() as u64
        );
        // Track in the appropriate ordered queue for later TTL / capacity pruning.
        if is_err {
            inner.failed_queue.push_back((now, task_id.to_string()));
        } else {
            inner.completed_queue.push_back((now, task_id.to_string()));
        }
        Self::record_event_locked(&mut inner, device_id, 1);
        // Publish before pruning: a waiter must see what B reported even when
        // this completion is trimmed immediately by the capacity/TTL sweep.
        Self::publish_result_locked(&mut inner, task_id);
        // Prune completed/failed tasks to stay within capacity/TTL limits.
        Self::expire_locked(
            &mut inner,
            self.pending_ttl,
            self.completed_max,
            self.completed_ttl,
        );
        // Only this task's A-side watch receivers need the completion. B polls
        // wait for new work, not unrelated result uploads.
        Ok(())
    }

    /// Wait until the task is completed or failed. Returns None on timeout.
    ///
    /// Every open task owns a `watch` sender, so a completion wakes only the
    /// waiters of that task. B-side long polls use device-specific notifiers
    /// and are not woken by result uploads. A result
    /// published just before the task is pruned stays readable in the
    /// receivers.
    pub async fn wait_for_result(&self, task_id: &str, timeout: Duration) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        let rx = {
            let inner = self.inner.lock().await;
            if let Some(task) = inner.tasks.get(task_id) {
                if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed) {
                    return task.result.clone();
                }
            }
            // Subscribe while holding the transition lock: there is no
            // check-then-subscribe gap in which a completion could be lost.
            inner
                .result_senders
                .get(task_id)
                .map(|sender| sender.subscribe())
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if let Some(mut rx) = rx {
            let result = tokio::time::timeout(remaining, async {
                loop {
                    if let Some(result) = rx.borrow_and_update().clone() {
                        return Some(result);
                    }
                    if rx.changed().await.is_err() {
                        // Cancelled task: the channel closes without a result.
                        return None;
                    }
                }
            })
            .await;
            if let Ok(Some(result)) = result {
                return Some(result);
            }
        }
        // No result yet: keep the previous semantics (the caller sees the
        // deadline, not an early return) and re-check once at the deadline,
        // because a completion can race it.
        tokio::time::sleep(deadline.saturating_duration_since(Instant::now())).await;
        let inner = self.inner.lock().await;
        inner.tasks.get(task_id).and_then(|task| {
            matches!(task.status, TaskStatus::Completed | TaskStatus::Failed)
                .then(|| task.result.clone())
                .flatten()
        })
    }

    pub async fn list_tasks(&self, limit: usize) -> Vec<Task> {
        let inner = self.inner.lock().await;
        let mut v: Vec<Task> = inner.tasks.values().cloned().collect();
        v.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        v.truncate(limit);
        v
    }

    pub async fn counts(&self) -> TaskCounts {
        let inner = self.inner.lock().await;
        let mut c = TaskCounts::default();
        for t in inner.tasks.values() {
            match t.status {
                TaskStatus::Pending => c.pending += 1,
                TaskStatus::Assigned => c.assigned += 1,
                TaskStatus::Completed => c.completed += 1,
                TaskStatus::Failed => c.failed += 1,
            }
        }
        c
    }

    pub async fn cancel_task(&self, task_id: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().await;
        if Self::remove_task_locked(&mut inner, task_id).is_some() {
            // Remove from all pending queues.
            for queue in inner.pending_by_device.values_mut() {
                queue.retain(|id| id != task_id);
            }
            inner.pending_by_device.retain(|_, q| !q.is_empty());
            inner.pending_any.retain(|id| id != task_id);
            inner.completed_queue.retain(|(_, id)| id != task_id);
            inner.failed_queue.retain(|(_, id)| id != task_id);
            Ok(())
        } else {
            Err("task not found".to_string())
        }
    }

    /// Withdraw a timed-out SOTER task only if B has not claimed it. An
    /// assigned task may already be inside the vendor HAL and must be left
    /// alone; its late result is evidence, not permission to replay it.
    pub async fn cancel_pending_task(&self, task_id: &str) -> bool {
        let mut inner = self.inner.lock().await;
        if !inner
            .tasks
            .get(task_id)
            .is_some_and(|task| task.status == TaskStatus::Pending)
        {
            return false;
        }
        Self::remove_task_locked(&mut inner, task_id);
        for queue in inner.pending_by_device.values_mut() {
            queue.retain(|id| id != task_id);
        }
        inner.pending_by_device.retain(|_, queue| !queue.is_empty());
        inner.pending_any.retain(|id| id != task_id);
        true
    }

    pub async fn get_active_machine_id(&self, device_id: &str) -> Option<String> {
        let inner = self.inner.lock().await;
        let now = Self::now_ms();
        inner
            .active_machine
            .get(device_id)
            .filter(|(_, ts)| now.saturating_sub(*ts) < 30_000)
            .map(|(m, _)| m.clone())
    }

    pub async fn get_connected_devices(&self) -> Vec<DeviceEntry> {
        let inner = self.inner.lock().await;
        let now = Self::now_ms();
        inner
            .devices
            .values()
            .filter(|d| now.saturating_sub(d.last_seen_ms) < 120_000)
            .cloned()
            .collect()
    }

    pub async fn is_device_online(&self, device_id: &str) -> bool {
        let inner = self.inner.lock().await;
        let now = Self::now_ms();
        inner
            .devices
            .get(device_id)
            .is_some_and(|device| now.saturating_sub(device.last_seen_ms) < 120_000)
    }

    /// A SOTER task must stay on its named device; absent/negative capability
    /// is not permission to substitute another device or a software identity.
    pub async fn can_route_soter(&self, device_id: &str, needs_sign: bool) -> bool {
        let inner = self.inner.lock().await;
        let now = Self::now_ms();
        inner.devices.get(device_id).is_some_and(|device| {
            now.saturating_sub(device.last_seen_ms) < 120_000
                && device.soter == Some(true)
                && !(needs_sign && device.soter_nosign == Some(true))
        })
    }

    pub async fn get_device_load(&self, device_id: &str) -> u64 {
        let inner = self.inner.lock().await;
        Self::window_activity_at(&inner, device_id, Self::now_ms())
    }

    /// Sum only the events inside [`ACTIVITY_WINDOW_MS`]. Without the filter a
    /// device that went quiet keeps its old events (the queue is only pruned
    /// when a new event arrives) and reads as busy forever, losing the
    /// tie-break to a machine that is actually idle.
    fn window_activity_at(inner: &Inner, device_id: &str, now: u64) -> u64 {
        inner
            .device_events
            .get(device_id)
            .map(|events| {
                events
                    .iter()
                    .filter(|(ts, _)| now.saturating_sub(*ts) <= ACTIVITY_WINDOW_MS)
                    .map(|(_, weight)| *weight)
                    .sum()
            })
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn battery_parser_preserves_boundaries_and_rejects_bogus_metadata() {
        assert_eq!(
            DeviceBattery::parse(Some("0"), Some("false")),
            DeviceBattery {
                percent: Some(0),
                charging: Some(false)
            }
        );
        assert_eq!(
            DeviceBattery::parse(Some("100"), Some("true")),
            DeviceBattery {
                percent: Some(100),
                charging: Some(true)
            }
        );
        assert_eq!(DeviceBattery::parse(None, None), DeviceBattery::default());
        for percent in [
            "",
            "-1",
            "+1",
            "101",
            "256",
            "999999999999",
            "5.5",
            "80%",
            "NaN",
        ] {
            assert_eq!(DeviceBattery::parse(Some(percent), None).percent, None);
        }
        for charging in ["", "yes", "0", "1", "unknown", "TRUE"] {
            assert_eq!(DeviceBattery::parse(None, Some(charging)).charging, None);
        }
        assert_eq!(
            DeviceBattery::parse(Some("bad"), Some("false")),
            DeviceBattery {
                percent: None,
                charging: Some(false)
            }
        );
    }

    #[tokio::test]
    async fn battery_receipt_timestamp_is_not_extended_by_long_poll_checks() {
        let store = store();
        store
            .pop_for_b_with_metadata(
                "dev",
                "m1",
                Duration::from_millis(30),
                DeviceMetadata {
                    battery: DeviceBattery::parse(Some("0"), Some("false")),
                    ..DeviceMetadata::default()
                },
            )
            .await;
        let inner = store.inner.lock().await;
        let device = inner.devices.get("dev").unwrap();
        assert_eq!(device.battery_percent, Some(0));
        assert_eq!(device.battery_charging, Some(false));
        assert!(
            device.battery_updated_at_ms.unwrap() < device.last_seen_ms,
            "a timeout re-check is not a newly received battery sample"
        );
    }

    #[tokio::test]
    async fn battery_unknown_poll_clears_snapshot_and_offline_devices_are_omitted() {
        let store = store();
        store
            .pop_for_b_with_metadata(
                "dev",
                "m1",
                Duration::ZERO,
                DeviceMetadata {
                    battery: DeviceBattery::parse(Some("100"), Some("true")),
                    ..DeviceMetadata::default()
                },
            )
            .await;
        assert!(store.get_connected_devices().await[0]
            .battery_updated_at_ms
            .is_some());
        store.pop_for_b("dev", "m1", Duration::ZERO).await;
        let device = &store.get_connected_devices().await[0];
        assert_eq!(device.battery_percent, None);
        assert_eq!(device.battery_charging, None);
        assert_eq!(device.battery_updated_at_ms, None);
        {
            let mut inner = store.inner.lock().await;
            inner.devices.get_mut("dev").unwrap().last_seen_ms = 1;
        }
        assert!(store.get_connected_devices().await.is_empty());
    }

    #[test]
    fn soter_caps_distinguish_unreported_from_disabled_and_sign_limited() {
        assert_eq!(DeviceCaps::parse(None).soter, None);
        assert_eq!(DeviceCaps::parse(Some("")).soter, Some(false));
        let caps = DeviceCaps::parse(Some("soter,soter_nosign"));
        assert_eq!(caps.soter, Some(true));
        assert_eq!(caps.soter_nosign, Some(true));
        assert_eq!(caps.soter_sign, Some(false));
    }

    #[tokio::test]
    async fn soter_tasks_require_named_device_capability_and_preserve_sign_boundary() {
        let store = store();
        let identity = store
            .create_task("soter", json!({"op": "get_device_id"}), "dev", None)
            .await;
        assert!(store
            .pop_for_b_with_caps(
                "other",
                "m2",
                Duration::ZERO,
                DeviceCaps::parse(Some("soter,soter_sign")),
            )
            .await
            .is_none());
        assert!(store
            .pop_for_b_with_caps("dev", "m1", Duration::ZERO, DeviceCaps::parse(Some("")))
            .await
            .is_none());
        assert!(!store.can_route_soter("dev", false).await);
        let caps = DeviceCaps::parse(Some("soter,soter_nosign"));
        let served = store
            .pop_for_b_with_caps("dev", "m1", Duration::ZERO, caps)
            .await
            .unwrap();
        assert_eq!(served.task_id, identity);
        assert!(!store.can_route_soter("dev", true).await);

        let sign = store
            .create_task("soter", json!({"op": "finish_sign"}), "dev", None)
            .await;
        assert!(store
            .pop_for_b_with_caps("dev", "m1", Duration::ZERO, caps)
            .await
            .is_none());
        let eligible = DeviceCaps::parse(Some("soter,soter_sign"));
        assert!(store.can_route_soter("dev", false).await);
        let served = store
            .pop_for_b_with_caps("dev", "m1", Duration::ZERO, eligible)
            .await
            .unwrap();
        assert_eq!(served.task_id, sign);
    }

    #[tokio::test]
    async fn relay_version_is_recorded_and_kept_when_a_later_poll_omits_it() {
        let store = store();
        store
            .pop_for_b_with_caps_and_version(
                "dev",
                "m1",
                Duration::ZERO,
                DeviceCaps::default(),
                Some("1.8.0"),
            )
            .await;
        {
            let inner = store.inner.lock().await;
            assert_eq!(
                inner.devices.get("dev").and_then(|d| d.version.as_deref()),
                Some("1.8.0")
            );
        }
        store
            .pop_for_b_with_caps_and_version(
                "dev",
                "m1",
                Duration::ZERO,
                DeviceCaps::default(),
                None,
            )
            .await;
        let inner = store.inner.lock().await;
        assert_eq!(
            inner.devices.get("dev").and_then(|d| d.version.as_deref()),
            Some("1.8.0")
        );
    }

    #[tokio::test]
    async fn lost_soter_assignment_is_never_replayed() {
        let store = store();
        let id = store
            .create_task(
                "soter",
                json!({"op": "generate_auth_key_pair"}),
                "dev",
                None,
            )
            .await;
        let caps = DeviceCaps::parse(Some("soter"));
        store
            .pop_for_b_with_caps("dev", "m1", Duration::ZERO, caps)
            .await
            .unwrap();
        {
            let mut inner = store.inner.lock().await;
            inner.tasks.get_mut(&id).unwrap().assigned_at_ms = 1;
            inner.devices.get_mut("dev").unwrap().last_seen_ms = 1;
            TaskStore::reclaim_locked(&mut inner, Duration::ZERO);
        }
        let result = store.wait_for_result(&id, Duration::ZERO).await.unwrap();
        assert!(result["error"].as_str().unwrap().contains("not retried"));
        assert!(store
            .pop_for_b_with_caps("dev", "m1", Duration::ZERO, caps)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn timed_out_pending_soter_is_withdrawn_before_a_late_poll() {
        let store = store();
        let id = store
            .create_task(
                "soter",
                json!({"op": "generate_auth_key_pair"}),
                "dev",
                None,
            )
            .await;
        assert!(store.cancel_pending_task(&id).await);
        assert!(store
            .pop_for_b_with_caps(
                "dev",
                "m1",
                Duration::ZERO,
                DeviceCaps::parse(Some("soter")),
            )
            .await
            .is_none());
    }

    fn store() -> Arc<TaskStore> {
        TaskStore::new(60, 60, 100, 60)
    }

    #[tokio::test]
    async fn attest_is_dequeued_before_an_earlier_sign() {
        let store = store();
        let sign = store
            .create_task("sign", json!({}), "dev", None)
            .await;
        let attest = store
            .create_task("attest", json!({}), "dev", None)
            .await;

        let first = store
            .pop_for_b("dev", "m1", Duration::ZERO)
            .await
            .expect("first task");
        assert_eq!(first.task_id, attest);
        assert_eq!(first.task_type, "attest");

        let second = store
            .pop_for_b("dev", "m1", Duration::ZERO)
            .await
            .expect("second task");
        assert_eq!(second.task_id, sign);
        assert_eq!(second.task_type, "sign");
    }

    fn assert_woke_fast(started: Instant) {
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(80),
            "notify wake took {elapsed:?} (250ms fallback would still pass 400ms)"
        );
    }

    async fn create_assigned(store: &Arc<TaskStore>) -> String {
        let task_id = store.create_task("attest", json!({}), "dev", None).await;
        store
            .pop_for_b("dev", "m1", Duration::from_millis(1))
            .await
            .expect("assign task");
        task_id
    }

    #[tokio::test]
    async fn pop_wakes_immediately_when_task_arrives() {
        let store = store();
        let waiter = store.clone();
        let started = Instant::now();
        let join =
            tokio::spawn(
                async move { waiter.pop_for_b("dev", "m1", Duration::from_secs(2)).await },
            );
        tokio::time::sleep(Duration::from_millis(20)).await;
        let task_id = store.create_task("attest", json!({}), "dev", None).await;
        let task = join.await.expect("join").expect("task");
        assert_eq!(task.task_id, task_id);
        assert_woke_fast(started);
    }

    #[tokio::test]
    async fn wait_for_result_wakes_immediately_on_complete() {
        let store = store();
        let task_id = create_assigned(&store).await;
        let waiter = store.clone();
        let id = task_id.clone();
        let started = Instant::now();
        let join =
            tokio::spawn(async move { waiter.wait_for_result(&id, Duration::from_secs(2)).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        store
            .complete_task(&task_id, json!({"ok": true}), "dev")
            .await
            .expect("complete");
        let result = join.await.expect("join").expect("result");
        assert_eq!(result.get("ok"), Some(&json!(true)));
        assert_woke_fast(started);
    }

    #[tokio::test]
    async fn wait_for_result_sees_already_completed_task() {
        let store = store();
        let task_id = create_assigned(&store).await;
        store
            .complete_task(&task_id, json!({"ok": true}), "dev")
            .await
            .expect("complete");
        let result = store
            .wait_for_result(&task_id, Duration::from_millis(50))
            .await
            .expect("result");
        assert_eq!(result.get("ok"), Some(&json!(true)));
    }

    #[tokio::test]
    async fn wait_for_result_complete_before_waiter_is_registered() {
        let store = store();
        let task_id = create_assigned(&store).await;
        let waiter = store.clone();
        let id = task_id.clone();
        let join =
            tokio::spawn(async move { waiter.wait_for_result(&id, Duration::from_secs(2)).await });
        store
            .complete_task(&task_id, json!({"ok": true}), "dev")
            .await
            .expect("complete");
        let result = join.await.expect("join").expect("result");
        assert_eq!(result.get("ok"), Some(&json!(true)));
    }

    #[tokio::test]
    async fn wait_for_result_timeout_rechecks_completed_task() {
        let store = store();
        let task_id = create_assigned(&store).await;
        let waiter = store.clone();
        let id = task_id.clone();
        let join =
            tokio::spawn(
                async move { waiter.wait_for_result(&id, Duration::from_millis(80)).await },
            );
        tokio::time::sleep(Duration::from_millis(70)).await;
        store
            .complete_task(&task_id, json!({"ok": true}), "dev")
            .await
            .expect("complete");
        let result = join.await.expect("join").expect("result");
        assert_eq!(result.get("ok"), Some(&json!(true)));
    }

    #[tokio::test]
    async fn live_assignee_is_not_reclaimed_for_a_second_pop() {
        let store = TaskStore::new(0, 60, 100, 60);
        let task_id = store.create_task("attest", json!({}), "dev", None).await;
        let taken = store
            .pop_for_b("dev", "m1", Duration::from_millis(40))
            .await
            .expect("assigned");
        assert_eq!(taken.task_id, task_id);
        {
            let mut inner = store.inner.lock().await;
            if let Some(t) = inner.tasks.get_mut(&task_id) {
                t.assigned_at_ms = 1;
            }
        }
        let stolen = store
            .pop_for_b("dev", "m1", Duration::from_millis(40))
            .await;
        assert!(
            stolen.is_none(),
            "live B poller must not reclaim its in-flight task"
        );
    }

    #[tokio::test]
    async fn request_id_reuses_the_existing_task() {
        let store = store();
        let first = store
            .create_task("attest", json!({"n": 1}), "dev", Some("request-1"))
            .await;
        let second = store
            .create_task("attest", json!({"n": 1}), "dev", Some("request-1"))
            .await;
        assert_eq!(first, second);
        assert_eq!(store.counts().await.pending, 1);
    }

    #[tokio::test]
    async fn lost_agreement_is_failed_without_replay_or_fabricated_hal_error() {
        let store = store();
        let id = store
            .create_task("agree", json!({}), "dev", Some("agree-1"))
            .await;
        store.pop_for_b("dev", "m1", Duration::ZERO).await.unwrap();
        {
            let mut inner = store.inner.lock().await;
            inner.tasks.get_mut(&id).unwrap().assigned_at_ms = 1;
            inner.devices.get_mut("dev").unwrap().last_seen_ms = 1;
            TaskStore::reclaim_locked(&mut inner, Duration::ZERO);
        }
        let result = store.wait_for_result(&id, Duration::ZERO).await.unwrap();
        assert!(result["error"].as_str().unwrap().contains("not retried"));
        assert!(result.get("keymint_error_code").is_none());
        assert!(store.pop_for_b("dev", "m1", Duration::ZERO).await.is_none());
        assert_eq!(
            store
                .create_task("agree", json!({}), "dev", Some("agree-1"))
                .await,
            id
        );
        assert!(store
            .complete_task(&id, json!({"data": "AQ=="}), "dev")
            .await
            .is_err());
        assert_eq!(store.counts().await.failed, 1);
    }

    #[tokio::test]
    async fn lost_agreement_generation_is_not_requeued_but_signing_generation_is() {
        for (payload, agreement) in [
            (json!({"purpose": [6]}), true),
            (json!({"device_attest_context": {"purpose": [6]}}), true),
            (
                json!({"purpose": [2], "device_attest_context": {"purpose": [6]}}),
                false,
            ),
        ] {
            let store = store();
            let id = store.create_task("attest", payload, "dev", None).await;
            store.pop_for_b("dev", "m1", Duration::ZERO).await.unwrap();
            let mut inner = store.inner.lock().await;
            inner.tasks.get_mut(&id).unwrap().assigned_at_ms = 1;
            inner.devices.get_mut("dev").unwrap().last_seen_ms = 1;
            TaskStore::reclaim_locked(&mut inner, Duration::ZERO);
            assert_eq!(
                inner.tasks[&id].status,
                if agreement {
                    TaskStatus::Failed
                } else {
                    TaskStatus::Pending
                }
            );
            assert_eq!(
                inner.pending_by_device.get("dev").map_or(0, |q| q.len()),
                usize::from(!agreement)
            );
        }
    }

    #[tokio::test]
    async fn result_must_match_the_active_assignment() {
        let store = store();
        let task_id = store
            .create_task("attest", json!({}), "dev", Some("request-2"))
            .await;
        store
            .pop_for_b("dev", "machine", Duration::from_millis(1))
            .await
            .expect("assign task");

        assert!(store
            .complete_task(&task_id, json!({"ok": true}), "other")
            .await
            .is_err());
        store
            .complete_task(&task_id, json!({"ok": true}), "dev")
            .await
            .expect("correct device completes");
        assert!(store
            .complete_task(&task_id, json!({"ok": true}), "dev")
            .await
            .is_err());
    }

    /// An unrelated completion or enqueue must not wake a waiter: each open
    /// task owns its own channel, and the shared `notify` is only for the
    /// B-side long poll.
    #[tokio::test]
    async fn independent_tasks_do_not_wake_or_poll_each_other() {
        use std::future::Future;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::task::{Context, Poll, Wake, Waker};

        #[derive(Default)]
        struct WakeCount(AtomicUsize);
        impl Wake for WakeCount {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            fn wake_by_ref(self: &Arc<Self>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let store = store();
        let a = create_assigned(&store).await;
        let b = create_assigned(&store).await;
        let count = Arc::new(WakeCount::default());
        let waker = Waker::from(count.clone());
        let mut cx = Context::from_waker(&waker);
        let mut wait = Box::pin(store.wait_for_result(&a, Duration::from_secs(5)));
        assert!(wait.as_mut().poll(&mut cx).is_pending());
        store
            .complete_task(&b, json!({"ok": "b"}), "dev")
            .await
            .expect("complete b");
        store.create_task("attest", json!({}), "dev", None).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            count.0.load(Ordering::SeqCst),
            0,
            "an unrelated completion must not wake this waiter"
        );
        let result = json!({"ok": "a"});
        store
            .complete_task(&a, result.clone(), "dev")
            .await
            .expect("complete a");
        assert_eq!(wait.as_mut().poll(&mut cx), Poll::Ready(Some(result)));
    }

    /// A result published before the task is pruned stays readable for a
    /// waiter that has not been scheduled yet.
    #[tokio::test]
    async fn a_published_result_survives_pruning() {
        let store = store();
        let id = create_assigned(&store).await;
        let waiter = store.clone();
        let task_id = id.clone();
        let join = tokio::spawn(async move {
            waiter
                .wait_for_result(&task_id, Duration::from_secs(2))
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        store
            .complete_task(&id, json!({"ok": true}), "dev")
            .await
            .expect("complete");
        {
            // Prune the completed task out of the table before the waiter runs.
            let mut inner = store.inner.lock().await;
            inner.completed_queue.clear();
            TaskStore::remove_task_locked(&mut inner, &id);
        }
        let result = join.await.expect("join").expect("result");
        assert_eq!(result.get("ok"), Some(&json!(true)));
    }

    /// Load is summed over the activity window only: the queue itself is
    /// pruned lazily, so a device that went quiet must not read as busy.
    #[tokio::test]
    async fn device_load_ignores_events_outside_the_activity_window() {
        let store = store();
        {
            let mut inner = store.inner.lock().await;
            let now = TaskStore::now_ms();
            let events = inner.device_events.entry("dev".to_string()).or_default();
            events.push_back((now.saturating_sub(ACTIVITY_WINDOW_MS + 1_000), 5));
            events.push_back((now.saturating_sub(1_000), 3));
        }
        assert_eq!(store.get_device_load("dev").await, 3);
        assert_eq!(store.get_device_load("other").await, 0);
    }
}
