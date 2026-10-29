use super::*;
use crate::{HardwareInfo, KeyMintHalVersion, KeyMintTa, RpcInfo, RpcInfoV3};
use kmr_wire::keymint::{Algorithm, DateTime, PaddingMode};

// In-process software fixture only: no Binder, HTTP, device files or real TEE.
struct TestKeys;
impl device::RetrieveKeyMaterial for TestKeys {
    fn root_kek(&self, _: &[u8]) -> Result<OpaqueOr<crypto::hmac::Key>, Error> {
        Ok(crypto::hmac::Key::new(vec![0x11; 32]).into())
    }
    fn kak(&self) -> Result<OpaqueOr<aes::Key>, Error> {
        Ok(aes::Key::Aes256([0x22; 32]).into())
    }
}

struct TestClock;
impl crypto::MonotonicClock for TestClock {
    fn now(&self) -> crypto::MillisecondsSinceEpoch {
        crypto::MillisecondsSinceEpoch(1000)
    }
}

#[derive(Default)]
struct TestRemote {
    version_tags: Option<device::KeyMintVersionTags>,
    cert_chain: Option<Vec<Vec<u8>>>,
    expected_purpose: Option<KeyPurpose>,
    expected_issuer: Option<String>,
    unavailable: bool,
    expected_user_auth: Option<Vec<(u32, i64)>>,
    sign_calls: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    verified_calls: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
}
impl device::RemoteBackend for TestRemote {
    fn version_tags(
        &self,
        level: SecurityLevel,
    ) -> Result<Option<device::KeyMintVersionTags>, Error> {
        assert_eq!(level, SecurityLevel::TrustedEnvironment);
        Ok(self.version_tags)
    }

