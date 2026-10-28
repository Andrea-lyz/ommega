//! Capture a verified remote attestation and count it only if key creation succeeds.
use std::{
    cell::RefCell,
    collections::BTreeMap,
    marker::PhantomData,
    path::Path,
    rc::Rc,
    sync::{
        mpsc::{self, SyncSender},
        OnceLock,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use der::Decode;

use crate::{attestation_counts, plat::aaid::AttestationApplicationId};

thread_local! {
    static REQUESTS: RefCell<Vec<RequestState>> = const { RefCell::new(Vec::new()) };
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    packages: Vec<String>,
    latency_ms: Option<u64>,
}

#[derive(Default)]
struct RequestState {
    response_received: bool,
    verified: bool,
    observation: Option<Observation>,
}

struct PersistEvent {
    observation: Observation,
    completed_at_ms: i64,
}
static COUNT_SENDER: OnceLock<Option<SyncSender<PersistEvent>>> = OnceLock::new();

fn count_sender() -> Option<&'static SyncSender<PersistEvent>> {
    COUNT_SENDER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<PersistEvent>(1024);
            match thread::Builder::new()
                .name("ommega-attest-count".into())
                .spawn(move || {
                    while let Ok(first) = receiver.recv() {
                        let mut counts: BTreeMap<String, attestation_counts::CountDelta> =
                            BTreeMap::new();
                        let mut latest = None;
                        for event in std::iter::once(first).chain(receiver.try_iter().take(1023)) {
                            let shared = event.observation.packages.len() > 1;
                            for package in event.observation.packages {
                                let delta = counts.entry(package).or_default();
                                delta.count = delta.count.saturating_add(1);
                                delta.shared |= shared;
                            }
                            if latest
                                .as_ref()
                                .is_none_or(|(_, at)| event.completed_at_ms >= *at)
                            {
                                latest =
                                    Some((event.observation.latency_ms, event.completed_at_ms));
                            }
                        }
                        for attempt in 0..3 {
                            match attestation_counts::apply_batch_at(
                                Path::new(attestation_counts::DATABASE_PATH),
                                &counts,
                                latest,
                            ) {
                                Ok(()) => break,
                                Err(error) if attempt < 2 && is_busy(&error) => {
                                    thread::sleep(Duration::from_millis(50 * (attempt + 1)))
                                }
                                Err(error) => {
                                    log::warn!("could not persist attestation count: {error}");
                                    break;
                                }
                            }
                        }
                    }
                }) {
                Ok(_) => Some(sender),
                Err(error) => {
                    log::warn!("could not start attestation count worker: {error}");
                    None
                }
            }
        })
        .as_ref()
}

