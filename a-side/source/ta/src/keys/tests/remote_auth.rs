//! Software-only contract tests; no Binder, network or physical KeyMint calls.
use super::*;
use kmr_wire::{keymint::HardwareAuthToken, secureclock::Timestamp};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct TestHmac;
impl device::DeviceHmac for TestHmac {
    fn hmac(&self, imp: &dyn crypto::Hmac, data: &[u8]) -> Result<Vec<u8>, Error> {
        let mut op = imp.begin(
            crypto::hmac::Key::new(vec![0x44; 32]).into(),
            Digest::Sha256,
        )?;
        op.update(data)?;
        op.finish()
    }
}

struct Fixture {
    ta: KeyMintTa,
    params: Vec<KeyParam>,
    key: KeyCreationResult,
    calls: Arc<AtomicUsize>,
}

impl Fixture {
    fn new(timeout: Option<u32>) -> Self {
        let mut source = DelegationFixture::new();
        let calls = Arc::new(AtomicUsize::new(0));
        source.ta.dev.remote = Some(Arc::new(TestRemote {
            cert_chain: Some(source.chain.clone()),
            auth_signing_on_b: true,
            expected_user_auth: Some(vec![(503, 1)]),
            sign_calls: Some(calls.clone()),
            ..TestRemote::default()
        }));
        source.ta.set_device_hmac(Box::new(TestHmac));
        let mut params = source.params;
        params.retain(|p| !matches!(p, KeyParam::NoAuthRequired));
        params.extend([
            KeyParam::UserSecureId(42),
            KeyParam::UserAuthType(HardwareAuthenticatorType::Fingerprint as u32),
        ]);
        if let Some(timeout) = timeout {
            params.push(KeyParam::AuthTimeout(timeout));
        }
        let key = source.ta.generate_key(&params, None).unwrap();
        // The B certificate must remain byte-for-byte the no-auth fixture.
        assert_eq!(
            key.certificate_chain[0].encoded_certificate,
            source.chain[0]
        );
        let (stored, _) = source.ta.keyblob_parse_decrypt(&key.key_blob, &[]).unwrap();
        assert!(matches!(stored.key_material, KeyMaterial::Remote(_)));
        let chars = kmr_common::tag::characteristics_at(
            &stored.characteristics,
            SecurityLevel::TrustedEnvironment,
        )
        .unwrap();
        assert!(chars.contains(&KeyParam::UserSecureId(42)));
        assert!(!chars.contains(&KeyParam::NoAuthRequired));
        Self {
            ta: source.ta,
            params,
            key,
            calls,
        }
    }

    fn token(&self, challenge: i64) -> HardwareAuthToken {
        let mut token = HardwareAuthToken {
            challenge,
            user_id: 42,
            authenticator_id: 42,
            authenticator_type: HardwareAuthenticatorType::Fingerprint,
            timestamp: Timestamp { milliseconds: 1000 },
            mac: vec![],
        };
        self.resign(&mut token);
        token
    }

    fn resign(&self, token: &mut HardwareAuthToken) {
        token.mac = self
            .ta
            .device_hmac(&crate::hardware_auth_token_mac_input(token).unwrap())
            .unwrap();
    }

    fn begin(
        &mut self,
        token: Option<HardwareAuthToken>,
    ) -> Result<kmr_wire::InternalBeginResult, Error> {
        self.ta.begin_operation(
            KeyPurpose::Sign,
            &self.key.key_blob,
            vec![
                KeyParam::Algorithm(Algorithm::Ec),
                KeyParam::Digest(Digest::Sha256),
            ],
            token,
        )
    }

    fn finish(
        &mut self,
        handle: i64,
        token: Option<HardwareAuthToken>,
    ) -> kmr_wire::PerformOpResponse {
        self.ta
            .process_req(PerformOpReq::OperationFinish(kmr_wire::FinishRequest {
                op_handle: handle,
                input: Some(vec![1, 2, 3]),
                signature: None,
                auth_token: token,
                timestamp_token: None,
                confirmation_token: None,
            }))
    }
}