    fn attestation_verified(&self, _: &[u8], _: Option<u64>) {
        if let Some(calls) = &self.verified_calls {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    fn attest(
        &self,
        _: &[u8],
        _: &[u8],
        _: &str,
        _: Option<&[u8]>,
        params: &device::RemoteAttestParams,
    ) -> Result<Option<device::RemoteAttestation>, Error> {
        if let Some(expected) = &self.expected_user_auth {
            assert_eq!(&params.user_auth, expected);
        }
        assert_eq!(
            params.purpose,
            [self.expected_purpose.unwrap_or(KeyPurpose::Sign) as i32]
        );
        assert_eq!(
            params
                .attestation_key
                .as_ref()
                .map(|key| key.alias.as_str()),
            self.expected_issuer.as_deref()
        );
        if self.unavailable {
            return Ok(None);
        }
        let cert_chain = self
            .cert_chain
            .as_ref()
            .expect("unexpected remote generation");
        Ok(Some(device::RemoteAttestation {
            cert_chain: cert_chain.clone(),
            relay_latency_ms: None,
            effective_security_level: Some(SecurityLevel::TrustedEnvironment),
            profile: device::RemoteKeyMintProfile {
                interface_version: 5,
                interface_hash: "test".into(),
                profile_version: 500,
                hardware_version: 500,
                security_level: SecurityLevel::TrustedEnvironment,
                has_strongbox: false,
                max_operations: None,
            },
        }))
    }
    fn sign(&self, alias: &str, data: &[u8], algorithm: &str) -> Result<Option<Vec<u8>>, Error> {
        if let Some(calls) = &self.sign_calls {
            assert!(alias.starts_with("ommega-remote-"));
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        } else {
            assert_eq!(alias, "test-only", "unexpected remote certificate signing");
        }
        assert_eq!(algorithm, "SHA256withECDSA");
        assert_eq!(data, [1, 2, 3]);
        Ok(Some(vec![1; 64]))
    }
    fn decrypt(&self, _: &str, _: &[u8], _: &str) -> Result<Option<Vec<u8>>, Error> {
        panic!("unexpected decrypt")
    }
    fn agree_key(&self, _: &str, _: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        panic!("unexpected agreement")
    }
    fn enabled(&self) -> bool {
        true
    }
}

mod remote_auth;

fn install_remote_sign(ta: &mut KeyMintTa) -> kmr_wire::FinishRequest {
    ta.operations[0] = Some(crate::operation::Operation {
        handle: crate::operation::OpHandle(17),
        aad_allowed: false,
        slot_to_delete: None,
        trusted_conf_data: None,
        auth_info: None,
        crypto_op: crate::operation::CryptoOperation::RemoteSign {
            alias: "test-only".into(),
            algorithm: "SHA256withECDSA".into(),
            data: vec![1, 2],
        },
        input_size: 2,
    });
    kmr_wire::FinishRequest {
        op_handle: 17,
        input: Some(vec![3]),
        signature: None,
        auth_token: None,
        timestamp_token: None,
        confirmation_token: None,
    }
}

#[test]
fn detached_sign_consumes_the_handle_and_does_not_hold_the_ta() {
    let ta = std::sync::Arc::new(std::sync::Mutex::new(test_ta()));
    let task = {
        let mut guard = ta.lock().unwrap();
        let req = install_remote_sign(&mut guard);
        let task = guard.take_unrestricted_remote_sign(&req).unwrap().unwrap();
        assert!(guard.operations[0].is_none());
        assert!(guard.take_unrestricted_remote_sign(&req).is_err());
        task
    };
    let guard = ta
        .try_lock()
        .expect("detached work must not retain TA state");
    // The remote backend is an in-process stub. Completing while the TA is
    // locked proves it does not reacquire that lock to send its request.
    let reply = task.execute();
    assert_eq!(reply.error_code, 0);
    assert!(guard.operations[0].is_none());
}

#[test]
fn trusted_confirmation_sign_retains_the_existing_finish_path() {
    let mut ta = test_ta();
    let req = install_remote_sign(&mut ta);
    ta.operations[0].as_mut().unwrap().trusted_conf_data = Some(vec![]);
    assert!(ta.take_unrestricted_remote_sign(&req).unwrap().is_none());
    assert!(
        ta.operations[0].is_some(),
        "serialized finish still owns its handle"
    );
}

#[test]
fn single_use_remote_sign_retains_deletion_ordering() {
    let mut ta = test_ta();
    let req = install_remote_sign(&mut ta);
    ta.operations[0].as_mut().unwrap().slot_to_delete = Some(keyblob::SecureDeletionSlot(0));
    assert!(ta.take_unrestricted_remote_sign(&req).unwrap().is_none());
    assert!(ta.operations[0].is_some());
}

#[test]
fn oversized_detached_sign_consumes_the_handle_and_rejects_before_io() {
    let mut ta = test_ta();
    let mut req = install_remote_sign(&mut ta);
    req.input = Some(vec![0; 1 << 20]);
    assert!(ta.take_unrestricted_remote_sign(&req).is_err());
    assert!(ta.operations[0].is_none());
}

fn test_ta() -> KeyMintTa {
    let mut ta = KeyMintTa::new_allowing_versions(
        HardwareInfo {
            security_level: SecurityLevel::TrustedEnvironment,
            version_number: 500,
            impl_name: "test",
            author_name: "test",
            unique_id: "test",
        },
        RpcInfo::V3(RpcInfoV3 {
            author_name: "test",
            unique_id: "test",
            fused: false,
            supported_num_of_keys_in_csr: 20,
        }),
        kmr_crypto_ommega::implementation(
            Box::new(kmr_crypto_ommega::rng::OmmegaRng),
            Box::new(TestClock),
        ),
        device::Implementation {
            keys: Box::new(TestKeys),
            sign_info: None,
            remote: Some(std::sync::Arc::new(TestRemote::default())),
            attest_ids: None,
            sdd_mgr: None,
            bootloader: Box::new(device::BootloaderDone),
            sk_wrapper: None,
            tup: Box::new(device::TrustedPresenceUnsupported),
            legacy_key: None,
            rpc: Box::new(device::NoOpRetrieveRpcArtifacts),
        },
        vec![KeyMintHalVersion::V5],
    );
    assert_eq!(
        ta.process_req(PerformOpReq::SetBootInfo(SetBootInfoRequest {
            verified_boot_state: 0,
            verified_boot_hash: vec![3; 32],
            verified_boot_key: vec![4; 32],
            device_boot_locked: true,
            boot_patchlevel: 20260801,
        }))
        .error_code,
        0
    );
    assert_eq!(
        ta.process_req(PerformOpReq::SetHalInfo(SetHalInfoRequest {
            os_version: 160000,
            os_patchlevel: 202608,
            vendor_patchlevel: 20260801,
        }))
        .error_code,
        0
    );
    ta
}

fn key_params(purpose: KeyPurpose) -> Vec<KeyParam> {
    vec![
        KeyParam::Algorithm(Algorithm::Ec),
        KeyParam::EcCurve(EcCurve::P256),
        KeyParam::Purpose(purpose),
        KeyParam::Digest(Digest::Sha256),
        KeyParam::NoAuthRequired,
        KeyParam::CertificateNotBefore(DateTime { ms_since_epoch: 0 }),
        KeyParam::CertificateNotAfter(DateTime {
            ms_since_epoch: 2_000_000_000_000,
        }),
    ]
}

#[test]
fn remote_properties_preserve_requested_values_and_other_key_params() {
    let mut params = key_params(KeyPurpose::Sign);
    params.extend([
        KeyParam::AttestationIdBrand(b"OnePlus".to_vec()),
        KeyParam::AttestationIdDevice(b"OP591BL1".to_vec()),
        KeyParam::AttestationIdProduct(b"PHB110".to_vec()),
        KeyParam::AttestationIdManufacturer(b"OnePlus".to_vec()),
        KeyParam::AttestationIdModel(b"PHB110".to_vec()),
    ]);
    let forwarded = extract_remote_attest_params(&params);
    assert_eq!(
        forwarded.device_properties,
        vec![
            (710, b"OnePlus".to_vec()),
            (711, b"OP591BL1".to_vec()),
            (712, b"PHB110".to_vec()),
            (716, b"OnePlus".to_vec()),
            (717, b"PHB110".to_vec()),
        ]
    );
    assert_eq!(forwarded.purpose, vec![KeyPurpose::Sign as i32]);
    assert_eq!(forwarded.user_auth, vec![(503, 1)]);
}

#[test]
fn remote_properties_keep_absence_subset_and_empty_values() {
    assert!(extract_remote_attest_params(&key_params(KeyPurpose::Sign))
        .device_properties
        .is_empty());
    let forwarded = extract_remote_attest_params(&[
        KeyParam::AttestationIdModel(vec![0, 255, 65]),
        KeyParam::AttestationIdBrand(Vec::new()),
    ]);
    assert_eq!(
        forwarded.device_properties,
        vec![(717, vec![0, 255, 65]), (710, Vec::new())]
    );
}

fn remote_rot() -> crypto::RemoteRootOfTrust {
    crypto::RemoteRootOfTrust {
        verified_boot_key: vec![5; 32],
        device_locked: true,
        verified_boot_state: 0,
        verified_boot_hash: vec![6; 32],
        attestation_version: 500,
        keymaster_version: 500,
        os_version: Some(150000),
        os_patchlevel: Some(202512),
        vendor_patchlevel: Some(20251205),
        boot_patchlevel: Some(20251201),
    }
}

#[test]
fn unchallenged_ec_and_rsa_use_profile_before_first_remote_generation() {
    let versions = device::KeyMintVersionTags {
        os_version: 170000,
        os_patchlevel: 202609,
        vendor_patchlevel: Some(20260901),
        boot_patchlevel: Some(20251201),
    };
    let mut ta = test_ta();
    ta.dev.remote = Some(std::sync::Arc::new(TestRemote {
        version_tags: Some(versions),
        // Any attempt to acquire remote key material in this test panics.
        ..TestRemote::default()
    }));
    for rsa in [false, true] {
        let mut params = key_params(KeyPurpose::Sign);
        if rsa {
            params.retain(|p| !matches!(p, KeyParam::Algorithm(_) | KeyParam::EcCurve(_)));
            params.extend([
                KeyParam::Algorithm(Algorithm::Rsa),
                KeyParam::KeySize(kmr_wire::KeySizeInBits(2048)),
                KeyParam::RsaPublicExponent(kmr_wire::RsaExponent(65537)),
                KeyParam::Padding(PaddingMode::RsaPss),
            ]);
        }
        let key = ta.generate_key_default(&params, None).unwrap();
        let (stored, _) = ta.keyblob_parse_decrypt(&key.key_blob, &[]).unwrap();
        assert!(!matches!(stored.key_material, KeyMaterial::Remote(_)));
        let auths = stored
            .characteristics_at(SecurityLevel::TrustedEnvironment)
            .unwrap();
        for tag in [
            KeyParam::OsVersion(170000),
            KeyParam::OsPatchlevel(202609),
            KeyParam::VendorPatchlevel(20260901),
            KeyParam::BootPatchlevel(20251201),
        ] {
            assert!(auths.contains(&tag), "missing {tag:?}, rsa={rsa}");
        }
        assert_eq!(
            ta.get_key_characteristics(&key.key_blob, vec![], vec![])
                .unwrap(),
            stored.characteristics
        );
        // An explicit upgrade must not switch the metadata back to A's dates.
        assert!(ta.upgrade_key(&key.key_blob, vec![]).unwrap().is_empty());
    }
    // The profile changes only new key authorizations, never boot/RoT state.
    assert_eq!(ta.boot_info.as_ref().unwrap().boot_patchlevel, 20260801);
    assert_eq!(ta.hal_info.as_ref().unwrap().os_version, 160000);
}

#[test]
fn absent_remote_partition_tags_are_not_filled_from_local_boot_info() {
    let mut ta = test_ta();
    ta.dev.remote = Some(std::sync::Arc::new(TestRemote {
        version_tags: Some(device::KeyMintVersionTags {
            os_version: 170000,
            os_patchlevel: 202609,
            vendor_patchlevel: None,
            boot_patchlevel: None,
        }),
        ..TestRemote::default()
    }));
    let key = ta
        .generate_key_default(&key_params(KeyPurpose::Sign), None)
        .unwrap();
    let (stored, _) = ta.keyblob_parse_decrypt(&key.key_blob, &[]).unwrap();
    let auths = stored
        .characteristics_at(SecurityLevel::TrustedEnvironment)
        .unwrap();
    assert!(!auths.iter().any(|tag| matches!(
        tag,
        KeyParam::BootPatchlevel(_) | KeyParam::VendorPatchlevel(_)
    )));
}

#[test]
fn legacy_remote_and_disabled_remote_keep_configured_version_tags() {
    for disabled in [false, true] {
        let mut ta = test_ta();
        if disabled {
            ta.dev.remote = None;
        }
        let key = ta
            .generate_key_default(&key_params(KeyPurpose::Sign), None)
            .unwrap();
        let (stored, _) = ta.keyblob_parse_decrypt(&key.key_blob, &[]).unwrap();
        assert!(stored
            .characteristics_at(SecurityLevel::TrustedEnvironment)
            .unwrap()
            .contains(&KeyParam::BootPatchlevel(20260801)));
    }
}

#[test]
fn import_and_existing_blob_upgrade_share_profile_without_changing_key_material() {
    let mut ta = test_ta();
    let params = vec![
        KeyParam::Algorithm(Algorithm::Aes),
        KeyParam::KeySize(kmr_wire::KeySizeInBits(256)),
        KeyParam::Purpose(KeyPurpose::Encrypt),
        KeyParam::NoAuthRequired,
    ];
    let old = ta
        .import_key(
            &params,
            KeyFormat::Raw,
            &[7; 32],
            None,
            KeyImport::NonWrapped,
        )
        .unwrap();
    let (before, _) = ta.keyblob_parse_decrypt(&old.key_blob, &[]).unwrap();
    ta.dev.remote = Some(std::sync::Arc::new(TestRemote {
        version_tags: Some(device::KeyMintVersionTags {
            os_version: 170000,
            os_patchlevel: 202609,
            vendor_patchlevel: None,
            boot_patchlevel: None,
        }),
        ..TestRemote::default()
    }));
    let imported = ta
        .import_key(
            &params,
            KeyFormat::Raw,
            &[7; 32],
            None,
            KeyImport::NonWrapped,
        )
        .unwrap();
    let upgraded = ta.upgrade_key(&old.key_blob, vec![]).unwrap();
    assert!(!upgraded.is_empty());
    let (after, _) = ta.keyblob_parse_decrypt(&upgraded, &[]).unwrap();
    let (new, _) = ta.keyblob_parse_decrypt(&imported.key_blob, &[]).unwrap();
    assert_eq!(before.key_material, after.key_material);
    assert_eq!(new.characteristics, after.characteristics);
    assert!(ta.upgrade_key(&upgraded, vec![]).unwrap().is_empty());
    assert_eq!(ta.boot_info.as_ref().unwrap().boot_patchlevel, 20260801);
}

#[test]
fn unchallenged_auth_bound_generation_stays_local() {
    let mut ta = test_ta();
    let mut params = key_params(KeyPurpose::Sign);
    params.retain(|p| !matches!(p, KeyParam::NoAuthRequired));
    params.extend([
        KeyParam::UserSecureId(42),
        KeyParam::UserAuthType(HardwareAuthenticatorType::Password as u32),
        KeyParam::AuthTimeout(5),
        KeyParam::UnlockedDeviceRequired,
    ]);
    params.push(KeyParam::AttestationApplicationId(b"test-app".to_vec()));

    // A request without a challenge must not acquire a remote key.
    let key = ta.generate_key_default(&params, None).unwrap();
    let (stored, _) = ta.keyblob_parse_decrypt(&key.key_blob, &[]).unwrap();
    assert!(!matches!(stored.key_material, KeyMaterial::Remote(_)));
}

#[test]
fn remote_key_cannot_issue_certificates_via_the_sign_transport() {
    let ta = test_ta();
    let error = ta
        .sign_cert_data(
            KeyMaterial::Remote(crypto::RemoteRef {
                alias: "legacy-attest-key".into(),
                public_key: vec![],
                root_of_trust: Some(remote_rot()),
            }),
            b"child TBS",
        )
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        ErrorKind::Hal(ErrorCode::IncompatiblePurpose, _)
    ));
}

