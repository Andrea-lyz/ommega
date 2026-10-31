//! Software-only scheduling tests: no listener, database, HAL or device.
use super::*;
use serde_json::json;
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

fn store() -> Arc<TaskStore> {
    TaskStore::new(1, 60, 100, 60)
}

#[tokio::test]
async fn targeted_enqueue_and_completion_leave_unrelated_polls_asleep() {
    let store = store();
    let done = store.create_task("attest", json!({}), "busy", None).await;
    store.pop_for_b("busy", "m", Duration::ZERO).await.unwrap();
    let count = Arc::new(WakeCount::default());
    let waker = Waker::from(count.clone());
    let mut cx = Context::from_waker(&waker);
    let mut idle = Box::pin(store.pop_for_b("idle", "m", Duration::from_secs(20)));
    assert!(idle.as_mut().poll(&mut cx).is_pending());
    store.create_task("attest", json!({}), "busy", None).await;
    store
        .complete_task(&done, json!({"ok": true}), "busy")
        .await
        .unwrap();
    assert_eq!(count.0.load(Ordering::SeqCst), 0);
    let target = store.create_task("attest", json!({}), "idle", None).await;
    assert!(count.0.load(Ordering::SeqCst) > 0);
    match idle.as_mut().poll(&mut cx) {
        Poll::Ready(Some(task)) => assert_eq!(task.task_id, target),
        other => panic!("targeted arrival did not wake: {other:?}"),
    }
}

#[tokio::test]
async fn wildcard_wakes_both_devices_but_each_task_has_one_claim() {
    let store = store();
    let a_count = Arc::new(WakeCount::default());
    let b_count = Arc::new(WakeCount::default());
    let a_waker = Waker::from(a_count.clone());
    let b_waker = Waker::from(b_count.clone());
    let mut a_cx = Context::from_waker(&a_waker);
    let mut b_cx = Context::from_waker(&b_waker);
    let mut a = Box::pin(store.pop_for_b("a", "m", Duration::from_secs(20)));
    let mut b = Box::pin(store.pop_for_b("b", "m", Duration::from_secs(20)));
    assert!(a.as_mut().poll(&mut a_cx).is_pending());
    assert!(b.as_mut().poll(&mut b_cx).is_pending());
    let first = store.create_task("attest", json!({}), "", None).await;
    assert!(a_count.0.load(Ordering::SeqCst) > 0);
    assert!(b_count.0.load(Ordering::SeqCst) > 0);
    match a.as_mut().poll(&mut a_cx) {
        Poll::Ready(Some(task)) => assert_eq!(task.task_id, first),
        other => panic!("wildcard not claimed: {other:?}"),
    }
    assert!(b.as_mut().poll(&mut b_cx).is_pending());
    let second = store.create_task("attest", json!({}), "", None).await;
    match b.as_mut().poll(&mut b_cx) {
        Poll::Ready(Some(task)) => assert_eq!(task.task_id, second),
        other => panic!("second wildcard lost: {other:?}"),
    }
    assert_eq!(store.counts().await.assigned, 2);
}

#[tokio::test]
async fn cancelled_poll_releases_waiter_and_next_poll_sees_queued_work() {
    let store = store();
    let waker = Waker::from(Arc::new(WakeCount::default()));
    let mut cx = Context::from_waker(&waker);
    let mut poll = Box::pin(store.pop_for_b("a", "m", Duration::from_secs(20)));
    assert!(poll.as_mut().poll(&mut cx).is_pending());
    let weak = store.inner.lock().await.poll_notifiers["a"].clone();
    assert!(weak.upgrade().is_some());
    drop(poll);
    assert!(weak.upgrade().is_none());
    let id = store.create_task("attest", json!({}), "a", None).await;
    assert_eq!(
        store
            .pop_for_b("a", "m", Duration::ZERO)
            .await
            .unwrap()
            .task_id,
        id
    );
}

#[tokio::test]
async fn concurrent_same_device_polls_share_notification_without_duplicate_claims() {
    let store = store();
    let waker = Waker::from(Arc::new(WakeCount::default()));
    let mut cx = Context::from_waker(&waker);
    let mut polls: Vec<_> = (0..3)
        .map(|_| Box::pin(store.pop_for_b("a", "m", Duration::from_secs(20))))
        .collect();
    for poll in &mut polls {
        assert!(poll.as_mut().poll(&mut cx).is_pending());
    }
    let mut expected = Vec::new();
    for _ in 0..3 {
        expected.push(store.create_task("attest", json!({}), "a", None).await);
    }
    for (poll, id) in polls.iter_mut().zip(expected) {
        match poll.as_mut().poll(&mut cx) {
            Poll::Ready(Some(task)) => assert_eq!(task.task_id, id),
            other => panic!("FIFO arrival lost: {other:?}"),
        }
    }
}

#[tokio::test]
async fn reclaim_notifies_wildcard_waiter_but_never_replays_sensitive_work() {
    let store = store();
    let id = store.create_task("attest", json!({}), "", None).await;
    store.pop_for_b("dead", "m", Duration::ZERO).await.unwrap();
    let sensitive = store.create_task("agree", json!({}), "dead", None).await;
    store.pop_for_b("dead", "m", Duration::ZERO).await.unwrap();
    let count = Arc::new(WakeCount::default());
    let waker = Waker::from(count.clone());
    let mut cx = Context::from_waker(&waker);
    let mut poll = Box::pin(store.pop_for_b("live", "m", Duration::from_secs(20)));
    assert!(poll.as_mut().poll(&mut cx).is_pending());
    {
        let mut inner = store.inner.lock().await;
        inner.devices.get_mut("dead").unwrap().last_seen_ms = 0;
        inner.tasks.get_mut(&id).unwrap().assigned_at_ms = 0;
        inner.tasks.get_mut(&sensitive).unwrap().assigned_at_ms = 0;
        TaskStore::reclaim_locked(&mut inner, Duration::ZERO);
        assert_eq!(inner.tasks[&sensitive].status, TaskStatus::Failed);
    }
    assert!(count.0.load(Ordering::SeqCst) > 0);
    match poll.as_mut().poll(&mut cx) {
        Poll::Ready(Some(task)) => assert_eq!(task.task_id, id),
        other => panic!("reclaimed wildcard lost: {other:?}"),
    }
}
