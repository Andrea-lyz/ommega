//! Inputs for the import path, replacing the clap definitions of the original CLI.

#[derive(Debug, Clone, Default)]
pub struct ImportArgs {
    /// versioned JSON material bundle
    pub bundle: Option<String>,
    /// raw CBOR DICE chain (requires signing)
    pub chain: Option<String>,
    /// signing description for a raw chain
    pub signing: Option<String>,
    /// optional device/issuance configuration
    pub config: Option<String>,
}