struct DelegationFixture {
    ta: KeyMintTa,
    issuer: AttestationKey,
    issuer_alias: String,
    params: Vec<KeyParam>,
    chain: Vec<Vec<u8>>,
    batch_chain: Vec<Vec<u8>>,
}

impl DelegationFixture {
    fn new() -> Self {
        Self::with_rsa_issuer(false)
    }

    fn with_rsa_issuer(rsa: bool) -> Self {
        let mut ta = test_ta();
        let root_params = key_params(KeyPurpose::AttestKey);
        let root = ta.generate_key_default(&root_params, None).unwrap();
        let root_info = AttestationKey {
            key_blob: root.key_blob.clone(),
            attest_key_params: vec![],
            issuer_subject_name: cert::extract_subject(&root.certificate_chain[0]).unwrap(),
        };
        let mut parent_params = key_params(KeyPurpose::AttestKey);
        if rsa {
            parent_params.retain(|p| !matches!(p, KeyParam::Algorithm(_) | KeyParam::EcCurve(_)));
            parent_params.extend([
                KeyParam::Algorithm(Algorithm::Rsa),
                KeyParam::KeySize(kmr_wire::KeySizeInBits(2048)),
                KeyParam::RsaPublicExponent(kmr_wire::RsaExponent(65537)),
            ]);
        }
        parent_params.extend([
            KeyParam::AttestationChallenge(b"parent-challenge".to_vec()),
            KeyParam::AttestationApplicationId(b"test-app".to_vec()),
        ]);
        let parent = ta
            .generate_key_default(&parent_params, Some(root_info.clone()))
            .unwrap();
        let subject = cert::extract_subject(&parent.certificate_chain[0]).unwrap();
        let parent_chain: Vec<_> = parent
            .certificate_chain
            .iter()
            .chain(&root.certificate_chain)
            .map(|cert| cert.encoded_certificate.clone())
            .collect();
        ta.dev.remote = Some(std::sync::Arc::new(TestRemote {
            cert_chain: Some(parent_chain.clone()),
            expected_purpose: Some(KeyPurpose::AttestKey),
            ..TestRemote::default()
        }));
        let remote_parent = ta.generate_key_default(&parent_params, None).unwrap();
        let (stored, _) = ta
            .keyblob_parse_decrypt(&remote_parent.key_blob, &[])
            .unwrap();
        let KeyMaterial::Remote(reference) = stored.key_material else {
            panic!("remote parent")
        };
        let issuer_alias = reference.alias;
        let mut params = key_params(KeyPurpose::Sign);
        params.extend([
            KeyParam::AttestationChallenge(b"child-challenge".to_vec()),
            KeyParam::AttestationApplicationId(b"test-app".to_vec()),
        ]);
        let physical_child = ta
            .generate_key_default(
                &params,
                Some(AttestationKey {
                    key_blob: parent.key_blob,
                    attest_key_params: vec![],
                    issuer_subject_name: subject.clone(),
                }),
            )
            .unwrap();
        let mut chain: Vec<_> = physical_child
            .certificate_chain
            .into_iter()
            .map(|cert| cert.encoded_certificate)
            .collect();
        chain.extend(parent_chain);
        let batch_child = ta.generate_key_default(&params, Some(root_info)).unwrap();
        let batch_chain: Vec<_> = batch_child
            .certificate_chain
            .into_iter()
            .chain(root.certificate_chain)
            .map(|cert| cert.encoded_certificate)
            .collect();
        let mut fixture = Self {
            ta,
            issuer: AttestationKey {
                key_blob: remote_parent.key_blob,
                attest_key_params: vec![],
                issuer_subject_name: subject,
            },
            issuer_alias,
            params,
            chain,
            batch_chain,
        };
        fixture.response(fixture.chain.clone(), false);
        fixture
    }

