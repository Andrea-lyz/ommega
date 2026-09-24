//! Software Soter TA for the A-side device.
//!
//! The vendor AIDL HAL `vendor.qti.hardware.soter.ISoter` is a thin proxy over a
//! Qualcomm TA that lives in the secure world. On a device whose secure world is
//! unusable (unlocked bootloader with a dead TrustZone applet) every Soter call
//! fails with `-20`, which applications that use Soter as a local device-integrity
//! probe read as "this device is up to something".
//!
//! This crate implements the HAL state machine with purely local material: its own
//! RSA keys, its own device id, its own anti-rollback counters and sign sessions.
//! Applications that go through the Java `SoterService` then see a healthy,
//! self-consistent Soter.
//!
//! Scope: local checks only. The ASK blob's own signature is produced by the
//! device ATTK inside the secure world; that signature cannot be reproduced here,
//! so material minted by this crate is rejected by any server that checks it.
pub mod blob;
pub mod dispatch;
pub mod error;
pub mod parcel;
pub mod state;

pub use error::{SOTER_ERR_BAD_VALUE, SOTER_ERR_NO_KEY, SOTER_ERR_TA_UNAVAILABLE, SOTER_OK};
pub use state::TaState;

#[cfg(test)]
mod tests;

