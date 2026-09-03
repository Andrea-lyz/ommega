//! Client for Android's system Remote Key Provisioning service.

use crate::android::hardware::security::keymint::{
    AttestationKey::AttestationKey, SecurityLevel::SecurityLevel,
};
use crate::android::security::rkp::{
    IGetKeyCallback::{BnGetKeyCallback, ErrorCode::ErrorCode as GetKeyErrorCode, IGetKeyCallback},
    IGetRegistrationCallback::{BnGetRegistrationCallback, IGetRegistrationCallback},
    IRegistration::IRegistration,
    IRemoteProvisioning::IRemoteProvisioning,
    IStoreUpgradedKeyCallback::{BnStoreUpgradedKeyCallback, IStoreUpgradedKeyCallback},
    RemotelyProvisionedKey::RemotelyProvisionedKey,
};
use crate::keymaster::crypto::parse_subject_from_certificate;
use crate::keymaster::error::{Error as KsError, ResponseCode};
use crate::keymaster::utils::get_interface_once;
use anyhow::{Context, Result};
use rsbinder::{BinderFeatures, Interface, Strong};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

const REMOTE_PROVISIONING_SERVICE: &str = "remote_provisioning";
const RKP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
enum RkpError {
    #[error("request cancelled")]
    Cancelled,
    #[error("failed to get registration: {0}")]
    Registration(String),
    #[error("failed to get key ({code}): {description}")]
    GetKey { code: i8, description: String },
    #[error("failed to store upgraded key: {0}")]
    StoreUpgradedKey(String),
    #[error("timed out waiting for Remote Key Provisioning")]
    Timeout,
}

impl RkpError {
    fn keystore_error(&self) -> KsError {
        let response = match self {
            Self::GetKey { code, .. }
                if *code == GetKeyErrorCode::ERROR_REQUIRES_SECURITY_PATCH.0 =>
            {
                ResponseCode::OUT_OF_KEYS_REQUIRES_SYSTEM_UPGRADE
            }
            Self::GetKey { code, .. }
                if *code == GetKeyErrorCode::ERROR_PENDING_INTERNET_CONNECTIVITY.0 =>
            {
                ResponseCode::OUT_OF_KEYS_PENDING_INTERNET_CONNECTIVITY
            }
            Self::GetKey { code, .. } if *code == GetKeyErrorCode::ERROR_PERMANENT.0 => {
                ResponseCode::OUT_OF_KEYS_PERMANENT_ERROR
            }
            _ => ResponseCode::OUT_OF_KEYS_TRANSIENT_ERROR,
        };
        KsError::Rc(response)
    }
}

struct SafeSender<T> {
    inner: Mutex<Option<mpsc::SyncSender<std::result::Result<T, RkpError>>>>,
}

impl<T> SafeSender<T> {
    fn new(sender: mpsc::SyncSender<std::result::Result<T, RkpError>>) -> Self {
        Self {
            inner: Mutex::new(Some(sender)),
        }
    }

    fn send(&self, value: std::result::Result<T, RkpError>) {
        if let Some(sender) = self.inner.lock().unwrap().take() {
            if sender.send(value).is_err() {
                log::warn!("RKP callback arrived after its receiver was dropped");
            }
        }
    }
}

struct GetRegistrationCallback {
    sender: SafeSender<Strong<dyn IRegistration>>,
}

impl Interface for GetRegistrationCallback {}

impl IGetRegistrationCallback for GetRegistrationCallback {
    fn onSuccess(&self, registration: &Strong<dyn IRegistration>) -> rsbinder::BinderResult<()> {
        self.sender.send(Ok(registration.clone()));
        Ok(())
    }

    fn onCancel(&self) -> rsbinder::BinderResult<()> {
        self.sender.send(Err(RkpError::Cancelled));
        Ok(())
    }

    fn onError(&self, error: &str) -> rsbinder::BinderResult<()> {
        self.sender
            .send(Err(RkpError::Registration(error.to_string())));
        Ok(())
    }
}

struct GetKeyCallback {
    sender: SafeSender<RemotelyProvisionedKey>,
}

