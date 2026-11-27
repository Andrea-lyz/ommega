use std::mem::size_of;

use rsbinder::{ExceptionCode, Status, StatusCode};

use super::*;
use crate::android::hardware::security::keymint::SecurityLevel::SecurityLevel;
use crate::android::system::keystore2::{
    KeyEntryResponse::KeyEntryResponse, KeyMetadata::KeyMetadata,
};
use crate::hook::rewrite::tests::*;
fn unavailable_test_caller() -> CallerInfo {
    CallerInfo {
        uid: 1000,
        sid: String::new(),
        pid: 2000,
    }
}

pub(super) fn assert_unknown_transaction_reply(reply: SyntheticReply) {
    let SyntheticReply::Status(status) = reply else {
        panic!("unknown transaction should be returned as a binder status code");
    };
    assert_eq!(status, i32::from(StatusCode::UnknownTransaction));
}

pub(super) fn assert_synthetic_status(reply: SyntheticReply, expected: StatusCode) {
    let SyntheticReply::Status(status) = reply else {
        panic!("expected synthetic binder status");
    };
    assert_eq!(status, i32::from(expected));
}

pub(super) fn assert_synthetic_ok_reply(reply: SyntheticReply, label: &str) {
    let SyntheticReply::Parcel(mut reply) = reply else {
        panic!("{label} should be a status-bearing parcel reply");
    };
    assert_eq!(reply.offsets_size(), 0);
    let (data, data_size, offsets, offsets_size) = raw_parts(&mut reply);
    let status = unsafe { parcel::parse_reply_status(data, data_size, offsets, offsets_size) }
        .expect("status reply should parse");
    assert!(status.is_ok(), "{label} should be OK, got {status:?}");
}

pub(super) fn assert_synthetic_empty_parcel_reply(reply: SyntheticReply, label: &str) {
    let SyntheticReply::Parcel(reply) = reply else {
        panic!("{label} should be an empty parcel reply");
    };
    assert_eq!(reply.data_size(), 0, "{label} data size");
    assert_eq!(reply.offsets_size(), 0, "{label} offsets size");
}

pub(super) fn assert_synthetic_exception_reply(reply: SyntheticReply, expected: ExceptionCode) {
    let SyntheticReply::Parcel(mut reply) = reply else {
        panic!("expected synthetic status parcel reply");
    };
    let (data, data_size, offsets, offsets_size) = raw_parts(&mut reply);
    let status = unsafe { parcel::parse_reply_status(data, data_size, offsets, offsets_size) }
        .expect("status reply should parse");
    assert_eq!(status.exception_code(), expected);
}

pub(super) fn assert_synthetic_raw_i32_reply(reply: SyntheticReply, expected: i32) {
    let SyntheticReply::Parcel(mut reply) = reply else {
        panic!("raw i32 reply should be a parcel");
    };
    assert_eq!(reply.data_size(), size_of::<i32>());
    assert_eq!(reply.offsets_size(), 0);
    let (data, _, _, _) = raw_parts(&mut reply);
    let value = unsafe { std::ptr::read_unaligned(data as *const i32) };
    assert_eq!(value, expected);
}

#[test]
fn plain_ommega_error_becomes_system_error_reply() {
    let error = anyhow::anyhow!("plain ommega failure");
    let mut reply =
        build_ommega_error_reply(&error).expect("plain error should produce a status reply");
    let (data, data_size, offsets, offsets_size) = raw_parts(&mut reply);
    let parsed = unsafe { parcel::parse_reply_status(data, data_size, offsets, offsets_size) }
        .expect("status reply should parse");

    assert_eq!(
        parsed.exception_code(),
        rsbinder::ExceptionCode::ServiceSpecific
    );
    assert_eq!(
        parsed.service_specific_error(),
        ResponseCode::SYSTEM_ERROR.0
    );
}

#[test]
fn contextual_ommega_status_error_keeps_service_specific_code() {
    let status = Status::new_service_specific_error(ResponseCode::PERMISSION_DENIED.0, None);
    let error = anyhow::Error::new(status).context("wrapped ommega failure");
    let mut reply =
        build_ommega_error_reply(&error).expect("wrapped status should produce a status reply");
    let (data, data_size, offsets, offsets_size) = raw_parts(&mut reply);
    let parsed = unsafe { parcel::parse_reply_status(data, data_size, offsets, offsets_size) }
        .expect("status reply should parse");

    assert_eq!(
        parsed.exception_code(),
        rsbinder::ExceptionCode::ServiceSpecific
    );
    assert_eq!(
        parsed.service_specific_error(),
        ResponseCode::PERMISSION_DENIED.0
    );
}

