use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use rsbinder::{Interface, Status};

use super::*;
use crate::{
    android::hardware::security::keymint::SecurityLevel::SecurityLevel,
    android::hardware::security::keymint::Tag::Tag, android::system::keystore2::Domain::Domain,
    android::system::keystore2::IKeystoreOperation::IKeystoreOperation as AospKeystoreOperation,
    android::system::keystore2::KeyDescriptor::KeyDescriptor,
};

pub(super) fn ensure_binder_process_state() {
    let _ = rsbinder::ProcessState::init_default();
}

pub(super) fn sample_key_descriptor() -> KeyDescriptor {
    KeyDescriptor {
        domain: Domain::APP,
        nspace: 7,
        alias: Some("alias".to_string()),
        blob: None,
    }
}

pub(super) fn sample_service_requests() -> Vec<ParsedServiceRequest> {
    vec![
        ParsedServiceRequest::GetSecurityLevel {
            security_level: SecurityLevel::TRUSTED_ENVIRONMENT,
        },
        ParsedServiceRequest::GetKeyEntry {
            key: sample_key_descriptor(),
        },
        ParsedServiceRequest::UpdateSubcomponent {
            key: sample_key_descriptor(),
            public_cert: None,
            certificate_chain: None,
        },
        ParsedServiceRequest::ListEntries {
            domain: Domain::APP,
            nspace: 0,
        },
        ParsedServiceRequest::DeleteKey {
            key: sample_key_descriptor(),
        },
        ParsedServiceRequest::Grant {
            key: sample_key_descriptor(),
            grantee_uid: 12345,
            access_vector: 7,
        },
        ParsedServiceRequest::Ungrant {
            key: sample_key_descriptor(),
            grantee_uid: 12345,
        },
        ParsedServiceRequest::GetNumberOfEntries {
            domain: Domain::APP,
            nspace: 0,
        },
        ParsedServiceRequest::ListEntriesBatched {
            domain: Domain::APP,
            nspace: 0,
            starting_past_alias: Some("alias".to_string()),
        },
        ParsedServiceRequest::GetSupplementaryAttestationInfo {
            tag: Tag::MODULE_HASH,
        },
    ]
}

pub(super) fn disabled_intercept_config() -> config::InterceptConfig {
    config::InterceptConfig {
        get_security_level: false,
        get_key_entry: false,
        update_subcomponent: false,
        list_entries: false,
        delete_key: false,
        grant: false,
        ungrant: false,
        get_number_of_entries: false,
        list_entries_batched: false,
        get_supplementary_attestation_info: false,
    }
}

pub(super) struct TestOperationBackend {
    pub(super) update_output: Vec<u8>,
    pub(super) aborts: Arc<AtomicUsize>,
    pub(super) update_aad_status: Option<Status>,
}

impl Interface for TestOperationBackend {}

impl AospKeystoreOperation for TestOperationBackend {
    fn r#updateAad(&self, _aad_input: &[u8]) -> rsbinder::status::Result<()> {
        match self.update_aad_status.as_ref() {
            Some(status) => Err(status.clone()),
            None => Ok(()),
        }
    }

    fn r#update(&self, _input: &[u8]) -> rsbinder::status::Result<Option<Vec<u8>>> {
        Ok(Some(self.update_output.clone()))
    }

    fn r#finish(
        &self,
        _input: Option<&[u8]>,
        _signature: Option<&[u8]>,
    ) -> rsbinder::status::Result<Option<Vec<u8>>> {
        Ok(Some(self.update_output.clone()))
    }

    fn r#abort(&self) -> rsbinder::status::Result<()> {
        self.aborts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

pub(super) fn raw_parts(reply: &mut parcel::OwnedReply) -> (*mut u8, usize, *mut usize, usize) {
    (
        reply.data_mut_ptr(),
        reply.data_size(),
        if reply.offsets.is_empty() {
            std::ptr::null_mut()
        } else {
            reply.offsets.as_mut_ptr()
        },
        reply.offsets_size(),
    )
}

pub(super) fn carrier_target(carrier: &parcel::ReplyBinderCarrier) -> LocalBinderTarget {
    unsafe { parse_local_binder_target_from_parcel_bytes(&carrier.bytes) }
        .expect("synthetic carrier should expose a native target")
}

pub(super) fn request_parcel(interface: &str) -> rsbinder::Parcel {
    request_parcel_with_marker(interface, rsbinder::INTERFACE_HEADER)
}

