use thiserror::Error;

#[derive(Debug, Error)]
pub enum MaterialError {
    #[error("invalid imported material structure")]
    Structure,
    #[error("imported DICE chain or UDS evidence is invalid: {0}")]
    InvalidChain(String),
    #[error("unsupported signing material type or derivation")]
    Unsupported,
    #[error("signing material does not match the DICE leaf public key")]
    KeyMismatch,
    #[error("multiple supported signing material derivations match the DICE leaf")]
    AmbiguousMatch,
    #[error("invalid or conflicting imported issuance configuration")]
    Configuration,
}

pub fn error_code(error: &anyhow::Error) -> Option<&'static str> {
    error
        .downcast_ref::<MaterialError>()
        .or_else(|| {
            error
                .chain()
                .find_map(|cause| cause.downcast_ref::<MaterialError>())
        })
        .map(|error| match error {
            MaterialError::Structure => "material_structure",
            MaterialError::InvalidChain(_) => "material_chain_invalid",
            MaterialError::Unsupported => "material_unsupported",
            MaterialError::KeyMismatch => "material_key_mismatch",
            MaterialError::AmbiguousMatch => "material_ambiguous_match",
            MaterialError::Configuration => "material_configuration",
        })
}
