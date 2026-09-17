use super::*;
use rsbinder::{TransactionCode, FIRST_CALL_TRANSACTION};

/// Stands in for an AIDL stub that knows `implemented` and dispatches nothing
/// else; it also checks that the probe sends the interface token first.
struct KnownCodeStub {
    interface: String,
    implemented: Vec<u32>,
}

impl Transactable for KnownCodeStub {
    fn transact(
        &self,
        code: TransactionCode,
        reader: &mut Parcel,
        reply: &mut Parcel,
    ) -> rsbinder::Result<()> {
        reader.set_data_position(0);
        let token: String = reader.read()?;
        assert_eq!(
            token, self.interface,
            "the probe must send the interface token"
        );
        if self.implemented.contains(&code) {
            // A stub that knows the code fails while reading its arguments.
            reply.write(&i32::from(StatusCode::BadValue))?;
            Ok(())
        } else {
            Err(StatusCode::UnknownTransaction)
        }
    }
}

#[test]
fn probe_marks_only_missing_codes_as_unimplemented() {
    let interface = identify::KEYSTORE_SECURITY_LEVEL_INTERFACE;
    let stub = KnownCodeStub {
        interface: interface.to_string(),
        implemented: vec![FIRST_CALL_TRANSACTION, FIRST_CALL_TRANSACTION + 1],
    };
    assert_eq!(
        probe_local_code(&stub, interface, FIRST_CALL_TRANSACTION),
        ProbeVerdict::Implemented
    );
    assert_eq!(
        probe_local_code(&stub, interface, FIRST_CALL_TRANSACTION + 5),
        ProbeVerdict::Unimplemented
    );
}

#[test]
fn status_reply_with_unknown_transaction_is_unimplemented() {
    struct StatusStub;

    impl Transactable for StatusStub {
        fn transact(
            &self,
            _code: TransactionCode,
            reader: &mut Parcel,
            reply: &mut Parcel,
        ) -> rsbinder::Result<()> {
            reader.set_data_position(0);
            let _: String = reader.read()?;
            reply.write(&i32::from(StatusCode::UnknownTransaction))?;
            Ok(())
        }
    }

    assert_eq!(
        probe_local_code(
            &StatusStub,
            identify::KEYSTORE_SECURITY_LEVEL_INTERFACE,
            FIRST_CALL_TRANSACTION + 9
        ),
        ProbeVerdict::Unimplemented
    );
}

#[test]
fn transport_failures_keep_codes_implemented() {
    struct FailingStub;

    impl Transactable for FailingStub {
        fn transact(
            &self,
            _code: TransactionCode,
            _reader: &mut Parcel,
            _reply: &mut Parcel,
        ) -> rsbinder::Result<()> {
            Err(StatusCode::FailedTransaction)
        }
    }

    assert_eq!(
        probe_local_code(
            &FailingStub,
            identify::KEYSTORE_SECURITY_LEVEL_INTERFACE,
            FIRST_CALL_TRANSACTION
        ),
        ProbeVerdict::Implemented
    );
}

#[test]
fn security_level_codes_cover_the_dispatch_table() {
    let codes = identify::security_level_codes();
    for code in &codes {
        assert!(
            identify::security_level_method_from_code(*code).is_some(),
            "code {code} is probed but has no dispatch entry"
        );
    }
    for offset in 0..16 {
        let code = FIRST_CALL_TRANSACTION + offset;
        if identify::security_level_method_from_code(code).is_some() {
            assert!(
                codes.contains(&code),
                "code {code} is dispatched but never probed"
            );
        }
    }
}

#[test]
fn unreachable_local_stub_keeps_every_code_implemented() {
    // In this test process the service handle is a proxy, so the probe has no
    // authoritative answer and must keep the previous behavior.
    super::super::tests::ensure_binder_process_state();
    assert!(platform_implements_security_level_code(
        FIRST_CALL_TRANSACTION
    ));
    assert!(platform_implements_security_level_code(
        FIRST_CALL_TRANSACTION + 5
    ));
}
