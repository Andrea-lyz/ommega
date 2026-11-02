//! `ProtectedData`: the DICE chain and a signed MAC key, encrypted to the server's EEK with
//! COSE_Encrypt (ECDH-ES + HKDF-256, AES-256-GCM).

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use anyhow::{Context, Result, anyhow};
use ciborium::value::Value;
use hkdf::Hkdf;
use hkdf::hmac::{Hmac, Mac, digest::KeyInit as HmacKeyInit};
use p256::{
    PublicKey as P256PublicKey, SecretKey as P256SecretKey, ecdh::diffie_hellman,
    elliptic_curve::sec1::ToSec1Point,
};
use sha2::Sha256;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

use crate::{
    cbor::{bytes, empty_map, encode, int, text},
    error::RkpError,
    profile::DeviceInfo,
};

use super::{
    ALG_A256GCM, ALG_ECDH_ES_HKDF_256, ALG_HMAC_256, RPC_CURVE_25519, RPC_CURVE_P256,
    csr::{build_cose_structure, device_info_to_cbor},
    fill_random,
    keys::DeviceKeys,
    random_bytes_array,
};

pub(super) struct ProtectedDataBundle {
    pub(super) protected_data_bytes: Vec<u8>,
    pub(super) protected_data_len: usize,
}

type TransportKey = (Vec<u8>, Value, [u8; 32]);

#[allow(clippy::too_many_arguments)]
pub(super) fn build_protected_data(
    keys: &DeviceKeys,
    challenge: &[u8],
    keys_to_sign: &[Value],
    eek_pub_bytes: &[u8],
    eek_key_id: &[u8],
    device_info: &DeviceInfo,
    eek_curve: i128,
    dice: Value,
) -> Result<ProtectedDataBundle> {
    let keys_to_sign_cbor = encode(&Value::Array(keys_to_sign.to_vec()))?;
    let mac_key = random_bytes_array::<32>()?;
    let keys_to_sign_mac = build_keys_to_sign_mac(&mac_key, &keys_to_sign_cbor)?;
    let signed_mac_protected = encode(&Value::Map(vec![(int(1), int(keys.algorithm()))]))?;
    let signed_mac_aad = encode(&Value::Array(vec![
        bytes(challenge.to_vec()),
        device_info_to_cbor(device_info)?,
        bytes(keys_to_sign_mac),
    ]))?;
    let signed_mac_input = build_cose_structure(
        "Signature1",
        &signed_mac_protected,
        &signed_mac_aad,
        &mac_key,
    )?;
    let signed_mac = Value::Array(vec![
        bytes(signed_mac_protected),
        empty_map(),
        bytes(mac_key.to_vec()),
        bytes(keys.sign(&signed_mac_input)),
    ]);
    let plaintext = encode(&Value::Array(vec![signed_mac, dice]))?;

    let (_ephemeral_public, sender_cose_key, aes_key) =
        derive_transport_key(eek_curve, eek_pub_bytes)?;

    let mut nonce = [0_u8; 12];
    fill_random(&mut nonce)?;

    let protected = encode(&Value::Map(vec![(int(1), int(ALG_A256GCM))]))?;
    let aad = encode(&Value::Array(vec![
        text("Encrypt"),
        bytes(protected.clone()),
        bytes(Vec::new()),
    ]))?;
    let cipher = Aes256Gcm::new_from_slice(&aes_key).context("create AES-256-GCM")?;
    let ciphertext = cipher
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("encrypt protected data"))?;

    let recipient_protected = encode(&Value::Map(vec![(int(1), int(ALG_ECDH_ES_HKDF_256))]))?;
    let mut recipient_unprotected = vec![(int(-1), sender_cose_key)];
    if !eek_key_id.is_empty() {
        recipient_unprotected.push((int(4), bytes(eek_key_id.to_vec())));
    }
    let recipient = Value::Array(vec![
        bytes(recipient_protected),
        Value::Map(recipient_unprotected),
        Value::Null,
    ]);
    let encrypt = Value::Array(vec![
        bytes(protected),
        Value::Map(vec![(int(5), bytes(nonce.to_vec()))]),
        bytes(ciphertext),
        Value::Array(vec![recipient]),
    ]);
    let protected_data_bytes = encode(&encrypt)?;

    Ok(ProtectedDataBundle {
        protected_data_len: protected_data_bytes.len(),
        protected_data_bytes,
    })
}

fn derive_transport_key(eek_curve: i128, eek_pub_bytes: &[u8]) -> Result<TransportKey> {
    match eek_curve {
        RPC_CURVE_25519 => derive_x25519_transport_key(eek_pub_bytes),
        RPC_CURVE_P256 => derive_p256_transport_key(eek_pub_bytes),
        other => Err(RkpError::UnsupportedEekCurve(other).into()),
    }
}

