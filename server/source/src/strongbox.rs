//! StrongBox robustness mode switch (runtime, in-memory).
//!
//! When enabled, a B-side StrongBox capability error (StrongBox not supported /
//! attestation keys not provisioned / HAL not present) is transparently retried
//! as a TEE request on the same B device — the Android-standard silent fallback
//! behaviour. The downgraded chain is tagged TRUSTED_ENVIRONMENT by the B side,
//! so this is an honest degradation, never a mislabelled StrongBox.
//!
//! When disabled (default), StrongBox errors propagate to the next fulfilment
//! layer exactly as before (strict native semantics: server three-layer
//! fallback → A-side local keybox).

use std::sync::atomic::{AtomicBool, Ordering};

static ROBUST_MODE: AtomicBool = AtomicBool::new(false);

/// Whether StrongBox robustness mode is currently enabled.
pub fn is_robust() -> bool {
    ROBUST_MODE.load(Ordering::Relaxed)
}

/// Enable/disable StrongBox robustness mode.
pub fn set_robust(v: bool) {
    ROBUST_MODE.store(v, Ordering::Relaxed);
}