    fn response(&mut self, chain: Vec<Vec<u8>>, unavailable: bool) {
        self.ta.dev.remote = Some(std::sync::Arc::new(TestRemote {
            cert_chain: Some(chain),
            expected_issuer: Some(self.issuer_alias.clone()),
            unavailable,
            ..TestRemote::default()
        }));
    }

    fn generate(&mut self) -> Result<KeyCreationResult, Error> {
        self.ta
            .generate_key_default(&self.params, Some(self.issuer.clone()))
    }
}

#[test]
fn native_remote_delegation_returns_only_the_child_certificate() {
    let mut fixture = DelegationFixture::new();
    let result = fixture.generate().unwrap();
    let returned: Vec<_> = result
        .certificate_chain
        .into_iter()
        .map(|cert| cert.encoded_certificate)
        .collect();
    assert_eq!(returned, fixture.chain[..1]);
    // setAttestKeyAlias callers append their existing issuer chain themselves.
    let mut caller_chain = returned;
    caller_chain.extend_from_slice(&fixture.chain[1..]);
    assert_eq!(caller_chain, fixture.chain);
    let (stored, _) = fixture
        .ta
        .keyblob_parse_decrypt(&result.key_blob, &[])
        .unwrap();
    let KeyMaterial::Remote(reference) = stored.key_material else {
        panic!("remote child")
    };
    assert_ne!(reference.alias, fixture.issuer_alias);
}