#[test]
fn unavailable_ommega_errors_preserve_system_reply() {
    for status in [
        StatusCode::DeadObject,
        StatusCode::RpcError,
        StatusCode::NotEnoughData,
        StatusCode::NoInit,
        StatusCode::NameNotFound,
        StatusCode::Errno(libc::ECONNREFUSED),
        StatusCode::Errno(libc::EPIPE),
    ] {
        let error = anyhow::Error::new(Status::from(status));
        assert!(
            ommega_error_reply_for_method("test", &unavailable_test_caller(), &error)
                .expect("unavailable ommega errors should classify cleanly")
                .is_some(),
            "{status:?} after retry means ommega is unavailable and returns SYSTEM_ERROR"
        );
    }

    for (message, unavailable) in [
        ("failed to connect to ommega service", true),
        ("failed to connect to ommega_maintenance service", true),
        ("failed to connect to ommega_authorization service", true),
        (
            "failed to connect to ommega service: permission denied",
            false,
        ),
        (
            "failed to connect to ommega_maintenance service: permission denied",
            false,
        ),
        (
            "failed to connect to ommega_authorization service: permission denied",
            false,
        ),
    ] {
        assert_eq!(
            ommega_unavailable_error(&anyhow::anyhow!(message)),
            unavailable,
            "ommega connection marker classification mismatch for {message}"
        );
    }

    for marker in [
        "failed to connect to ommega service",
        "failed to connect to ommega_maintenance service",
        "failed to connect to ommega_authorization service",
    ] {
        let error = anyhow::Error::new(StatusCode::PermissionDenied).context(marker);
        assert!(
            !ommega_unavailable_error(&error),
            "typed permission error must override connection marker {marker}"
        );
    }
    let stale = anyhow::Error::new(StatusCode::DeadObject)
        .context("failed to connect to ommega_authorization service");
    assert!(ommega_unavailable_error(&stale));

    let missing_service = anyhow::Error::new(StatusCode::NameNotFound);
    assert!(
        ommega_error_reply_for_method("test", &unavailable_test_caller(), &missing_service)
            .expect("missing RPC service should classify cleanly")
            .is_some(),
        "missing ommega RPC service means ommega is unavailable and returns SYSTEM_ERROR"
    );

    for status in [
        StatusCode::DeadObject,
        StatusCode::RpcError,
        StatusCode::NotEnoughData,
    ] {
        for error in [
            anyhow::Error::new(Status::from(status)).context("wrapped status"),
            anyhow::Error::new(status).context("wrapped status code"),
        ] {
            assert!(
                ommega_error_reply_for_method("test", &unavailable_test_caller(), &error)
                    .expect("wrapped unavailable errors should classify cleanly")
                    .is_some()
            );
        }
    }

    let local = anyhow::anyhow!("plain ommega failure");
    assert!(
        ommega_error_reply_for_method("test", &unavailable_test_caller(), &local)
            .expect("plain ommega errors should classify cleanly")
            .is_some(),
        "non-connection ommega errors must be returned instead of preserving system"
    );
}

#[test]
fn reachable_non_stale_ommega_status_code_errors_become_system_error_reply() {
    // AOSP keystore2 maps a bare transport StatusCode through
    // map_binder_status_code -> Error::BinderTransaction -> SYSTEM_ERROR,
    // surfaced as a service-specific parcel, never a raw transport status.
    for status in [
        StatusCode::TimedOut,
        StatusCode::PermissionDenied,
        StatusCode::UnknownTransaction,
    ] {
        let error = anyhow::Error::new(status);
        let mut reply = ommega_error_reply_for_method("test", &unavailable_test_caller(), &error)
            .expect("reachable ommega errors should classify cleanly")
            .expect("non-stale ommega errors must replace system reply");
        let (data, data_size, offsets, offsets_size) = raw_parts(&mut reply);
        let parsed = unsafe { parcel::parse_reply_status(data, data_size, offsets, offsets_size) }
            .expect("status reply should parse");
        assert_eq!(
            parsed.exception_code(),
            rsbinder::ExceptionCode::ServiceSpecific
        );
        assert_eq!(
            parsed.service_specific_error(),
            ResponseCode::SYSTEM_ERROR.0
        );
    }
}

