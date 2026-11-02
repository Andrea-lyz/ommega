use anyhow::Result;
use ciborium::value::Value;
use serde::Serialize;

use crate::{
    cbor::{bytes, empty_map, encode, int, text},
    error::RkpError,
    profile::DeviceInfo,
};

use super::{
    CWT_ISSUER, CWT_SUBJECT, DICE_KEY_USAGE, DICE_SUBJECT_PUB_KEY, RPC_CURVE_25519, RPC_CURVE_P256,
    keys::DeviceKeys, protected::build_protected_data,
};

#[derive(Debug, Clone, Serialize)]
pub struct CsrBundle {
    pub csr_bytes: Vec<u8>,
    pub protected_data_bytes: Vec<u8>,
    pub protected_data_len: usize,
}

/// constructs the experimental degenerate-chain CSR without legacy EEK encryption
pub fn build_experimental_csr_v3(
    keys: &DeviceKeys,
    challenge: &[u8],
    keys_to_sign: &[Value],
    device_info: &DeviceInfo,
) -> Result<Vec<u8>> {
    let csr_payload = encode(&Value::Array(vec![
        int(3),
        text("keymint"),
        device_info_to_cbor(device_info)?,
        Value::Array(keys_to_sign.to_vec()),
    ]))?;
    let signed = cose_sign1(
        keys,
        encode(&Value::Array(vec![bytes(challenge), bytes(csr_payload)]))?,
    )?;
    encode(&Value::Array(vec![
        int(1),
        empty_map(),
        build_dice_chain(keys, device_info)?,
        signed,
    ]))
}

pub fn build_csr(
    keys: &DeviceKeys,
    challenge: &[u8],
    keys_to_sign: &[Value],
    eek_pub_bytes: &[u8],
    eek_key_id: &[u8],
    device_info: &DeviceInfo,
    eek_curve: i128,
) -> Result<CsrBundle> {
    match eek_curve {
        RPC_CURVE_25519 | RPC_CURVE_P256 => {}
        other => return Err(RkpError::UnsupportedEekCurve(other).into()),
    }

    let csr_payload = encode(&Value::Array(vec![
        int(3),
        text("keymint"),
        device_info_to_cbor(device_info)?,
        Value::Array(keys_to_sign.to_vec()),
    ]))?;

    let dice = build_dice_chain(keys, device_info)?;
    let protected = build_protected_data(
        keys,
        challenge,
        keys_to_sign,
        eek_pub_bytes,
        eek_key_id,
        device_info,
        eek_curve,
        dice.clone(),
    )?;
    let signed_payload = encode(&Value::Array(vec![
        bytes(challenge.to_vec()),
        bytes(csr_payload),
    ]))?;
    let signed_data = cose_sign1(keys, signed_payload)?;

    let csr = Value::Array(vec![int(1), empty_map(), dice, signed_data]);
    let csr_bytes = encode(&csr)?;

    Ok(CsrBundle {
        csr_bytes,
        protected_data_bytes: protected.protected_data_bytes,
        protected_data_len: protected.protected_data_len,
    })
}

pub fn build_sig_structure(context: &str, protected: &[u8], payload: &[u8]) -> Result<Vec<u8>> {
    build_cose_structure(context, protected, &[], payload)
}

pub(super) fn build_cose_structure(
    context: &str,
    protected: &[u8],
    external_aad: &[u8],
    payload: &[u8],
) -> Result<Vec<u8>> {
    encode(&Value::Array(vec![
        text(context),
        bytes(protected.to_vec()),
        bytes(external_aad.to_vec()),
        bytes(payload.to_vec()),
    ]))
}

pub fn device_info_to_cbor(device_info: &DeviceInfo) -> Result<Value> {
    let mut entries = vec![
        (text("brand"), text(device_info.brand.clone())),
        (text("manufacturer"), text(device_info.manufacturer.clone())),
        (text("product"), text(device_info.product.clone())),
        (text("model"), text(device_info.model.clone())),
        (text("device"), text(device_info.device.clone())),
        (text("vb_state"), text(device_info.vb_state.clone())),
        (
            text("bootloader_state"),
            text(device_info.bootloader_state.clone()),
        ),
        (
            text("system_patch_level"),
            int(i128::from(device_info.system_patch_level)),
        ),
        (
            text("boot_patch_level"),
            int(i128::from(device_info.boot_patch_level)),
        ),
        (
            text("vendor_patch_level"),
            int(i128::from(device_info.vendor_patch_level)),
        ),
        (
            text("security_level"),
            text(device_info.security_level.clone()),
        ),
        (text("fused"), int(i128::from(device_info.fused))),
    ];

    if let Some(vbmeta_digest) = device_info.vbmeta_digest.as_deref() {
        entries.push((text("vbmeta_digest"), bytes(hex::decode(vbmeta_digest)?)));
    }

    if !device_info.os_version.trim().is_empty() {
        entries.push((text("os_version"), text(device_info.os_version.clone())));
    }

    Ok(Value::Map(entries))
}

pub(super) fn build_dice_chain(keys: &DeviceKeys, device_info: &DeviceInfo) -> Result<Value> {
    Ok(Value::Array(vec![
        keys.cose_key(),
        build_dice_entry(keys, device_info)?,
    ]))
}

fn build_dice_entry(keys: &DeviceKeys, device_info: &DeviceInfo) -> Result<Value> {
    let payload = encode(&Value::Map(vec![
        (int(CWT_ISSUER), text(device_info.dice_issuer.clone())),
        (int(CWT_SUBJECT), text(device_info.dice_subject.clone())),
        (int(DICE_SUBJECT_PUB_KEY), bytes(encode(&keys.cose_key())?)),
        (int(DICE_KEY_USAGE), bytes(vec![0x20])),
    ]))?;

    cose_sign1(keys, payload)
}

fn cose_sign1(keys: &DeviceKeys, payload: Vec<u8>) -> Result<Value> {
    let protected = encode(&Value::Map(vec![(int(1), int(keys.algorithm()))]))?;
    let signature_input = build_sig_structure("Signature1", &protected, &payload)?;

    Ok(Value::Array(vec![
        bytes(protected),
        empty_map(),
        bytes(payload),
        bytes(keys.sign(&signature_input)),
    ]))
}
