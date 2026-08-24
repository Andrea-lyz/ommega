//! ommegaclient-b library target.
//!
//! This crate is the **new B-side** relay agent: it receives tasks from the
//! relay_server (ommega-old), calls the *real* on-device hardware TEE to mint
//! attestation certificate chains that embed a caller-supplied application id
//! (tag 709), and forwards the result back.
//!
//! Only the modules needed for that forwarding path are kept here; the
//! software keystore body has been removed.

#![recursion_limit = "256"]

pub mod keymaster;
pub mod logging;
pub mod macros;
pub mod plat;

include!(concat!(env!("OUT_DIR"), "/aidl.rs"));
