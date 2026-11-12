//! Software-only contract tests; no Binder, network or physical KeyMint calls.
use super::*;
use kmr_wire::{keymint::HardwareAuthToken, secureclock::Timestamp};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct TestSignInfo {
    snapshot: device::SigningInfoSnapshot,
    local_issued: Option<Arc<AtomicUsize>>,
}

impl device::RetrieveCertSigningInfo for TestSignInfo {
    fn signing_info(
        &self,
        _key_type: device::SigningKeyType,
    ) -> Result<device::SigningInfoSnapshot, Error> {
        Ok(self.snapshot.clone())
    }

    fn local_attestation_issued(&self) {
        if let Some(calls) = &self.local_issued {
            calls.fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn local_signing_material(ta: &mut KeyMintTa) -> (device::SigningInfoSnapshot, AttestationKey) {
    let attest = ta
        .generate_key_default(&key_params(KeyPurpose::AttestKey), None)
        .unwrap();
    let (stored, _) = ta.keyblob_parse_decrypt(&attest.key_blob, &[]).unwrap();
    let issuer = AttestationKey {
        key_blob: attest.key_blob,
        attest_key_params: vec![],
        issuer_subject_name: cert::extract_subject(&attest.certificate_chain[0]).unwrap(),
    };
    (
        device::SigningInfoSnapshot {
            signing_key: stored.key_material,
            cert_chain: attest.certificate_chain,
            identity_digest: [0x11; 32],
        },
        issuer,
    )
}

fn local_timing_fixture() -> (KeyMintTa, Arc<AtomicUsize>, AttestationKey) {
    let mut ta = test_ta();
    ta.dev.remote = None;
    let (snapshot, issuer) = local_signing_material(&mut ta);
    let calls = Arc::new(AtomicUsize::new(0));
    ta.set_sign_info(Some(Box::new(TestSignInfo {
        snapshot,
        local_issued: Some(calls.clone()),
    })));
    (ta, calls, issuer)
}

fn local_timing_params() -> Vec<KeyParam> {
    let mut params = key_params(KeyPurpose::Sign);
    params.extend([
        KeyParam::AttestationChallenge(b"local-timing".to_vec()),
        KeyParam::AttestationApplicationId(b"test-app".to_vec()),
    ]);
    params
}

#[test]
fn local_attestation_accounting_excludes_self_signed_keys() {
    let (mut ta, calls, _) = local_timing_fixture();
    ta.generate_key_default(&key_params(KeyPurpose::Sign), None)
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn local_attestation_accounting_runs_once_per_leaf_with_a_warm_cache() {
    let (mut ta, calls, _) = local_timing_fixture();
    for expected in 1..=3 {
        let result = ta
            .generate_key_default(&local_timing_params(), None)
            .unwrap();
        assert_eq!(result.certificate_chain.len(), 2);
        assert_eq!(calls.load(Ordering::SeqCst), expected);
    }
}

#[test]
fn local_attestation_accounting_includes_caller_selected_local_issuers() {
    let (mut ta, calls, issuer) = local_timing_fixture();
    // An enabled remote backend must not hide work performed by a local issuer.
    // This stub would panic if contacted, so the test cannot send a real request.
    ta.dev.remote = Some(Arc::new(TestRemote::default()));
    let result = ta
        .generate_key_default(&local_timing_params(), Some(issuer))
        .unwrap();
    assert_eq!(result.certificate_chain.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn local_attestation_accounting_excludes_remote_success_and_protocol_errors() {
    let mut fixture = DelegationFixture::new();
    let (snapshot, _) = local_signing_material(&mut fixture.ta);
    let calls = Arc::new(AtomicUsize::new(0));
    fixture.ta.set_sign_info(Some(Box::new(TestSignInfo {
        snapshot,
        local_issued: Some(calls.clone()),
    })));
    fixture.generate().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    fixture.response(vec![vec![0x30, 0]], false);
    assert!(fixture.generate().is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn local_attestation_accounting_includes_permitted_remote_fallback() {
    let (mut ta, calls, _) = local_timing_fixture();
    ta.dev.remote = Some(Arc::new(TestRemote {
        unavailable: true,
        ..TestRemote::default()
    }));
    ta.generate_key_default(&local_timing_params(), None)
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn local_attestation_accounting_excludes_invalid_challenges_and_failed_signatures() {
    let (mut ta, calls, _) = local_timing_fixture();
    let mut missing_app_id = key_params(KeyPurpose::Sign);
    missing_app_id.push(KeyParam::AttestationChallenge(b"challenge".to_vec()));
    assert!(ta.generate_key_default(&missing_app_id, None).is_err());
    let mut oversized = local_timing_params();
    oversized.retain(|p| !matches!(p, KeyParam::AttestationChallenge(_)));
    oversized.push(KeyParam::AttestationChallenge(vec![
        1;
        MAX_ATTESTATION_CHALLENGE_LEN
            + 1
    ]));
    assert!(ta.generate_key_default(&oversized, None).is_err());
    let (mut snapshot, _) = local_signing_material(&mut ta);
    snapshot.signing_key = KeyMaterial::Aes(aes::Key::Aes256([0x33; 32]).into());
    ta.dev.sign_info = Some(Box::new(TestSignInfo {
        snapshot,
        local_issued: Some(calls.clone()),
    }));
    assert!(ta
        .generate_key_default(&local_timing_params(), None)
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

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
        Self::with_policy(timeout, kmr_common::auth_signing::Mode::Compatibility)
    }

    fn with_policy(timeout: Option<u32>, mode: kmr_common::auth_signing::Mode) -> Self {
        let mut source = DelegationFixture::new();
        // Software keybox stand-in: a locally generated attestation key signs the
        // compatibility certificates, mirroring the device's keybox.xml path.
        let (snapshot, _) = local_signing_material(&mut source.ta);
        let local_issued = Arc::new(AtomicUsize::new(0));
        source.ta.set_sign_info(Some(Box::new(TestSignInfo {
            snapshot,
            local_issued: Some(local_issued.clone()),
        })));
        let calls = Arc::new(AtomicUsize::new(0));
        // Payment compatibility is certified locally with A's keybox; the relay
        // must not be contacted and the certificate must describe the real policy.
        source.ta.dev.remote = Some(Arc::new(TestRemote {
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
        let authorize_on_a =
            kmr_common::auth_signing::authorize_on_a(mode, Some("ommega-auth-repro"), &params);
        let key = source
            .ta
            .generate_key(&params, None, authorize_on_a)
            .unwrap();
        assert_eq!(local_issued.load(Ordering::SeqCst), 1);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "compatibility keys must not reach the relay"
        );
        let (stored, _) = source.ta.keyblob_parse_decrypt(&key.key_blob, &[]).unwrap();
        assert!(
            matches!(stored.key_material, KeyMaterial::Ec(..)),
            "compatibility keys are held by A's local keybox"
        );
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
fn valid_per_operation_auth_allows_exactly_one_local_finish() {
    let mut f = Fixture::new(None);
    let begin = f.begin(None).unwrap();
    let token = f.token(begin.challenge);
    let handle = begin.op_handle;
    assert_eq!(f.finish(handle, Some(token.clone())).error_code, 0);
    assert_ne!(f.finish(handle, Some(token)).error_code, 0);
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn auto_payment_key_signs_only_after_valid_per_operation_authentication() {
    for authenticated in [false, true] {
        let mut fixture = Fixture::with_policy(None, kmr_common::auth_signing::Mode::Auto);
        let begin = fixture.begin(None).unwrap();
        let token = authenticated.then(|| fixture.token(begin.challenge));
        let response = fixture.finish(begin.op_handle, token);
        assert_eq!(
            response.error_code,
            if authenticated {
                0
            } else {
                ErrorCode::KeyUserNotAuthenticated as i32
            },
        );
        assert_eq!(
            fixture.calls.load(Ordering::SeqCst),
            0,
            "the local compatibility path never contacts B"
        );
    }
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
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn timeout_auth_is_checked_before_begin_and_local_io() {
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
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn device_credential_fallback_can_authorize_the_same_local_signing_path() {
    let mut f = Fixture::new(None);
    f.params.retain(|p| !matches!(p, KeyParam::UserAuthType(_)));
    f.params.push(KeyParam::UserAuthType(3));
    f.key = f.ta.generate_key(&f.params, None, true).unwrap();
    let begin = f.begin(None).unwrap();
    let mut token = f.token(begin.challenge);
    token.authenticator_type = HardwareAuthenticatorType::Password;
    f.resign(&mut token);
    assert_eq!(f.finish(begin.op_handle, Some(token)).error_code, 0);
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
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
        assert!(!kmr_common::auth_signing::eligible(&params));
    }
    for auth_type in [1, 2, 3] {
        let mut params = f.params.clone();
        params.retain(|p| !matches!(p, KeyParam::UserAuthType(_)));
        params.push(KeyParam::UserAuthType(auth_type));
        assert!(kmr_common::auth_signing::eligible(&params));
    }
}

#[test]
fn preserved_request_never_falls_back_to_a_local_key() {
    let mut f = Fixture::new(None);
    f.ta.dev.remote = Some(Arc::new(TestRemote {
        unavailable: true,
        ..TestRemote::default()
    }));
    let err = f.ta.generate_key(&f.params, None, false).unwrap_err();
    assert!(matches!(
        err.kind(),
        ErrorKind::Hal(ErrorCode::UnknownError, _)
    ));
}

#[test]
fn preserve_and_compatibility_decisions_remain_per_key() {
    let mut fixture = Fixture::new(None);
    let compatible_blob = fixture.key.key_blob.clone();
    let preserved_source = signed_auth_fixture(None);
    let preserved_chain = preserved_source.chain;
    fixture.ta.dev.remote = Some(Arc::new(TestRemote {
        cert_chain: Some(preserved_chain.clone()),
        expected_user_auth: Some(vec![(502, 42), (504, 2)]),
        ..TestRemote::default()
    }));
    let preserved = fixture
        .ta
        .generate_key(&fixture.params, None, false)
        .unwrap();
    let (compatible, _) = fixture
        .ta
        .keyblob_parse_decrypt(&compatible_blob, &[])
        .unwrap();
    let (preserved_blob, _) = fixture
        .ta
        .keyblob_parse_decrypt(&preserved.key_blob, &[])
        .unwrap();
    assert!(
        matches!(compatible.key_material, KeyMaterial::Ec(..)),
        "compatibility is certified locally with A's keybox"
    );
    assert!(
        matches!(preserved_blob.key_material, KeyMaterial::Remote(_)),
        "preserve keeps the physical B key"
    );
    assert_eq!(
        preserved.certificate_chain[0].encoded_certificate, preserved_chain[0],
        "never rewrite the B certificate"
    );
    fixture.ta.dev.remote = Some(Arc::new(TestRemote {
        unavailable: true,
        expected_user_auth: Some(vec![(502, 42), (504, 2)]),
        ..TestRemote::default()
    }));
    assert!(
        fixture
            .ta
            .generate_key(&fixture.params, None, false)
            .is_err(),
        "preserve must not fall back to a local keybox"
    );
}

fn signed_auth_fixture(timeout: Option<u32>) -> DelegationFixture {
    DelegationFixture::with_child_policy(false, |params| {
        params.retain(|p| !matches!(p, KeyParam::NoAuthRequired));
        params.extend([
            KeyParam::UserSecureId(42),
            KeyParam::UserAuthType(HardwareAuthenticatorType::Fingerprint as u32),
        ]);
        if let Some(timeout) = timeout {
            params.push(KeyParam::AuthTimeout(timeout));
        }
    })
}

#[test]
fn auto_known_probe_keeps_the_signed_b_authentication_policy() {
    let mut source = DelegationFixture::with_child_policy(false, |params| {
        params.retain(|p| {
            !matches!(
                p,
                KeyParam::NoAuthRequired | KeyParam::AttestationChallenge(_)
            )
        });
        params.extend([
            KeyParam::UserSecureId(42),
            KeyParam::UserAuthType(3),
            KeyParam::Purpose(KeyPurpose::Verify),
            KeyParam::AttestationChallenge(vec![7; 32]),
        ]);
    });
    let chain = source.chain.clone();
    source.ta.dev.remote = Some(Arc::new(TestRemote {
        cert_chain: Some(chain.clone()),
        expected_user_auth: Some(vec![(502, 42), (504, 3)]),
        expected_purposes: Some(vec![KeyPurpose::Sign as i32, KeyPurpose::Verify as i32]),
        ..TestRemote::default()
    }));
    let decision = kmr_common::auth_signing::authorize_on_a(
        kmr_common::auth_signing::Mode::Auto,
        Some("TrustAttestor_silent_auth_14c21f61-4c15-4e09-92d7-3a33d665458c"),
        &source.params,
    );
    assert!(
        !decision,
        "the complete probe must preserve B authentication"
    );
    let key = source
        .ta
        .generate_key(&source.params, None, decision)
        .unwrap();
    assert_eq!(key.certificate_chain[0].encoded_certificate, chain[0]);
}

#[test]
fn compatibility_certificate_keeps_the_requested_authentication_policy() {
    let fixture = Fixture::new(None);
    assert_eq!(
        fixture.calls.load(Ordering::SeqCst),
        0,
        "compatibility must not contact B"
    );
    let blob = fixture.key.key_blob.clone();
    let (stored, _) = fixture.ta.keyblob_parse_decrypt(&blob, &[]).unwrap();
    let chars = kmr_common::tag::characteristics_at(
        &stored.characteristics,
        SecurityLevel::TrustedEnvironment,
    )
    .unwrap();
    assert!(chars.contains(&KeyParam::UserSecureId(42)));
    assert!(chars.contains(&KeyParam::UserAuthType(
        HardwareAuthenticatorType::Fingerprint as u32
    )));
    assert!(
        !chars.contains(&KeyParam::NoAuthRequired),
        "an auth-bound key must never advertise NO_AUTH_REQUIRED"
    );
}

#[test]
fn preserved_auth_timeout_matches_the_signed_b_policy_exactly() {
    for timeout in [None, Some(0), Some(30)] {
        let mut source = signed_auth_fixture(timeout);
        let expected_chain = source.chain.clone();
        source.ta.dev.remote = Some(Arc::new(TestRemote {
            cert_chain: Some(expected_chain.clone()),
            ..TestRemote::default()
        }));
        let generated = source.ta.generate_key(&source.params, None, false).unwrap();
        assert_eq!(
            generated.certificate_chain[0].encoded_certificate,
            expected_chain[0]
        );
    }
}

#[test]
fn validly_signed_wrong_timeout_window_is_rejected_before_key_storage() {
    let mut source = signed_auth_fixture(Some(10));
    let chain = source.chain.clone();
    source
        .params
        .retain(|p| !matches!(p, KeyParam::AuthTimeout(_)));
    source.params.push(KeyParam::AuthTimeout(30));
    source.ta.dev.remote = Some(Arc::new(TestRemote {
        cert_chain: Some(chain),
        ..TestRemote::default()
    }));
    let error = source
        .ta
        .generate_key(&source.params, None, false)
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        ErrorKind::Hal(ErrorCode::VerificationFailed, _)
    ));
}
