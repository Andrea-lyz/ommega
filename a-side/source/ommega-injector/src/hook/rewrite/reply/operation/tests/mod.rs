use super::*;
use crate::android::system::keystore2::IKeystoreOperation::BnKeystoreOperation;
use crate::hook::rewrite::tests::*;
use rsbinder::{Status, StatusCode};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

mod aad;
mod finalization;

#[test]
fn overlapping_calls_are_busy_without_finalizing_or_blocking_other_operations() {
    ensure_binder_process_state();
    let _state = route_state_test_guard();
    let target = LocalBinderTarget {
        ptr: 0x1234,
        cookie: 0x5678,
    };
    let other = LocalBinderTarget {
        ptr: 0x1235,
        cookie: 0x5679,
    };
    let aborts = Arc::new(AtomicUsize::new(0));
    for key in [target, other] {
        remember_operation_target(
            key,
            OperationTargetInfo {
                in_flight: Default::default(),
                route: RouteTarget::Ommega,
                aad_allowed: true,
                backend: Some(BnKeystoreOperation::new_binder(TestOperationBackend {
                    update_output: vec![7],
                    aborts: aborts.clone(),
                    update_aad_status: None,
                })),
                finalized: false,
            },
        );
    }
    let snapshot = lookup_operation_target(target).unwrap();
    let held = snapshot.in_flight.lock().unwrap();
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                for request in [
                    ParsedOperationRequest::UpdateAad { aad_input: vec![1] },
                    ParsedOperationRequest::Update { input: vec![1] },
                    ParsedOperationRequest::Finish {
                        input: None,
                        signature: None,
                    },
                    ParsedOperationRequest::Abort,
                ] {
                    let mut reply = build_operation_reply_rewrite(&PendingOperationCall {
                        request,
                        caller: CallerInfo {
                            uid: 1000,
                            sid: String::new(),
                            pid: 2000,
                        },
                        target,
                    })
                    .unwrap()
                    .unwrap();
                    let (data, size, offsets, offsets_size) = raw_parts(&mut reply);
                    let status =
                        unsafe { parcel::parse_reply_status(data, size, offsets, offsets_size) }
                            .unwrap();
                    assert_eq!(
                        status.exception_code(),
                        rsbinder::ExceptionCode::ServiceSpecific
                    );
                    assert_eq!(
                        status.service_specific_error(),
                        ResponseCode::OPERATION_BUSY.0
                    );
                    let current = lookup_operation_target(target).unwrap();
                    assert!(!current.finalized);
                    assert!(current.backend.is_some());
                }
            })
            .join()
            .unwrap();
    });
    let update = |key| {
        let mut reply = build_operation_reply_rewrite(&PendingOperationCall {
            request: ParsedOperationRequest::Update { input: vec![1] },
            caller: CallerInfo {
                uid: 1000,
                sid: String::new(),
                pid: 2000,
            },
            target: key,
        })
        .unwrap()
        .unwrap();
        let (data, size, offsets, offsets_size) = raw_parts(&mut reply);
        let output: Option<Vec<u8>> =
            unsafe { parcel::parse_success_reply(data, size, offsets, offsets_size) }.unwrap();
        assert_eq!(output, Some(vec![7]));
    };
    update(other);
    drop(held);
    update(target);
    assert_eq!(aborts.load(Ordering::SeqCst), 0);
}
