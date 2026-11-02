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

use crate::{attestation_counts, attestation_counts::Source, plat::aaid::AttestationApplicationId};

// Android uses OS-backed TLS: even a const block expands into a runtime init
// function, which Clippy 1.96 incorrectly flags as missing const. Default makes
// the same empty per-thread stack without that affected macro expansion.
thread_local! {
    static REQUESTS: RefCell<Vec<RequestState>> = RefCell::default();
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    packages: Vec<String>,
    latency_ms: Option<u64>,
}

struct RequestState {
    response_received: bool,
    verified: bool,
    /// Which counter table this request belongs to. Defaults to the relay so a
    /// request that never says otherwise is never counted as local.
    source: Source,
    observation: Option<Observation>,
}

impl Default for RequestState {
    fn default() -> Self {
        Self {
            response_received: false,
            verified: false,
            source: Source::Remote,
            observation: None,
        }
    }
}

struct PersistEvent {
    observation: Observation,
    completed_at_ms: i64,
    source: Source,
}

/// One pending write per counter source.
#[derive(Default)]
struct Batch {
    counts: BTreeMap<String, attestation_counts::CountDelta>,
    latest: Option<(Option<u64>, i64)>,
}

impl Batch {
    fn add(&mut self, observation: &Observation, completed_at_ms: i64) {
        let shared = observation.packages.len() > 1;
        for package in &observation.packages {
            let delta = self.counts.entry(package.clone()).or_default();
            delta.count = delta.count.saturating_add(1);
            delta.shared |= shared;
        }
        if self
            .latest
            .as_ref()
            .is_none_or(|(_, at)| completed_at_ms >= *at)
        {
            self.latest = Some((observation.latency_ms, completed_at_ms));
        }
    }
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
                        let mut remote = Batch::default();
                        let mut local = Batch::default();
                        for event in std::iter::once(first).chain(receiver.try_iter().take(1023)) {
                            let batch = match event.source {
                                Source::Remote => &mut remote,
                                Source::Local => &mut local,
                            };
                            batch.add(&event.observation, event.completed_at_ms);
                        }
                        for (source, batch) in [(Source::Remote, remote), (Source::Local, local)] {
                            if batch.counts.is_empty() && batch.latest.is_none() {
                                continue;
                            }
                            for attempt in 0..3 {
                                match attestation_counts::apply_batch_at(
                                    source,
                                    Path::new(attestation_counts::DATABASE_PATH),
                                    &batch.counts,
                                    batch.latest,
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

/// Record an attestation the software TA minted locally. No latency is
/// reported: there is no round trip, so the relay latency stays untouched and
/// the local counter carries its own "latest" row.
pub fn mark_local_attestation(app_id_der: &[u8]) {
    let observation = caller_packages(app_id_der).map(|packages| Observation {
        packages,
        latency_ms: None,
    });
    REQUESTS.with(|requests| {
        if let Some(current) = requests.borrow_mut().last_mut() {
            current.source = Source::Local;
            current.observation = observation;
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
                source: state.source,
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
    fn request_stacks_are_thread_local_and_scoped() {
        REQUESTS.with(|requests| assert!(requests.borrow().is_empty()));
        let mut outer = begin_request();
        mark_verified(&app_id(&["com.example.outer"]), Some(120));
        std::thread::spawn(|| {
            REQUESTS.with(|requests| assert!(requests.borrow().is_empty()));
            let mut inner = begin_request();
            mark_verified(&app_id(&["com.example.worker"]), Some(50));
            assert_eq!(
                inner.take(),
                Some(Observation {
                    packages: vec!["com.example.worker".into()],
                    latency_ms: Some(50),
                })
            );
            REQUESTS.with(|requests| assert!(requests.borrow().is_empty()));
        })
        .join()
        .unwrap();
        assert_eq!(
            outer.take(),
            Some(Observation {
                packages: vec!["com.example.outer".into()],
                latency_ms: Some(120),
            })
        );
        REQUESTS.with(|requests| assert!(requests.borrow().is_empty()));
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
    fn local_attestation_records_its_own_source_and_no_latency() {
        let mut request = begin_request();
        mark_local_attestation(&app_id(&["com.example.local"]));
        let state = request.finish();
        assert_eq!(state.source, Source::Local);
        let observation = state.observation.expect("a local attestation is counted");
        assert_eq!(observation.packages, vec!["com.example.local".to_string()]);
        assert_eq!(
            observation.latency_ms, None,
            "a local attestation has no relay round trip"
        );

        // A relay request must stay attributed to the relay counter table.
        let mut relay = begin_request();
        mark_verified(&app_id(&["com.example.relay"]), Some(70));
        let state = relay.finish();
        assert_eq!(state.source, Source::Remote);
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
