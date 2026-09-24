//! `SoterErrorCode` values as they appear on the wire.
//!
//! `SoterErrorCode` is a plain `int32` in the HAL AIDL; the stock HAL forwards
//! whatever the TA/QSEE layer reported. The values below were read off the live
//! HAL of a OnePlus 13 whose TA is dead.

/// Success. The Java layer treats `rc == 0` as "yes/ok".
pub const SOTER_OK: i32 = 0;

/// No such key.
///
/// The stock HAL answers `-5` (`0xfffffffb`) to `hasAskAlready` for a uid that
/// has no ASK, and that is what an empty local ledger should answer too.
pub const SOTER_ERR_NO_KEY: i32 = -5;

/// Raw `rsp->status` of a TEE command that could not be delivered.
///
/// This is the value every Soter call fails with while the TA is unavailable,
/// and it is also the honest answer when local key generation or signing fails.
pub const SOTER_ERR_TA_UNAVAILABLE: i32 = -20;

/// No such sign session.
///
/// The stock HAL answers `-1000` (`0xfffffc18`) to `finishSign` for a session it
/// does not know, captured on the OnePlus 13 on 2026-09-24 while its TA was dead.
pub const SOTER_ERR_NO_SESSION: i32 = -1000;

/// Malformed request, unknown alias/session and similar argument errors.
pub const SOTER_ERR_BAD_VALUE: i32 = -22;