impl Interface for GetKeyCallback {}

impl IGetKeyCallback for GetKeyCallback {
    fn onSuccess(&self, key: &RemotelyProvisionedKey) -> rsbinder::BinderResult<()> {
        self.sender.send(Ok(RemotelyProvisionedKey {
            keyBlob: key.keyBlob.clone(),
            encodedCertChain: key.encodedCertChain.clone(),
        }));
        Ok(())
    }

    fn onCancel(&self) -> rsbinder::BinderResult<()> {
        self.sender.send(Err(RkpError::Cancelled));
        Ok(())
    }

    fn onError(&self, error: GetKeyErrorCode, description: &str) -> rsbinder::BinderResult<()> {
        self.sender.send(Err(RkpError::GetKey {
            code: error.0,
            description: description.to_string(),
        }));
        Ok(())
    }
}

struct StoreUpgradedKeyCallback {
    sender: SafeSender<()>,
}

impl Interface for StoreUpgradedKeyCallback {}

impl IStoreUpgradedKeyCallback for StoreUpgradedKeyCallback {
    fn onSuccess(&self) -> rsbinder::BinderResult<()> {
        self.sender.send(Ok(()));
        Ok(())
    }

    fn onError(&self, error: &str) -> rsbinder::BinderResult<()> {
        self.sender
            .send(Err(RkpError::StoreUpgradedKey(error.to_string())));
        Ok(())
    }
}

fn wait_for_callback<T>(
    receiver: mpsc::Receiver<std::result::Result<T, RkpError>>,
) -> std::result::Result<T, RkpError> {
    receiver
        .recv_timeout(RKP_TIMEOUT)
        .map_err(|_| RkpError::Timeout)?
}

fn get_registration(rpc_name: &str) -> Result<Strong<dyn IRegistration>> {
    let service: Strong<dyn IRemoteProvisioning> = get_interface_once(REMOTE_PROVISIONING_SERVICE)
        .map_err(KsError::BinderTransaction)
        .context("connecting to Android Remote Key Provisioning service")?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let callback = BnGetRegistrationCallback::new_binder_with_features(
        GetRegistrationCallback {
            sender: SafeSender::new(sender),
        },
        BinderFeatures::default(),
    );
    service
        .getRegistration(rpc_name, &callback)
        .map_err(|status| anyhow::anyhow!("getRegistration binder call failed: {status}"))?;
    wait_for_callback(receiver)
        .map_err(|error| anyhow::Error::new(error.keystore_error()).context(error.to_string()))
}

fn get_key(rpc_name: &str, caller_uid: u32) -> Result<RemotelyProvisionedKey> {
    let registration = get_registration(rpc_name)?;
    let key_id = i32::try_from(caller_uid).context("RKP key id does not fit i32")?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let callback = BnGetKeyCallback::new_binder_with_features(
        GetKeyCallback {
            sender: SafeSender::new(sender),
        },
        BinderFeatures::default(),
    );
    registration
        .getKey(key_id, &callback)
        .map_err(|status| anyhow::anyhow!("getKey binder call failed: {status}"))?;
    match wait_for_callback(receiver) {
        Ok(key) => Ok(key),
        Err(error) => {
            if matches!(error, RkpError::Timeout) {
                if let Err(status) = registration.cancelGetKey(&callback) {
                    log::warn!("failed to cancel timed-out RKP request: {status}");
                }
            }
            Err(anyhow::Error::new(error.keystore_error()).context(error.to_string()))
        }
    }
}

fn first_der_object(encoded: &[u8]) -> Result<&[u8]> {
    if encoded.len() < 2 || encoded[0] != 0x30 {
        anyhow::bail!("RKP certificate chain does not start with a DER SEQUENCE");
    }
    let length_octet = encoded[1];
    let (header_len, body_len) = if length_octet & 0x80 == 0 {
        (2usize, usize::from(length_octet))
    } else {
        let octets = usize::from(length_octet & 0x7f);
        if octets == 0 || octets > std::mem::size_of::<usize>() || encoded.len() < 2 + octets {
            anyhow::bail!("invalid DER length in RKP certificate chain");
        }
        let mut length = 0usize;
        for octet in &encoded[2..2 + octets] {
            length = length
                .checked_mul(256)
                .and_then(|value| value.checked_add(usize::from(*octet)))
                .context("DER length overflow in RKP certificate chain")?;
        }
        (2 + octets, length)
    };
    let total_len = header_len
        .checked_add(body_len)
        .context("DER object length overflow")?;
    encoded
        .get(..total_len)
        .context("truncated first certificate in RKP chain")
}

