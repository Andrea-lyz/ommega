use anyhow::{Result, ensure};
use ciborium::value::Value;

use super::{build_sig_structure, device_info_to_cbor};
use crate::{
    cbor::{bytes, empty_map, encode, int, text},
    material::ImportedMaterial,
    profile::DeviceInfo,
    verify::verify_csr,
};

pub(crate) fn build(
    material: &ImportedMaterial,
    device: &DeviceInfo,
    challenge: &[u8],
    keys: &[Value],
) -> Result<Vec<u8>> {
    ensure!(challenge.len() <= 64, "RKP challenge exceeds 64 bytes");
    let payload = encode(&Value::Array(vec![
        int(3),
        text("keymint"),
        device_info_to_cbor(device)?,
        Value::Array(keys.to_vec()),
    ]))?;
    let signed_payload = encode(&Value::Array(vec![
        bytes(challenge.to_vec()),
        bytes(payload),
    ]))?;
    let protected = encode(&Value::Map(vec![(
        int(1),
        int(material.signer.algorithm()),
    )]))?;
    let input = build_sig_structure("Signature1", &protected, &signed_payload)?;
    let signed = encode(&Value::Array(vec![
        bytes(protected),
        empty_map(),
        bytes(signed_payload),
        bytes(material.signer.sign(&input)),
    ]))?;

    // splice the validated CBOR items verbatim so imported chain bytes remain intact
    let mut csr = vec![0x84, 0x01];
    csr.extend_from_slice(&material.uds_certs_bytes);
    csr.extend_from_slice(&material.chain_bytes);
    csr.extend_from_slice(&signed);
    ensure!(
        verify_csr(&csr)?.signature_valid,
        "new CSR signature is invalid"
    );
    Ok(csr)
}
