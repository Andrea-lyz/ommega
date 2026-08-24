//! Real hardware TEE attestation helpers for ommegaclient-b.
//!
//! Exposes the real KeyMint HAL service names and an `AttestationApplicationId`
//! sanity-check. Driving the real hardware keymint (`generateKey()` with the
//! A-side's requested `ATTESTATION_APPLICATION_ID` tag 709 and challenge) is
//! done by `tee_ops`, which connects via `relay_tee` and mints the chain signed
//! by the real device hardware keybox.

use anyhow::{anyhow, Result};

/// The real TEE KeyMint HAL service name.
pub(crate) const SYSTEM_KEYMINT_DEFAULT: &str =
    "android.hardware.security.keymint.IKeyMintDevice/default";

/// The real StrongBox KeyMint HAL service name. Only used when the A-side
/// requests a StrongBox attestation and this B-side device actually exposes a
/// StrongBox HAL; otherwise the A-side falls back to its local software keybox.
pub const SYSTEM_KEYMINT_STRONGBOX: &str =
    "android.hardware.security.keymint.IKeyMintDevice/strongbox";

/// Small helper used by tests / callers to sanity-check that `app_id_der`
/// really is an `AttestationApplicationId` before handing it to the TEE.
pub fn check_app_id_der(app_id_der: &[u8]) -> Result<()> {
    use der::Decode;
    crate::plat::aaid::AttestationApplicationId::from_der(app_id_der)
        .map(|_| ())
        .map_err(|e| anyhow!("app_id_der is not a valid AttestationApplicationId: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plat::aaid::AttestationApplicationId as DerAAID;
    use crate::plat::aaid::PackageInfoRecord;
    use der::asn1::{OctetString, SetOfVec};
    use der::Encode;

    #[test]
    fn app_id_der_roundtrip() {
        let aaid = DerAAID {
            package_info_records: SetOfVec::from_iter(vec![PackageInfoRecord {
                package_name: OctetString::new(b"z.example".to_vec()).unwrap(),
                version: 2,
            }])
            .unwrap(),
            signature_digests: SetOfVec::from_iter(Vec::<OctetString>::new()).unwrap(),
        };
        let der = aaid.to_der().expect("encode");
        assert!(check_app_id_der(&der).is_ok());
        let bad = vec![0u8, 1, 2, 3];
        assert!(check_app_id_der(&bad).is_err());
    }
}
