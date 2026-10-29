use super::*;

#[test]
fn bad_interface_marker_is_rejected() {
    let blob_key = blob_key_descriptor(None);
    let mut request = build_request_with_marker(KEYSTORE_SECURITY_LEVEL_INTERFACE, 0, |parcel| {
        parcel.write(&blob_key).unwrap()
    });
    let (data, data_size, offsets, offsets_size) = raw_parts(&mut request);

    assert_status_code(
        unsafe {
            parse_security_level_request(
                data,
                data_size,
                offsets,
                offsets_size,
                crate::android::system::keystore2::IKeystoreSecurityLevel::transactions::r#deleteKey,
            )
        },
        StatusCode::BadType,
    );
}

#[test]
fn generated_key_descriptor_round_trip_preserves_non_null_empty_blob() {
    let key = blob_key_descriptor(Some(Vec::new()));
    let mut parcel = Parcel::new();
    parcel.write(&key).unwrap();
    parcel.set_data_position(0);

    let decoded: KeyDescriptor = parcel.read().unwrap();
    assert_eq!(decoded.blob, Some(Vec::new()));
}

#[test]
fn nonzero_parcelable_markers_preserve_service_request_routing_inputs() {
    use rsbinder::Parcelable;

    let key = blob_key_descriptor(Some(Vec::new()));
    use crate::android::system::keystore2::IKeystoreService::transactions;
    for marker in [1i32, 2, -1, i32::MAX] {
        for code in [
            transactions::r#getKeyEntry,
            transactions::r#updateSubcomponent,
            transactions::r#deleteKey,
        ] {
            let parsed = parse_service_request_with_payload(code, |parcel| {
                parcel.write(&marker).unwrap();
                key.write_to_parcel(parcel).unwrap();
                if code == transactions::r#updateSubcomponent {
                    parcel.write(&Option::<Vec<u8>>::None).unwrap();
                    parcel.write(&Some(vec![1u8, 2, 3])).unwrap();
                }
            })
            .unwrap();
            let decoded = match parsed {
                ParsedServiceRequest::GetKeyEntry { key }
                | ParsedServiceRequest::UpdateSubcomponent { key, .. }
                | ParsedServiceRequest::DeleteKey { key } => key,
                _ => panic!("unexpected request"),
            };
            assert_eq!(decoded, key);
        }
    }
}

#[test]
fn nonzero_parcelable_markers_work_for_optional_and_in_place_reads() {
    use rsbinder::{Deserialize, DeserializeOption, Parcelable};

    let key = blob_key_descriptor(Some(Vec::new()));
    for marker in [1i32, 2, -1, i32::MAX] {
        let mut parcel = Parcel::new();
        parcel.write(&marker).unwrap();
        key.write_to_parcel(&mut parcel).unwrap();
        parcel.set_data_position(0);
        let mut decoded = KeyDescriptor::default();
        decoded.deserialize_from(&mut parcel).unwrap();
        assert_eq!(decoded, key);
        parcel.set_data_position(0);
        let mut optional = None;
        KeyDescriptor::deserialize_option_from(&mut optional, &mut parcel).unwrap();
        assert_eq!(optional.as_ref(), Some(&key));
    }
    let mut parcel = Parcel::new();
    parcel.write(&0i32).unwrap();
    parcel.set_data_position(0);
    let mut optional = Some(key);
    KeyDescriptor::deserialize_option_from(&mut optional, &mut parcel).unwrap();
    assert_eq!(optional, None);
}

#[test]
fn optional_attestation_keys_accept_nonzero_parcelable_markers() {
    use crate::android::system::keystore2::IKeystoreSecurityLevel::transactions;
    use rsbinder::Parcelable;

    let key = blob_key_descriptor(None);
    for marker in [2i32, -1, i32::MAX] {
        let parsed =
            parse_security_level_request_with_payload(transactions::r#generateKey, |parcel| {
                parcel.write(&key).unwrap();
                parcel.write(&marker).unwrap();
                key.write_to_parcel(parcel).unwrap();
                parcel.write(&Vec::<KeyParameter>::new()).unwrap();
                parcel.write(&0i32).unwrap();
                parcel.write(&Vec::<u8>::new()).unwrap();
            })
            .unwrap();
        assert!(
            matches!(parsed, ParsedSecurityLevelRequest::GenerateKey { attestation_key: Some(value), .. } if value == key)
        );
    }
}

#[test]
fn generated_deserializers_reject_invalid_lengths_and_truncated_parcelables() {
    assert_status_code(
        parse_authorization_request_as_method(AuthorizationMethod::OnDeviceUnlocked, |parcel| {
            parcel.write(&0i32).unwrap();
            parcel.write(&-2i32).unwrap();
        }),
        StatusCode::UnexpectedNull,
    );
    assert_status_code(
        parse_operation_request_with_payload(
            crate::android::system::keystore2::IKeystoreOperation::transactions::r#update,
            |parcel| parcel.write(&-2i32).unwrap(),
        ),
        StatusCode::UnexpectedNull,
    );
    assert_status_code(
        parse_security_level_request_with_payload(
            crate::android::system::keystore2::IKeystoreSecurityLevel::transactions::r#deleteKey,
            |parcel| parcel.write(&2i32).unwrap(),
        ),
        StatusCode::NotEnoughData,
    );
    assert_status_code(
        parse_security_level_request_with_payload(
            crate::android::system::keystore2::IKeystoreSecurityLevel::transactions::r#generateKey,
            |parcel| {
                parcel.write(&blob_key_descriptor(None)).unwrap();
                parcel.write(&2i32).unwrap();
            },
        ),
        StatusCode::NotEnoughData,
    );
}
