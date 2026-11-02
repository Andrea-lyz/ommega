//! COSE/DICE structures for the RKP `AuthenticatedRequest` (CSR v3).
//!
//! Label values come from RFC 9052/9053 and the Open DICE profile referenced by
//! `hardware/interfaces/security/rkp/aidl/.../generateCertificateRequestV2.cddl`.

use anyhow::{Result, anyhow};

mod csr;
pub(crate) mod imported;
mod keys;
mod protected;

pub use csr::build_experimental_csr_v3;
pub use csr::{CsrBundle, build_csr, build_sig_structure, device_info_to_cbor};
pub use keys::{DeviceKeys, EcKeyPair, generate_ec_keypair};
pub use protected::{cose_mac0, key_mac_tag};

pub const ALG_EDDSA: i128 = -8;
pub const ALG_ES256: i128 = -7;
pub const ALG_A256GCM: i128 = 3;
pub const ALG_HMAC_256: i128 = 5;
pub const ALG_ECDH_ES_HKDF_256: i128 = -25;
pub const CWT_ISSUER: i128 = 1;
pub const CWT_SUBJECT: i128 = 2;
pub const DICE_SUBJECT_PUB_KEY: i128 = -4_670_552;
pub const DICE_KEY_USAGE: i128 = -4_670_553;
pub const RPC_CURVE_P256: i128 = 1;
pub const RPC_CURVE_25519: i128 = 2;

pub(crate) fn fill_random(bytes: &mut [u8]) -> Result<()> {
    getrandom::fill(bytes).map_err(|error| anyhow!("fill random bytes from OS RNG: {error}"))
}

pub(crate) fn random_bytes_array<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0_u8; N];
    fill_random(&mut bytes)?;
    Ok(bytes)
}