pub(super) fn request_parcel_with_marker(interface: &str, marker: u32) -> rsbinder::Parcel {
    let mut parcel = rsbinder::Parcel::new();
    parcel.write(&0i32).unwrap();
    parcel.write(&0i32).unwrap();
    parcel.write(&marker).unwrap();
    parcel.write(&interface.to_string()).unwrap();
    parcel
}

pub(super) fn transaction_for_parcel(
    target: LocalBinderTarget,
    code: rsbinder::TransactionCode,
    parcel: &rsbinder::Parcel,
) -> binder_transaction_data {
    let mut tr: binder_transaction_data = unsafe { std::mem::zeroed() };
    tr.target.ptr = target.ptr;
    tr.data.ptr.buffer = parcel.as_ptr() as libc::c_ulong;
    tr.data.ptr.offsets = 0;
    tr.cookie = target.cookie;
    tr.code = code;
    tr.sender_euid = 10002;
    tr.sender_pid = 2000;
    tr.data_size = parcel.data_size();
    tr.offsets_size = 0;
    tr
}

static ROUTE_STATE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(in crate::hook) fn route_state_test_guard() -> (
    std::sync::MutexGuard<'static, ()>,
    std::sync::MutexGuard<'static, ()>,
) {
    let tracker_guard = tracker::state_test_guard();
    let route_guard = ROUTE_STATE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    super::mirror::reset_mirror_state_for_tests();
    reset_route_state_for_tests();
    (tracker_guard, route_guard)
}

pub(super) fn reset_route_state_for_tests() {
    tracker::clear_state_for_tests();
    clear_operation_state_for_tests();
    super::pending::reset_pending_state_for_tests();
}

pub(super) fn clear_operation_state_for_tests() {
    super::synthetic::reset_state_for_tests();
}

#[test]
fn ommega_grant_descriptor_rule_covers_grant_and_key_id() {
    let _guard = route_state_test_guard();
    let caller = CallerInfo {
        uid: 99_014,
        sid: "u:r:isolated_app:s0".to_string(),
        pid: 4242,
    };
    let unknown_package = filter::FilterDecision {
        allowed: false,
        reason: FilterReason::RejectedUnknownPackage,
        packages: Vec::new(),
    };
    let probes = AtomicUsize::new(0);

    // Both descriptor shapes a grant hands out must be probed through ommega.
    for domain in [Domain::GRANT, Domain::KEY_ID] {
        let descriptor = KeyDescriptor {
            domain,
            nspace: 7,
            alias: None,
            blob: None,
        };
        let mut probe = |_: &CallerInfo, _: &KeyDescriptor| {
            probes.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        };
        assert!(
            should_allow_ommega_grant_descriptor_with_probe(
                &descriptor,
                &unknown_package,
                &caller,
                &mut probe
            )
            .unwrap(),
            "{domain:?} descriptor must be routed through the ommega grant probe"
        );
    }
    assert_eq!(
        probes.load(Ordering::SeqCst),
        2,
        "each accepted descriptor shape must be probed"
    );

    // A key id ommega does not grant stays with the system.
    let unknown_key_id = KeyDescriptor {
        domain: Domain::KEY_ID,
        nspace: 9,
        alias: None,
        blob: None,
    };
    let mut deny = |_: &CallerInfo, _: &KeyDescriptor| Ok(false);
    assert!(!should_allow_ommega_grant_descriptor_with_probe(
        &unknown_key_id,
        &unknown_package,
        &caller,
        &mut deny
    )
    .unwrap());

    // Other descriptor shapes and already-allowed callers keep the previous behavior.
    let app_descriptor = KeyDescriptor {
        domain: Domain::APP,
        nspace: 7,
        alias: Some("alias".to_string()),
        blob: None,
    };
    let not_probed = AtomicUsize::new(0);
    let mut probe = |_: &CallerInfo, _: &KeyDescriptor| {
        not_probed.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    };
    assert!(!should_allow_ommega_grant_descriptor_with_probe(
        &app_descriptor,
        &unknown_package,
        &caller,
        &mut probe
    )
    .unwrap());
    let allowed = filter::FilterDecision {
        allowed: true,
        reason: FilterReason::Allowed,
        packages: vec!["com.wu.kk".to_string()],
    };
    assert!(!should_allow_ommega_grant_descriptor_with_probe(
        &unknown_key_id,
        &allowed,
        &caller,
        &mut probe
    )
    .unwrap());
    assert_eq!(
        not_probed.load(Ordering::SeqCst),
        0,
        "descriptors the rule does not accept must not reach the probe"
    );
}