fn derive_x25519_transport_key(server_pub_bytes: &[u8]) -> Result<TransportKey> {
    let ephemeral_secret = StaticSecret::from(random_bytes_array::<32>()?);
    let ephemeral_public = X25519PublicKey::from(&ephemeral_secret);
    let client_pub_bytes = ephemeral_public.as_bytes();

    let server_pub = X25519PublicKey::from(
        <[u8; 32]>::try_from(server_pub_bytes)
            .map_err(|_| anyhow!("EEK public key must be 32 bytes"))?,
    );
    let shared = ephemeral_secret.diffie_hellman(&server_pub);
    let output = hkdf_expand(
        shared.as_bytes(),
        &encode_kdf_context(client_pub_bytes, server_pub_bytes)?,
    )?;

    Ok((
        client_pub_bytes.to_vec(),
        Value::Map(vec![
            (int(1), int(1)),
            (int(-1), int(4)),
            (int(-2), bytes(client_pub_bytes.to_vec())),
        ]),
        output,
    ))
}

fn derive_p256_transport_key(server_pub_bytes: &[u8]) -> Result<TransportKey> {
    let ephemeral_secret = generate_p256_secret_key()?;
    let encoded = ephemeral_secret.public_key().to_sec1_point(false);
    let x = encoded
        .x()
        .context("missing ephemeral P-256 x coordinate")?;
    let y = encoded
        .y()
        .context("missing ephemeral P-256 y coordinate")?;
    let client_pub_bytes = [x.as_slice(), y.as_slice()].concat();

    let server_public = parse_p256_public_key(server_pub_bytes)?;
    let shared = diffie_hellman(
        ephemeral_secret.to_nonzero_scalar(),
        server_public.as_affine(),
    );
    let output = hkdf_expand(
        shared.raw_secret_bytes(),
        &encode_kdf_context(&client_pub_bytes, server_pub_bytes)?,
    )?;

    Ok((
        client_pub_bytes,
        Value::Map(vec![
            (int(1), int(2)),
            (int(-1), int(1)),
            (int(-2), bytes(x.to_vec())),
            (int(-3), bytes(y.to_vec())),
        ]),
        output,
    ))
}

fn hkdf_expand(shared_secret: &[u8], context: &[u8]) -> Result<[u8; 32]> {
    let hkdf = Hkdf::<Sha256>::new(None, shared_secret);
    let mut output = [0_u8; 32];
    hkdf.expand(context, &mut output)
        .map_err(|_| anyhow!("expand HKDF"))?;
    Ok(output)
}

fn encode_kdf_context(client_pub_bytes: &[u8], server_pub_bytes: &[u8]) -> Result<Vec<u8>> {
    encode(&Value::Array(vec![
        int(ALG_A256GCM),
        Value::Array(vec![
            text("client"),
            bytes(Vec::new()),
            bytes(client_pub_bytes.to_vec()),
        ]),
        Value::Array(vec![
            text("server"),
            bytes(Vec::new()),
            bytes(server_pub_bytes.to_vec()),
        ]),
        Value::Array(vec![int(256), bytes(Vec::new())]),
    ]))
}

fn generate_p256_secret_key() -> Result<P256SecretKey> {
    loop {
        if let Ok(secret_key) = P256SecretKey::from_slice(&random_bytes_array::<32>()?) {
            return Ok(secret_key);
        }
    }
}

fn parse_p256_public_key(public_key_bytes: &[u8]) -> Result<P256PublicKey> {
    if public_key_bytes.len() != 64 {
        return Err(anyhow!(
            "P-256 EEK public key must be 64 bytes of x||y coordinates"
        ));
    }

    let sec1 = [&[0x04], public_key_bytes].concat();
    P256PublicKey::from_sec1_bytes(&sec1).map_err(|_| anyhow!("parse P-256 EEK public key"))
}

/// HMAC-SHA256 over the COSE MAC0 structure containing the requested public keys.
pub fn key_mac_tag(mac_key: &[u8; 32], keys_to_sign: &[Value]) -> Result<[u8; 32]> {
    let encoded = encode(&Value::Array(keys_to_sign.to_vec()))?;
    build_keys_to_sign_mac(mac_key, &encoded)?
        .try_into()
        .map_err(|_| anyhow!("HMAC-SHA256 tag must contain 32 bytes"))
}

/// Legacy COSE_Mac0 with the exact key-array payload used to calculate its tag.
pub fn cose_mac0(keys_to_sign: &[Value], tag: &[u8; 32]) -> Result<Vec<u8>> {
    encode(&Value::Array(vec![
        bytes(encode(&Value::Map(vec![(int(1), int(ALG_HMAC_256))]))?),
        empty_map(),
        bytes(encode(&Value::Array(keys_to_sign.to_vec()))?),
        bytes(tag.to_vec()),
    ]))
}

fn build_keys_to_sign_mac(mac_key: &[u8; 32], keys_to_sign_cbor: &[u8]) -> Result<Vec<u8>> {
    let mac_protected = encode(&Value::Map(vec![(int(1), int(ALG_HMAC_256))]))?;
    let mac_structure = build_cose_structure("MAC0", &mac_protected, &[], keys_to_sign_cbor)?;
    let mut mac = <Hmac<Sha256> as HmacKeyInit>::new_from_slice(mac_key)
        .map_err(|error| anyhow!("create HMAC-SHA256: {error}"))?;
    mac.update(&mac_structure);
    Ok(mac.finalize().into_bytes().to_vec())
}