#[test]
fn valid_per_operation_auth_allows_exactly_one_remote_finish() {
    let mut f = Fixture::new(None);
    let begin = f.begin(None).unwrap();
    let token = f.token(begin.challenge);
    let handle = begin.op_handle;
    assert_eq!(f.finish(handle, Some(token.clone())).error_code, 0);
    assert_ne!(f.finish(handle, Some(token)).error_code, 0);
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn missing_or_invalid_auth_never_reaches_b() {
    let mut f = Fixture::new(None);
    for case in 0..5 {
        let begin = f.begin(None).unwrap();
        let mut token = f.token(begin.challenge);
        match case {
            0 => {}
            1 => token.mac[0] ^= 1,
            2 => {
                token.challenge ^= 1;
                f.resign(&mut token);
            }
            3 => {
                token.user_id = 99;
                token.authenticator_id = 99;
                f.resign(&mut token);
            }
            4 => {
                token.authenticator_type = HardwareAuthenticatorType::Password;
                f.resign(&mut token);
            }
            _ => unreachable!(),
        }
        let token = if case == 0 { None } else { Some(token) };
        assert_eq!(
            f.finish(begin.op_handle, token).error_code,
            ErrorCode::KeyUserNotAuthenticated as i32
        );
    }
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn token_from_a_previous_operation_does_not_authorize_a_new_one() {
    let mut f = Fixture::new(None);
    let first = f.begin(None).unwrap();
    let token = f.token(first.challenge);
    assert_eq!(f.finish(first.op_handle, Some(token.clone())).error_code, 0);
    let next = f.begin(None).unwrap();
    assert_eq!(
        f.finish(next.op_handle, Some(token)).error_code,
        ErrorCode::KeyUserNotAuthenticated as i32
    );
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn timeout_auth_is_checked_before_begin_and_remote_io() {
    let mut f = Fixture::new(Some(30));
    assert!(f.begin(None).is_err());
    let mut expired = f.token(0);
    expired.timestamp.milliseconds = -31_000;
    f.resign(&mut expired);
    assert!(f.begin(Some(expired)).is_err());
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    let token = f.token(0);
    let begin = f.begin(Some(token)).unwrap();
    assert_eq!(f.finish(begin.op_handle, None).error_code, 0);
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn device_credential_fallback_can_authorize_the_same_remote_signing_path() {
    let mut f = Fixture::new(None);
    f.params.retain(|p| !matches!(p, KeyParam::UserAuthType(_)));
    f.params.push(KeyParam::UserAuthType(3));
    f.key = f.ta.generate_key(&f.params, None).unwrap();
    let begin = f.begin(None).unwrap();
    let mut token = f.token(begin.challenge);
    token.authenticator_type = HardwareAuthenticatorType::Password;
    f.resign(&mut token);
    assert_eq!(f.finish(begin.op_handle, Some(token)).error_code, 0);
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn experiment_is_not_used_for_other_purposes_or_presence_policies() {
    let f = Fixture::new(None);
    for extra in [
        KeyParam::Purpose(KeyPurpose::Decrypt),
        KeyParam::Purpose(KeyPurpose::AgreeKey),
        KeyParam::Purpose(KeyPurpose::AttestKey),
        KeyParam::TrustedConfirmationRequired,
        KeyParam::TrustedUserPresenceRequired,
        KeyParam::AllowWhileOnBody,
        KeyParam::NoAuthRequired,
    ] {
        let mut params = f.params.clone();
        params.push(extra);
        assert!(!supports_a_authorized_remote_signing(&params));
    }
    for auth_type in [1, 2, 3] {
        let mut params = f.params.clone();
        params.retain(|p| !matches!(p, KeyParam::UserAuthType(_)));
        params.push(KeyParam::UserAuthType(auth_type));
        assert!(supports_a_authorized_remote_signing(&params));
    }
}

#[test]
fn remote_unavailability_never_changes_the_experiment_to_a_local_key() {
    let mut f = Fixture::new(None);
    f.ta.dev.remote = Some(Arc::new(TestRemote {
        auth_signing_on_b: true,
        unavailable: true,
        expected_user_auth: Some(vec![(503, 1)]),
        ..TestRemote::default()
    }));
    let err = f.ta.generate_key(&f.params, None).unwrap_err();
    assert!(matches!(
        err.kind(),
        ErrorKind::Hal(ErrorCode::UnknownError, _)
    ));
}
