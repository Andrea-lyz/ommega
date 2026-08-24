//! Small shared helpers used across modules.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Lock a `std::sync::Mutex` and recover from a poisoned state.
/// If another thread panicked while holding the lock, the guard is still valid
/// (the data may be in an inconsistent state, but in practice the in-memory
/// maps/sets used here are simple enough that this is safe to continue).
pub fn mu<'a, T>(m: &'a Mutex<T>) -> MutexGuard<'a, T> {
    match m.lock() {
        Ok(g) => g,
        Err(PoisonError { .. }) => m.lock().unwrap_or_else(|e| e.into_inner()),
    }
}

/// Generate a URL-safe, 24-byte random token (32 chars, matches Django's
/// `secrets.token_urlsafe(24)`).
pub fn generate_token_string() -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