#[test]
fn remote_attestation_observer_runs_only_after_chain_validation() {
    let mut fixture = DelegationFixture::new();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    fixture.ta.dev.remote = Some(std::sync::Arc::new(TestRemote {
        cert_chain: Some(fixture.chain.clone()),
        expected_issuer: Some(fixture.issuer_alias.clone()),
        verified_calls: Some(calls.clone()),
        ..TestRemote::default()
    }));
    fixture.generate().unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

    fixture.ta.dev.remote = Some(std::sync::Arc::new(TestRemote {
        cert_chain: Some(fixture.batch_chain.clone()),
        expected_issuer: Some(fixture.issuer_alias.clone()),
        verified_calls: Some(calls.clone()),
        ..TestRemote::default()
    }));
    assert!(fixture.generate().is_err());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn remote_certificate_must_match_profile_and_is_returned_byte_exact() {
    let mut fixture = DelegationFixture::new();
    fixture.ta.dev.remote = Some(std::sync::Arc::new(TestRemote {
        version_tags: Some(device::KeyMintVersionTags {
            os_version: 160000,
            os_patchlevel: 202608,
            vendor_patchlevel: Some(20260801),
            boot_patchlevel: Some(20260801),
        }),
        cert_chain: Some(fixture.chain.clone()),
        expected_issuer: Some(fixture.issuer_alias.clone()),
        ..TestRemote::default()
    }));
    fixture
        .ta
        .update_patchlevels(202609, 20260901, 20260901)
        .unwrap();
    let result = fixture.generate().unwrap();
    assert_eq!(
        result.certificate_chain[0].encoded_certificate,
        fixture.chain[0]
    );
    fixture.ta.dev.remote = Some(std::sync::Arc::new(TestRemote {
        version_tags: Some(device::KeyMintVersionTags {
            os_version: 160000,
            os_patchlevel: 202608,
            vendor_patchlevel: Some(20260801),
            boot_patchlevel: Some(20260901),
        }),
        cert_chain: Some(fixture.chain.clone()),
        expected_issuer: Some(fixture.issuer_alias.clone()),
        ..TestRemote::default()
    }));
    assert!(matches!(
        fixture.generate().unwrap_err().kind(),
        ErrorKind::Hal(ErrorCode::VerificationFailed, _)
    ));
}

#[test]
fn native_remote_delegation_rejects_an_ignored_issuer() {
    let mut fixture = DelegationFixture::new();
    fixture.response(fixture.batch_chain.clone(), false);
    assert!(matches!(
        fixture.generate().unwrap_err().kind(),
        ErrorKind::Hal(ErrorCode::VerificationFailed, _)
    ));
}

#[test]
fn native_remote_delegation_accepts_a_valid_rsa_issuer_signature() {
    let mut fixture = DelegationFixture::with_rsa_issuer(true);
    let result = fixture.generate().unwrap();
    assert_eq!(result.certificate_chain.len(), 1);
    assert_eq!(
        result.certificate_chain[0].encoded_certificate,
        fixture.chain[0]
    );
}

#[test]
fn native_remote_delegation_requires_the_issuer_chain_for_validation() {
    let mut fixture = DelegationFixture::new();
    fixture.response(fixture.chain[..1].to_vec(), false);
    assert!(matches!(
        fixture.generate().unwrap_err().kind(),
        ErrorKind::Hal(ErrorCode::VerificationFailed, _)
    ));
}

#[test]
fn native_remote_delegation_verifies_the_child_signature() {
    let mut fixture = DelegationFixture::new();
    let mut chain = fixture.chain.clone();
    *chain[0].last_mut().unwrap() ^= 1;
    fixture.response(chain, false);
    assert!(matches!(
        fixture.generate().unwrap_err().kind(),
        ErrorKind::Hal(ErrorCode::VerificationFailed, _)
    ));
}

#[test]
fn unavailable_remote_issuer_never_falls_back_to_local_signing() {
    let mut fixture = DelegationFixture::new();
    fixture.response(vec![], true);
    assert!(matches!(
        fixture.generate().unwrap_err().kind(),
        ErrorKind::Hal(ErrorCode::UnknownError, _)
    ));
}

#[test]
fn local_attestation_child_keeps_local_patchlevels() {
    check_child_versions(false);
}

#[test]
fn local_attestation_child_preserves_user_authentication() {
    check_child_versions(true);
}

fn check_child_versions(auth_bound: bool) {
    let mut ta = test_ta();
    let ordinary_params = key_params(KeyPurpose::Sign);
    let old_key = ta.generate_key_default(&ordinary_params, None).unwrap();
    let (old_blob, _) = ta.keyblob_parse_decrypt(&old_key.key_blob, &[]).unwrap();

    let parent_params = key_params(KeyPurpose::AttestKey);
    let (material, chars) = ta.generate_key_material(&parent_params).unwrap();
    let parent = ta
        .finish_keyblob_creation(
            &parent_params,
            None,
            chars,
            material,
            keyblob::SlotPurpose::KeyGeneration,
        )
        .unwrap();
    let mut child_params = ordinary_params;
    if auth_bound {
        child_params.retain(|p| !matches!(p, KeyParam::NoAuthRequired));
        child_params.extend([
            KeyParam::UserSecureId(42),
            KeyParam::UserAuthType(HardwareAuthenticatorType::Password as u32),
            KeyParam::AuthTimeout(30),
        ]);
    }
    child_params.push(KeyParam::AttestationChallenge(b"test-challenge".to_vec()));
    child_params.push(KeyParam::AttestationApplicationId(b"test-app".to_vec()));
    let child = ta
        .generate_key_default(
            &child_params,
            Some(AttestationKey {
                key_blob: parent.key_blob,
                attest_key_params: Vec::new(),
                issuer_subject_name: tag::get_cert_subject(&parent_params).unwrap().to_vec(),
            }),
        )
        .unwrap();
    let (stored, _) = ta.keyblob_parse_decrypt(&child.key_blob, &[]).unwrap();
    let reported: Vec<_> = child
        .key_characteristics
        .iter()
        .filter(|c| c.security_level != SecurityLevel::Keystore)
        .cloned()
        .collect();
    assert_eq!(stored.characteristics, reported);
    let cert_rot =
        cert::parse_remote_root_of_trust(&child.certificate_chain[0].encoded_certificate)
            .unwrap()
            .unwrap();
    let auths = tag::characteristics_at(&reported, SecurityLevel::TrustedEnvironment).unwrap();
    for (param, expected) in [
        (KeyParam::OsVersion(cert_rot.os_version.unwrap()), 160000),
        (
            KeyParam::OsPatchlevel(cert_rot.os_patchlevel.unwrap()),
            202608,
        ),
        (
            KeyParam::VendorPatchlevel(cert_rot.vendor_patchlevel.unwrap()),
            20260801,
        ),
        (
            KeyParam::BootPatchlevel(cert_rot.boot_patchlevel.unwrap()),
            20260801,
        ),
    ] {
        assert!(
            auths.contains(&param),
            "certificate tag missing from metadata: {param:?}"
        );
        let value = match param {
            KeyParam::OsVersion(v)
            | KeyParam::OsPatchlevel(v)
            | KeyParam::VendorPatchlevel(v)
            | KeyParam::BootPatchlevel(v) => v,
            _ => unreachable!(),
        };
        assert_eq!(value, expected);
    }
    assert!(auths.contains(&KeyParam::Purpose(KeyPurpose::Sign)));
    assert_eq!(auths.contains(&KeyParam::NoAuthRequired), !auth_bound);
    if auth_bound {
        assert!(auths.contains(&KeyParam::UserSecureId(42)));
        assert!(auths.contains(&KeyParam::UserAuthType(
            HardwareAuthenticatorType::Password as u32
        )));
        assert!(auths.contains(&KeyParam::AuthTimeout(30)));
    }
    let (unchanged, _) = ta.keyblob_parse_decrypt(&old_key.key_blob, &[]).unwrap();
    assert_eq!(unchanged.characteristics, old_blob.characteristics);
    assert_eq!(unchanged.key_material, old_blob.key_material);
}

#[test]
fn operation_limit_mirrors_the_mirrored_implementation() {
    let mut ta = test_ta();
    assert_eq!(ta.operations.len(), 16);
    assert!(ta.set_max_operations(0).is_err());
    assert!(ta.set_max_operations(usize::MAX).is_err());
    ta.set_max_operations(32).unwrap();
    assert_eq!(ta.operations.len(), 32);
}

#[test]
fn relay_leaf_serial_is_the_aosp_default_unless_the_caller_set_it() {
    assert_eq!(effective_remote_serial(None), Some(&[1u8][..]));
    assert_eq!(effective_remote_serial(Some(&[])), Some(&[1u8][..]));
    assert_eq!(
        effective_remote_serial(Some(&[0x2a, 0x01])),
        Some(&[0x2a, 0x01][..])
    );
}
