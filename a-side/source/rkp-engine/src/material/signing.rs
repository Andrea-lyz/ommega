use anyhow::{Result, bail, ensure};
use ed25519_dalek::{Signer as _, SigningKey as Ed25519Key};
use hkdf::Hkdf;
use p256::{FieldBytes, NonZeroScalar, elliptic_curve::ops::Reduce};
use serde::Deserialize;
use sha2::Sha512;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use super::MaterialError;
use crate::verify::DiceChainReport;

pub(super) const OPEN_DICE_DERIVATION: &str = "open-dice-hkdf-sha512-ed25519";
const LEGACY_P256_DERIVATION: &str = "duck-rkp-legacy-mod-n-minus-1-plus-1";

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub(super) struct SigningInput {
    pub kind: String,
    pub bytes_hex: String,
    pub derivation: Option<String>,
}

pub(crate) enum Signer {
    Ed25519(Ed25519Key),
    P256(p256::ecdsa::SigningKey),
    P384(p384::ecdsa::SigningKey),
}

impl Signer {
    pub(crate) fn algorithm(&self) -> i128 {
        match self {
            Self::Ed25519(_) => -8,
            Self::P256(_) => -7,
            Self::P384(_) => -35,
        }
    }

    pub(crate) fn public_key(&self) -> Vec<u8> {
        match self {
            Self::Ed25519(key) => key.verifying_key().to_bytes().to_vec(),
            Self::P256(key) => key.verifying_key().to_sec1_point(false).as_bytes()[1..].to_vec(),
            Self::P384(key) => key.verifying_key().to_sec1_point(false).as_bytes()[1..].to_vec(),
        }
    }

    pub(crate) fn sign(&self, message: &[u8]) -> Vec<u8> {
        match self {
            Self::Ed25519(key) => key.sign(message).to_bytes().to_vec(),
            Self::P256(key) => {
                let signature: p256::ecdsa::Signature = key.sign(message);
                signature.to_bytes().to_vec()
            }
            Self::P384(key) => {
                let signature: p384::ecdsa::Signature = key.sign(message);
                signature.to_bytes().to_vec()
            }
        }
    }
}

pub(super) fn resolve(input: &SigningInput, chain: &DiceChainReport) -> Result<(Signer, String)> {
    let bytes =
        Zeroizing::new(hex::decode(&input.bytes_hex).map_err(|_| MaterialError::Structure)?);
    let derivation = input.derivation.as_deref();
    if input.kind != "auto" {
        let signer = explicit(&input.kind, derivation, &bytes)?;
        ensure!(matches_leaf(&signer, chain), MaterialError::KeyMismatch);
        return Ok((signer, input.kind.clone()));
    }
    ensure!(
        derivation.is_none() || derivation == Some("auto"),
        MaterialError::Unsupported
    );
    let mut matches = Vec::new();
    for (kind, derivation) in [
        ("ed25519-seed", None),
        ("p256-scalar", None),
        ("p384-scalar", None),
        ("p256-seed", Some(LEGACY_P256_DERIVATION)),
        ("cdi-attest", Some(OPEN_DICE_DERIVATION)),
    ] {
        if let Ok(signer) = explicit(kind, derivation, &bytes)
            && matches_leaf(&signer, chain)
        {
            matches.push((signer, kind.to_owned()));
        }
    }
    select_unique(matches)
}

pub(super) fn select_unique(mut matches: Vec<(Signer, String)>) -> Result<(Signer, String)> {
    ensure!(!matches.is_empty(), MaterialError::KeyMismatch);
    ensure!(matches.len() == 1, MaterialError::AmbiguousMatch);
    Ok(matches.remove(0))
}

fn matches_leaf(signer: &Signer, chain: &DiceChainReport) -> bool {
    signer.algorithm() == chain.leaf_algorithm && signer.public_key() == chain.leaf_public_key
}

fn explicit(kind: &str, derivation: Option<&str>, bytes: &[u8]) -> Result<Signer> {
    match kind {
        "ed25519-seed" => {
            ensure!(
                matches!(derivation, None | Some("direct") | Some("auto")),
                MaterialError::Unsupported
            );
            let seed = fixed_seed(bytes)?;
            Ok(Signer::Ed25519(Ed25519Key::from_bytes(&seed)))
        }
        "p256-scalar" => {
            ensure!(
                matches!(derivation, None | Some("none") | Some("auto")),
                MaterialError::Unsupported
            );
            ensure!(bytes.len() == 32, MaterialError::Structure);
            Ok(Signer::P256(p256::ecdsa::SigningKey::from(
                p256::SecretKey::from_slice(bytes).map_err(|_| MaterialError::Structure)?,
            )))
        }
        "p384-scalar" => {
            ensure!(
                matches!(derivation, None | Some("none") | Some("auto")),
                MaterialError::Unsupported
            );
            ensure!(bytes.len() == 48, MaterialError::Structure);
            Ok(Signer::P384(p384::ecdsa::SigningKey::from(
                p384::SecretKey::from_slice(bytes).map_err(|_| MaterialError::Structure)?,
            )))
        }
        "p256-seed" => {
            ensure!(
                matches!(derivation, Some(LEGACY_P256_DERIVATION) | Some("auto")),
                MaterialError::Unsupported
            );
            let seed = fixed_seed(bytes)?;
            let scalar = <NonZeroScalar as Reduce<FieldBytes>>::reduce(&FieldBytes::from(*seed));
            let secret: p256::SecretKey = scalar.into();
            Ok(Signer::P256(p256::ecdsa::SigningKey::from(secret)))
        }
        "cdi-attest" => {
            ensure!(
                matches!(derivation, Some(OPEN_DICE_DERIVATION) | Some("auto")),
                MaterialError::Unsupported
            );
            let cdi = fixed_seed(bytes)?;
            let seed = open_dice_seed(&cdi)?;
            Ok(Signer::Ed25519(Ed25519Key::from_bytes(&seed)))
        }
        _ => bail!(MaterialError::Unsupported),
    }
}

fn fixed_seed(bytes: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    Ok(Zeroizing::new(
        bytes.try_into().map_err(|_| MaterialError::Structure)?,
    ))
}

// Open Profile for DICE v2.5 ASYM_KDF: HKDF-SHA512 with ASYM_SALT and Key Pair info
fn open_dice_seed(cdi: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>> {
    let salt = hex::decode(concat!(
        "63B6A04D2C077FC10F639F21DA793844356CC2B0B441B3A77124035C03F8E1BE",
        "6035D31F282821A7450A02222AB1B3CFF1679B05AB1CA5D1AFFB789CCD2B0B3B"
    ))?;
    let kdf = Hkdf::<Sha512>::new(Some(&salt), cdi);
    let mut seed = Zeroizing::new([0_u8; 32]);
    kdf.expand(b"Key Pair", seed.as_mut())
        .map_err(|_| anyhow::anyhow!("derive Open DICE key pair seed"))?;
    Ok(seed)
}
