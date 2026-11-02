//! Remote Key Provisioning (RKP) engine.
//!
//! Builds AuthenticatedRequest CSRs v3 as specified by the AOSP
//! IRemotelyProvisionedComponent HAL, talks to the RKP server and exports the
//! returned certificate chain as a Tricky Store style keybox.
//!
//! Ported from Duck ToolBox `crates/duck-rkp` (MIT, Copyright (c) 2026 Eltavine).
//! The host CLI, the saved-profile layer and the device collection paths are not
//! part of this port: only the imported-material flow is kept, because issuing
//! against material the device did not collect is the only path this module needs.

mod args;
mod artifacts;
pub mod cbor;
pub mod certificates;
pub mod cose;
pub mod crypto_kdf;
pub mod error;
mod handlers;
pub mod http;
pub mod keybox_xml;
pub mod material;
pub mod patch_level;
pub mod paths;
pub mod private_output;
pub mod profile;
pub mod validate;
pub mod verify;
mod workflow;

pub use args::ImportArgs;
pub use handlers::import::{diagnose_bundle, issue_from_bundle, validate_bundle};
