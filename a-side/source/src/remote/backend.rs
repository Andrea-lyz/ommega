// Copyright 2026, The Android Open Source Project
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Adapts relay responses to KeyMint operations.

use base64::Engine as _;
use serde_json::Value;
use x509_cert::der::Decode as _;

use super::{
    parse_identity_profile, reject_strongbox_demotion, remote_enabled, remote_identity_profile,
    remote_operation_error, RemoteRelay,
};
use crate::config;

/// Adapts [`RemoteRelay`] to the TA's [`kmr_ta::device::RemoteBackend`] trait.
pub struct RemoteRelayBackend;

impl kmr_ta::device::RemoteBackend for RemoteRelayBackend {
    fn attest(
        &self,
        challenge: &[u8],
        app_id_der: &[u8],
        alias: &str,
        cert_serial: Option<&[u8]>,
        params: &kmr_ta::device::RemoteAttestParams,
    ) -> Result<Option<kmr_ta::device::RemoteAttestation>, kmr_common::Error> {
        let profile = match remote_identity_profile() {
            Ok(profile) => profile,
            Err(error)
                if config::config()
                    .read()
                    .is_ok_and(|config| config.remote.fallback_local) =>
            {
                log::warn!(
                    "remote identity profile unavailable with local fallback enabled: {error:#}"
                );
                return Ok(None);
            }
            Err(error) => {
                return Err(kmr_common::km_err!(
                    UnknownError,
                    "remote identity profile unavailable: {error:#}"
                ));
            }
        };
        if params.security_level == Some(kmr_wire::keymint::SecurityLevel::Strongbox as i32)
            && !profile.has_strongbox
        {
            return Err(kmr_common::km_err!(
                HardwareTypeUnavailable,
                "remote identity does not provide StrongBox"
            ));
        }
        let resp = RemoteRelay::attest(challenge, alias, app_id_der, params, cert_serial)
            .map_err(|e| remote_operation_error(e, "attest"))?;
        let Some(resp) = resp else {
            log::warn!("remote relay unavailable after transport retry");
            return Ok(None);
        };
        // The relay wraps the result as `{ result: { cert_chain: [...] } }`.
        let result = resp.get("result").unwrap_or(&resp);
        reject_strongbox_demotion(result, params.security_level)?;
        if let Some(result_profile) = result.get("remote_profile") {
            let observed = parse_identity_profile(result_profile).map_err(|error| {
                kmr_common::km_err!(
                    VerificationFailed,
                    "remote attest response has an invalid identity profile: {error:#}"
                )
            })?;
            if observed != profile {
                return Err(kmr_common::km_err!(
                    VerificationFailed,
                    "remote identity profile changed while minting"
                ));
            }
        }
        let certs = result
            .get("cert_chain")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                kmr_common::km_err!(UnknownError, "remote attest response missing cert_chain")
            })?;
        if certs.is_empty() {
            return Err(kmr_common::km_err!(
                UnknownError,
                "remote attest returned empty cert chain"
            ));
        }
        let debug_logging = config::config()
            .read()
            .is_ok_and(|config| config.remote.debug_logging);
        let mut layout = Vec::new();
        let mut chain = Vec::with_capacity(certs.len());
        for (index, cert) in certs.iter().enumerate() {
            let encoded = cert.as_str().ok_or_else(|| {
                kmr_common::km_err!(UnknownError, "remote cert_chain[{index}] is not a string")
            })?;
            let der = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|e| {
                    kmr_common::km_err!(
                        UnknownError,
                        "remote cert_chain[{index}] has invalid base64: {e}"
                    )
                })?;
            let certificate = x509_cert::Certificate::from_der(&der).map_err(|e| {
                kmr_common::km_err!(
                    UnknownError,
                    "remote cert_chain[{index}] is not a DER certificate: {e:?}"
                )
            })?;
            if debug_logging {
                layout.push(format!(
                    "[{index}] len={} sig={} spki={}",
                    der.len(),
                    certificate.signature_algorithm().oid,
                    certificate
                        .tbs_certificate()
                        .subject_public_key_info()
                        .algorithm
                        .oid
                ));
            }
            chain.push(der);
        }
        if debug_logging {
            let tail_bytes: usize = chain.iter().skip(1).map(|cert| cert.len()).sum();
            let layout = layout.join(" ");
            log::info!(
                "event=remote_attest_chain alias={alias} key_algorithm={:?} certs={} tail_bytes={} {layout}",
                params.key_algorithm,
                chain.len(),
                tail_bytes
            );
        }
        Ok(Some(kmr_ta::device::RemoteAttestation {
            cert_chain: chain,
            // The relay may only confirm the level that was requested; its
            // downgrade marker is rejected above.
            effective_security_level: None,
            profile: kmr_ta::device::RemoteKeyMintProfile {
                interface_version: profile.interface_version,
                interface_hash: profile.interface_hash,
                profile_version: profile.profile_version,
                hardware_version: profile.hardware_version,
                security_level: kmr_wire::keymint::SecurityLevel::TrustedEnvironment,
                has_strongbox: profile.has_strongbox,
                max_operations: profile.max_operations,
            },
        }))
    }

    fn sign(
        &self,
        alias: &str,
        data: &[u8],
        algorithm: &str,
    ) -> Result<Option<Vec<u8>>, kmr_common::Error> {
        let Some(resp) = RemoteRelay::sign(alias, data, algorithm)
            .map_err(|e| remote_operation_error(e, "sign"))?
        else {
            return Ok(None);
        };
        let result = resp.get("result").unwrap_or(&resp);
        let sig_b64 = result
            .get("signature")
            .or_else(|| result.get("data"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                kmr_common::km_err!(UnknownError, "remote sign response missing signature")
            })?;
        let sig = base64::engine::general_purpose::STANDARD
            .decode(sig_b64)
            .map_err(|e| kmr_common::km_err!(UnknownError, "bad remote signature base64: {e}"))?;
        Ok(Some(sig))
    }

    fn decrypt(
        &self,
        alias: &str,
        data: &[u8],
        algorithm: &str,
    ) -> Result<Option<Vec<u8>>, kmr_common::Error> {
        let Some(resp) = RemoteRelay::decrypt(alias, data, algorithm)
            .map_err(|e| remote_operation_error(e, "decrypt"))?
        else {
            return Ok(None);
        };
        let result = resp.get("result").unwrap_or(&resp);
        let plain_b64 = result.get("data").and_then(Value::as_str).ok_or_else(|| {
            kmr_common::km_err!(UnknownError, "remote decrypt response missing data")
        })?;
        let plain = base64::engine::general_purpose::STANDARD
            .decode(plain_b64)
            .map_err(|e| kmr_common::km_err!(UnknownError, "bad remote decrypt base64: {e}"))?;
        Ok(Some(plain))
    }

    fn agree_key(
        &self,
        alias: &str,
        peer_public_key: &[u8],
    ) -> Result<Option<Vec<u8>>, kmr_common::Error> {
        let Some(resp) = RemoteRelay::agree_key(alias, peer_public_key)
            .map_err(|e| remote_operation_error(e, "agree"))?
        else {
            return Ok(None);
        };
        let result = resp.get("result").unwrap_or(&resp);
        let secret_b64 = result.get("data").and_then(Value::as_str).ok_or_else(|| {
            kmr_common::km_err!(UnknownError, "remote key-agreement response missing data")
        })?;
        let secret = base64::engine::general_purpose::STANDARD
            .decode(secret_b64)
            .map_err(|e| {
                kmr_common::km_err!(UnknownError, "bad remote key-agreement base64: {e}")
            })?;
        Ok(Some(secret))
    }

    fn enabled(&self) -> bool {
        remote_enabled()
    }

    fn fallback_local(&self) -> bool {
        match config::config().read() {
            Ok(g) => g.remote.fallback_local,
            Err(_) => true,
        }
    }
}
