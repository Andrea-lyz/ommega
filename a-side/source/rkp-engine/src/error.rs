use thiserror::Error;

#[derive(Debug, Error)]
pub enum RkpError {
    #[error(
        "RKP endpoint must use HTTPS (HTTP is allowed only for loopback tests), without credentials, query or fragment"
    )]
    InvalidEndpoint,
    #[error("server challenge unavailable; no certificate request was submitted")]
    ChallengeUnavailable,
    #[error("certificate submission result is unknown; do not retry automatically")]
    SubmissionUnknown,
    #[error("returned certificates failed validation; no keybox was exported: {0}")]
    CertificateValidation(String),
    #[error("unsupported profile name `{0}`; only `default` is available")]
    UnsupportedProfile(String),
    #[error("key material is not configured; save a profile or pass `--seed` / `--hw-key`")]
    MissingKeySource,
    #[error("build fingerprint is required before provisioning")]
    MissingFingerprint,
    #[error("RKP server URL is required")]
    MissingServerUrl,
    #[error("invalid RKP server URL `{0}`")]
    InvalidServerUrl(String),
    #[error("output path `{0}` must stay relative to Duck ToolBox and include a file name")]
    InvalidOutputPath(String),
    #[error("`--kdf-label` is required when using `--hw-key`")]
    MissingKdfLabel,
    #[error("seed must be exactly 32 bytes, got {0}")]
    InvalidSeedLength(usize),
    #[error("hardware key must be exactly 16 bytes, got {0}")]
    InvalidHardwareKeyLength(usize),
    #[error("device field `{0}` is required before provisioning")]
    MissingDeviceField(&'static str),
    #[error("device field `{field}` is invalid: {reason}")]
    InvalidDeviceField { field: &'static str, reason: String },
    #[error("RKP server returned unsupported EEK curve `{0}`")]
    UnsupportedEekCurve(i128),
    #[error("invalid RKP server response: {0}")]
    InvalidRkpResponse(String),
    #[error("device not registered: {0}")]
    DeviceNotRegistered(String),
    #[error("RKP client error: {0}")]
    RkpClient(String),
    #[error("RKP server error: {0}")]
    RkpServer(String),
}

impl RkpError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidEndpoint => "invalid_endpoint",
            Self::ChallengeUnavailable => "challenge_unavailable",
            Self::SubmissionUnknown => "submission_unknown",
            Self::CertificateValidation(_) => "certificate_validation_failed",
            Self::UnsupportedProfile(_) => "unsupported_profile",
            Self::MissingKeySource => "missing_key_source",
            Self::MissingFingerprint => "missing_fingerprint",
            Self::MissingServerUrl => "missing_server_url",
            Self::InvalidServerUrl(_) => "invalid_server_url",
            Self::InvalidOutputPath(_) => "invalid_output_path",
            Self::MissingKdfLabel => "missing_kdf_label",
            Self::InvalidSeedLength(_) => "invalid_seed_length",
            Self::InvalidHardwareKeyLength(_) => "invalid_hardware_key_length",
            Self::MissingDeviceField(_) => "missing_device_field",
            Self::InvalidDeviceField { .. } => "invalid_device_field",
            Self::UnsupportedEekCurve(_) => "unsupported_eek_curve",
            Self::InvalidRkpResponse(_) => "invalid_rkp_response",
            Self::DeviceNotRegistered(_) => "device_not_registered",
            Self::RkpClient(_) => "rkp_client_error",
            Self::RkpServer(_) => "rkp_server_error",
        }
    }
}

/// Maps any error raised by this crate to its stable WebUI error code.
pub fn code_of(error: &anyhow::Error) -> &'static str {
    if let Some(code) = crate::material::error_code(error) {
        return code;
    }
    error
        .downcast_ref::<RkpError>()
        .or_else(|| {
            error
                .chain()
                .find_map(|cause| cause.downcast_ref::<RkpError>())
        })
        .map_or("internal_error", RkpError::code)
}