fn rpc_name(security_level: SecurityLevel) -> Result<&'static str> {
    match security_level {
        SecurityLevel::TRUSTED_ENVIRONMENT => {
            Ok("android.hardware.security.keymint.IRemotelyProvisionedComponent/default")
        }
        SecurityLevel::STRONGBOX => {
            Ok("android.hardware.security.keymint.IRemotelyProvisionedComponent/strongbox")
        }
        _ => anyhow::bail!("RKP is unsupported for security level {security_level:?}"),
    }
}

pub struct RkpAttestationKey {
    pub attestation_key: AttestationKey,
    pub certificate_chain: Vec<u8>,
    rpc_name: &'static str,
}

pub fn get_attestation_key(
    security_level: SecurityLevel,
    caller_uid: u32,
) -> Result<RkpAttestationKey> {
    let rpc_name = rpc_name(security_level)?;
    let provisioned = get_key(rpc_name, caller_uid)
        .with_context(|| format!("fetching {rpc_name} attestation key from Android RKP"))?;
    if provisioned.keyBlob.is_empty() || provisioned.encodedCertChain.is_empty() {
        anyhow::bail!("Android RKP returned an empty key blob or certificate chain");
    }
    let first_certificate = first_der_object(&provisioned.encodedCertChain)?;
    let issuer_subject = parse_subject_from_certificate(first_certificate)
        .context("parsing subject of RKP batch certificate")?;
    log::info!("obtained native {security_level:?} RKP attestation key for uid={caller_uid}");
    Ok(RkpAttestationKey {
        attestation_key: AttestationKey {
            keyBlob: provisioned.keyBlob,
            attestKeyParams: vec![],
            issuerSubjectName: issuer_subject,
        },
        certificate_chain: provisioned.encodedCertChain,
        rpc_name,
    })
}

pub fn store_upgraded_attestation_key(
    key: &RkpAttestationKey,
    old_blob: &[u8],
    upgraded_blob: &[u8],
) -> Result<()> {
    let registration = get_registration(key.rpc_name)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let callback = BnStoreUpgradedKeyCallback::new_binder_with_features(
        StoreUpgradedKeyCallback {
            sender: SafeSender::new(sender),
        },
        BinderFeatures::default(),
    );
    registration
        .storeUpgradedKeyAsync(old_blob, upgraded_blob, &callback)
        .map_err(|status| anyhow::anyhow!("storeUpgradedKeyAsync binder call failed: {status}"))?;
    wait_for_callback(receiver)
        .map_err(|error| anyhow::Error::new(error.keystore_error()).context(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_der_object_slices_concatenated_chain() {
        let chain = [0x30, 0x03, 1, 2, 3, 0x30, 0x01, 4];
        assert_eq!(first_der_object(&chain).unwrap(), &[0x30, 0x03, 1, 2, 3]);
    }

    #[test]
    fn first_der_object_supports_long_form_lengths() {
        let mut chain = vec![0x30, 0x81, 0x80];
        chain.extend([0u8; 128]);
        chain.extend([0x30, 0]);
        assert_eq!(first_der_object(&chain).unwrap().len(), 131);
    }

    #[test]
    fn rpc_name_matches_android_instances() {
        assert_eq!(
            rpc_name(SecurityLevel::TRUSTED_ENVIRONMENT).unwrap(),
            "android.hardware.security.keymint.IRemotelyProvisionedComponent/default"
        );
        assert_eq!(
            rpc_name(SecurityLevel::STRONGBOX).unwrap(),
            "android.hardware.security.keymint.IRemotelyProvisionedComponent/strongbox"
        );
    }
}
