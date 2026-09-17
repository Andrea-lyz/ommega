use super::*;
use crate::{HardwareInfo, KeyMintHalVersion, KeyMintTa, RpcInfo, RpcInfoV3};
use kmr_wire::keymint::{Algorithm, DateTime};

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

struct TestRemote;
impl device::RemoteBackend for TestRemote {
    fn attest(
        &self,
        _: &[u8],
        _: &[u8],
        _: &str,
        _: Option<&[u8]>,
        _: &device::RemoteAttestParams,
    ) -> Result<Option<device::RemoteAttestation>, Error> {
        panic!("child generation must not request a remote key")
    }
    fn sign(&self, _: &str, _: &[u8], _: &str) -> Result<Option<Vec<u8>>, Error> {
        // Only certificate encoding/metadata are under test, not the signature.
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
            remote: Some(Box::new(TestRemote)),
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
fn remote_child_certificate_metadata_and_blob_share_patchlevels() {
    check_child_versions(true, false);
}

#[test]
fn local_attestation_child_keeps_local_patchlevels() {
    check_child_versions(false, false);
}

#[test]
fn remote_child_version_normalization_preserves_user_authentication() {
    check_child_versions(true, true);
}

fn check_child_versions(remote: bool, auth_bound: bool) {
    let mut ta = test_ta();
    let ordinary_params = key_params(KeyPurpose::Sign);
    let old_key = ta.generate_key(&ordinary_params, None).unwrap();
    let (old_blob, _) = ta.keyblob_parse_decrypt(&old_key.key_blob, &[]).unwrap();

    let parent_params = key_params(KeyPurpose::AttestKey);
    let (mut material, chars) = ta.generate_key_material(&parent_params).unwrap();
    let rot = remote_rot();
    if remote {
        let spki = material
            .subject_public_key_info(&mut Vec::new(), &*ta.imp.ec, &*ta.imp.rsa, &*ta.imp.mldsa)
            .unwrap()
            .unwrap()
            .to_der()
            .unwrap();
        material = KeyMaterial::Remote(crypto::RemoteRef {
            alias: "unit-test".into(),
            public_key: spki,
            root_of_trust: Some(rot.clone()),
        });
    }
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
        .generate_key(
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
        (
            KeyParam::OsVersion(cert_rot.os_version.unwrap()),
            if remote { 150000 } else { 160000 },
        ),
        (
            KeyParam::OsPatchlevel(cert_rot.os_patchlevel.unwrap()),
            if remote { 202512 } else { 202608 },
        ),
        (
            KeyParam::VendorPatchlevel(cert_rot.vendor_patchlevel.unwrap()),
            if remote { 20251205 } else { 20260801 },
        ),
        (
            KeyParam::BootPatchlevel(cert_rot.boot_patchlevel.unwrap()),
            if remote { 20251201 } else { 20260801 },
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