fn is_busy(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(code, _) if matches!(code.code,
        rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
}

/// A synchronous request guard. The stack handles nested operations without
/// allowing one request's attestation to be attributed to another.
pub struct RequestGuard {
    active: bool,
    _same_thread: PhantomData<Rc<()>>,
}

pub fn begin_request() -> RequestGuard {
    REQUESTS.with(|requests| requests.borrow_mut().push(RequestState::default()));
    RequestGuard {
        active: true,
        _same_thread: PhantomData,
    }
}

pub fn clear_attempt() {
    REQUESTS.with(|requests| {
        if let Some(current) = requests.borrow_mut().last_mut() {
            current.verified = false;
            current.observation = None;
        }
    });
}

fn caller_packages(app_id_der: &[u8]) -> Option<Vec<String>> {
    let app_id = AttestationApplicationId::from_der(app_id_der).ok()?;
    if app_id.package_info_records.is_empty() || app_id.package_info_records.len() > 64 {
        return None;
    }
    let mut packages = Vec::new();
    for record in app_id.package_info_records.iter() {
        let package = std::str::from_utf8(record.package_name.as_bytes()).ok()?;
        if !kmr_common::cpu_mask::valid_package(package) {
            return None;
        }
        packages.push(package.to_owned());
    }
    packages.sort();
    packages.dedup();
    Some(packages)
}

pub fn mark_response_received() {
    REQUESTS.with(|requests| {
        if let Some(current) = requests.borrow_mut().last_mut() {
            current.response_received = true;
        }
    });
}

pub fn mark_verified(app_id_der: &[u8], latency_ms: Option<u64>) {
    let observation = caller_packages(app_id_der).map(|packages| Observation {
        packages,
        latency_ms,
    });
    REQUESTS.with(|requests| {
        if let Some(current) = requests.borrow_mut().last_mut() {
            current.verified = true;
            current.observation = observation;
        }
    });
}

impl RequestGuard {
    fn finish(&mut self) -> RequestState {
        self.active = false;
        REQUESTS.with(|requests| requests.borrow_mut().pop().unwrap_or_default())
    }

    #[cfg(test)]
    fn take(&mut self) -> Option<Observation> {
        self.finish().observation
    }

    pub fn commit(mut self) {
        let state = self.finish();
        if state.response_received {
            if state.verified {
                super::health::record_remote_ok();
            } else {
                report_invalid_response();
            }
        }
        let Some(observation) = state.observation else {
            return;
        };
        let completed_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or(0);
        // A bounded, nonblocking queue keeps disk I/O and worker failures away
        // from the binder/TEE transaction. Dropped observations are reported.
        if let Some(sender) = count_sender() {
            if let Err(error) = sender.try_send(PersistEvent {
                observation,
                completed_at_ms,
            }) {
                log::warn!("could not queue attestation count: {error}");
            }
        }
    }
}

fn report_invalid_response() {
    super::health::record_attestation_failure();
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if self.active && self.finish().response_received {
            report_invalid_response();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plat::aaid::PackageInfoRecord;
    use der::{
        asn1::{OctetString, SetOfVec},
        Encode,
    };

    fn app_id(packages: &[&str]) -> Vec<u8> {
        let records = packages.iter().map(|package| PackageInfoRecord {
            package_name: OctetString::new(package.as_bytes()).unwrap(),
            version: 1,
        });
        AttestationApplicationId {
            package_info_records: SetOfVec::from_iter(records).unwrap(),
            signature_digests: SetOfVec::new(),
        }
        .to_der()
        .unwrap()
    }

    #[test]
    fn guard_counts_only_the_successful_current_attempt() {
        let mut request = begin_request();
        mark_verified(&app_id(&["com.example.app"]), Some(100));
        clear_attempt();
        assert!(request.take().is_none());
        let mut request = begin_request();
        mark_verified(&app_id(&["com.example.app"]), Some(120));
        assert_eq!(
            request.take(),
            Some(Observation {
                packages: vec!["com.example.app".to_string()],
                latency_ms: Some(120)
            })
        );
    }

    #[test]
    fn shared_uid_and_failed_requests_do_not_leak_attribution() {
        let mut outer = begin_request();
        mark_verified(&app_id(&["com.example.outer"]), None);
        {
            let _failed = begin_request();
            mark_verified(&app_id(&["com.example.one", "com.example.two"]), Some(50));
        }
        assert_eq!(
            outer.take(),
            Some(Observation {
                packages: vec!["com.example.outer".to_string()],
                latency_ms: None
            })
        );
        let mut request = begin_request();
        mark_verified(&app_id(&["com.example.one", "com.example.two"]), Some(80));
        assert_eq!(
            request.take(),
            Some(Observation {
                packages: vec!["com.example.one".to_string(), "com.example.two".to_string()],
                latency_ms: Some(80)
            })
        );
    }

    #[test]
    fn response_and_verification_follow_the_final_attempt() {
        let mut request = begin_request();
        mark_response_received();
        mark_verified(&app_id(&["com.example.app"]), Some(100));
        clear_attempt();
        let state = request.finish();
        assert!(state.response_received);
        assert!(!state.verified);
        assert!(state.observation.is_none());

        let mut request = begin_request();
        mark_response_received();
        mark_verified(b"not-der", Some(100));
        let state = request.finish();
        assert!(state.response_received);
        assert!(state.verified);
        assert!(state.observation.is_none());
    }

    #[test]
    fn malformed_id_and_failed_request_leave_no_count() {
        let request = begin_request();
        mark_verified(&app_id(&["com.example.failed"]), Some(30));
        drop(request);
        let mut next = begin_request();
        mark_verified(b"not-der", Some(30));
        assert!(next.take().is_none());
    }
}