#[test]
fn reachable_ommega_status_error_becomes_authoritative_reply() {
    let status = Status::new_service_specific_error(ResponseCode::PERMISSION_DENIED.0, None);
    let reply = ommega_status_reply_for_method("test", &unavailable_test_caller(), &status)
        .expect("service-specific status should build")
        .expect("reachable ommega status should replace system");
    let mut reply = reply;
    let (data, data_size, offsets, offsets_size) = raw_parts(&mut reply);
    let parsed = unsafe { parcel::parse_reply_status(data, data_size, offsets, offsets_size) }
        .expect("status reply should parse");

    assert_eq!(
        parsed.exception_code(),
        rsbinder::ExceptionCode::ServiceSpecific
    );
    assert_eq!(
        parsed.service_specific_error(),
        ResponseCode::PERMISSION_DENIED.0
    );
}

fn key_entry_response_reply(
    key_security_level: crate::android::hardware::security::keymint::SecurityLevel::SecurityLevel,
) -> parcel::OwnedReply {
    parcel::build_key_entry_reply(KeyEntryResponse {
        r#iSecurityLevel: None,
        r#metadata: KeyMetadata {
            r#key: sample_key_descriptor(),
            r#keySecurityLevel: key_security_level,
            r#authorizations: Vec::new(),
            r#certificate: None,
            r#certificateChain: None,
            r#modificationTimeMs: 0,
        },
    })
    .expect("key entry reply should serialize")
}

fn key_entry_pending() -> PendingServiceCall {
    PendingServiceCall {
        request: ParsedServiceRequest::GetKeyEntry {
            key: sample_key_descriptor(),
        },
        caller: CallerInfo {
            uid: 1000,
            sid: String::new(),
            pid: 2000,
        },
        packages: vec!["com.example".to_string()],
        route: RouteTarget::System,
    }
}

fn transaction_from_reply(reply: &mut parcel::OwnedReply) -> binder_transaction_data {
    let (data, data_size, offsets, offsets_size) = raw_parts(reply);
    let mut tr: binder_transaction_data = unsafe { std::mem::zeroed() };
    tr.data_size = data_size;
    tr.offsets_size = offsets_size;
    tr.data.ptr.buffer = data as libc::c_ulong;
    tr.data.ptr.offsets = offsets as libc::c_ulong;
    tr
}

#[test]
fn system_strongbox_key_entry_reply_is_replaced_when_feature_hidden() {
    let mut reply = key_entry_response_reply(SecurityLevel::STRONGBOX);
    let tr = transaction_from_reply(&mut reply);
    let pending = key_entry_pending();

    let rewritten = unsafe {
        observe_system_service_reply_with_probe(&tr, &pending, || true)
            .expect("observe should succeed")
    }
    .expect("StrongBox reply must be replaced");
    let mut rewritten = rewritten;
    let (data, data_size, offsets, offsets_size) = raw_parts(&mut rewritten);
    let status = unsafe { parcel::parse_reply_status(data, data_size, offsets, offsets_size) }
        .expect("status reply should parse");

    assert_eq!(status.exception_code(), ExceptionCode::ServiceSpecific);
    assert_eq!(
        status.service_specific_error(),
        ResponseCode::SYSTEM_ERROR.0
    );
}

#[test]
fn system_strongbox_key_entry_reply_is_kept_when_feature_visible() {
    let mut reply = key_entry_response_reply(SecurityLevel::STRONGBOX);
    let tr = transaction_from_reply(&mut reply);
    let pending = key_entry_pending();

    let result =
        unsafe { observe_system_service_reply_with_probe(&tr, &pending, || false) }.expect("ok");
    assert!(
        result.is_none(),
        "visible StrongBox reply must pass through"
    );
}

#[test]
fn system_tee_key_entry_reply_is_kept_without_probing_mask() {
    let mut reply = key_entry_response_reply(SecurityLevel::TRUSTED_ENVIRONMENT);
    let tr = transaction_from_reply(&mut reply);
    let pending = key_entry_pending();

    let result = unsafe {
        observe_system_service_reply_with_probe(&tr, &pending, || {
            panic!("mask probe must not run for non-StrongBox replies")
        })
    }
    .expect("ok");
    assert!(result.is_none(), "TEE reply must pass through");
}

#[test]
fn system_error_key_entry_reply_is_left_untouched() {
    let mut reply = parcel::build_status_reply(&Status::new_service_specific_error(
        ResponseCode::PERMISSION_DENIED.0,
        None,
    ))
    .expect("error status reply should serialize");
    let tr = transaction_from_reply(&mut reply);
    let pending = key_entry_pending();

    let result = unsafe {
        observe_system_service_reply_with_probe(&tr, &pending, || {
            panic!("mask probe must not run for error replies")
        })
    }
    .expect("ok");
    assert!(
        result.is_none(),
        "error replies must pass through unchanged"
    );
}
