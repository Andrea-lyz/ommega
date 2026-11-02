use anyhow::{Context, Result};
use ciborium::value::Value;
use ed25519_dalek::{Signer as Ed25519Signer, SigningKey};
use p256::{
    FieldBytes as P256FieldBytes, NonZeroScalar as P256NonZeroScalar, SecretKey as P256SecretKey,
    ecdsa::{
        Signature as P256Signature, SigningKey as P256SigningKey, signature::Signer as P256Signer,
    },
    elliptic_curve::ops::Reduce,
};
use serde::Serialize;

use crate::{
    cbor::{bytes, int},
    profile::DiceCurve,
};

use super::{ALG_EDDSA, ALG_ES256, fill_random};

#[derive(Debug, Clone)]
enum DeviceKeyMaterial {
    Ed25519(SigningKey),
    P256(P256SigningKey),
}

/// The DICE CDI_Leaf key pair derived deterministically from a 32-byte seed.
#[derive(Debug, Clone)]
pub struct DeviceKeys {
    seed: [u8; 32],
    curve: DiceCurve,
    material: DeviceKeyMaterial,
}

#[derive(Debug, Clone, Serialize)]
pub struct EcKeyPair {
    #[serde(skip)]
    pub secret_key: P256SecretKey,
    #[serde(skip)]
    pub signing_key: P256SigningKey,
    #[serde(skip)]
    pub cose_public: Value,
}

impl DeviceKeys {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self::from_seed_with_curve(seed, DiceCurve::Ed25519)
    }

    pub fn from_seed_with_curve(seed: [u8; 32], curve: DiceCurve) -> Self {
        let material = match curve {
            DiceCurve::Ed25519 => DeviceKeyMaterial::Ed25519(SigningKey::from_bytes(&seed)),
            DiceCurve::P256 => {
                // Profiles persist only the seed, so this derivation (big-endian
                // `seed mod (n - 1) + 1`) must never change.
                let scalar = <P256NonZeroScalar as Reduce<P256FieldBytes>>::reduce(
                    &P256FieldBytes::from(seed),
                );
                let secret_key: P256SecretKey = scalar.into();
                DeviceKeyMaterial::P256(P256SigningKey::from(secret_key))
            }
        };

        Self {
            seed,
            curve,
            material,
        }
    }

    pub fn curve(&self) -> DiceCurve {
        self.curve
    }

    pub fn algorithm(&self) -> i128 {
        match self.curve {
            DiceCurve::Ed25519 => ALG_EDDSA,
            DiceCurve::P256 => ALG_ES256,
        }
    }

    pub fn seed_hex(&self) -> String {
        hex::encode(self.seed)
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key_bytes())
    }

    /// Raw public key: 32 bytes for Ed25519, `x || y` for P-256.
    pub fn public_key_bytes(&self) -> Vec<u8> {
        match &self.material {
            DeviceKeyMaterial::Ed25519(signing_key) => {
                signing_key.verifying_key().to_bytes().to_vec()
            }
            DeviceKeyMaterial::P256(signing_key) => {
                let (x, y) = p256_coordinates(signing_key);
                [x, y].concat()
            }
        }
    }

    pub fn sign(&self, payload: &[u8]) -> Vec<u8> {
        match &self.material {
            DeviceKeyMaterial::Ed25519(signing_key) => Ed25519Signer::sign(signing_key, payload)
                .to_bytes()
                .to_vec(),
            DeviceKeyMaterial::P256(signing_key) => {
                let signature: P256Signature = P256Signer::sign(signing_key, payload);
                signature.to_bytes().to_vec()
            }
        }
    }

    pub fn cose_key(&self) -> Value {
        match &self.material {
            DeviceKeyMaterial::Ed25519(_) => Value::Map(vec![
                (int(1), int(1)),
                (int(3), int(ALG_EDDSA)),
                (int(-1), int(6)),
                (int(-2), bytes(self.public_key_bytes())),
            ]),
            DeviceKeyMaterial::P256(signing_key) => {
                let (x, y) = p256_coordinates(signing_key);
                p256_cose_key(x, y)
            }
        }
    }
}

/// Generates a fresh P-256 key for `keysToSign`.
pub fn generate_ec_keypair() -> Result<EcKeyPair> {
    loop {
        let mut secret_bytes = [0_u8; 32];
        fill_random(&mut secret_bytes)?;

        if let Ok(secret_key) = P256SecretKey::from_slice(&secret_bytes) {
            let signing_key = P256SigningKey::from(secret_key.clone());
            let encoded = signing_key.verifying_key().to_sec1_point(false);
            let x = encoded.x().context("missing P-256 x coordinate")?.to_vec();
            let y = encoded.y().context("missing P-256 y coordinate")?.to_vec();

            return Ok(EcKeyPair {
                secret_key,
                signing_key,
                cose_public: p256_cose_key(x, y),
            });
        }
    }
}

fn p256_coordinates(signing_key: &P256SigningKey) -> (Vec<u8>, Vec<u8>) {
    let encoded = signing_key.verifying_key().to_sec1_point(false);
    let x = encoded
        .x()
        .expect("uncompressed P-256 point has an x coordinate");
    let y = encoded
        .y()
        .expect("uncompressed P-256 point has a y coordinate");
    (x.to_vec(), y.to_vec())
}

fn p256_cose_key(x: Vec<u8>, y: Vec<u8>) -> Value {
    Value::Map(vec![
        (int(1), int(2)),
        (int(3), int(ALG_ES256)),
        (int(-1), int(1)),
        (int(-2), bytes(x)),
        (int(-3), bytes(y)),
    ])
}
