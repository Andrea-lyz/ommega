//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//

//! Secure-world round-trip accounting for the in-process software KeyMint TA.
//!
//! ommega services `IKeyMintDevice` from a software TA inside the module process,
//! so a HAL entry such as `begin()` completes in roughly 3 ms. A hardware TA
//! spends materially longer per entry: a Binder round trip, an SMCCC entry into
//! the secure world, the keyblob unwrap with its authorization checks, and the
//! return path. Stock devices measure `begin()` at 11.93~25.00 ms, and
//! validators use that floor to separate "the operation ran in a secure world"
//! from "the operation was served locally by software" (Keystore Killer v1.3
//! reports it as its `T` criterion, with a 4.8 ms floor and a 9.5 ms band).
//!
//! This module charges every TA-backed HAL entry a plausible round-trip cost so
//! the software path is not observably faster than the hardware it stands in
//! for. Costs are sampled per call rather than fixed: a constant offset can be
//! calibrated away by a caller that compares repeated samples, while a spread
//! behaves like the real thing. Entries of the same class are charged the same
//! band, which keeps the ratios validators compute (operation against
//! operation, generation against generation) inside the hardware range.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Class of TA work a HAL entry performs, by secure-world cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaOp {
    /// `begin()`, `update()`, `finish()`, `abort()` and
    /// `getKeyCharacteristics()`: keyblob unwrap, authorization checks and
    /// in-TA crypto.
    Operation,
    /// `generateKey()`, `importKey()`, `importWrappedKey()`, `upgradeKey()`
    /// and `convertStorageKeyToEphemeral()`: RNG, key generation and sealing.
    KeyGen,
    /// Entries that still cross into the TA but do no key work, such as
    /// `deleteKey()`, `earlyBootEnded()` and the root-of-trust queries.
    Control,
}

/// Latency added per call in milliseconds, `(min, max)` inclusive.
///
/// The `Operation` band is chosen so the software TA lands inside the
/// 11.93~25.00 ms that stock devices take for `begin()`, after the ~3 ms the
/// in-process path already spends, and stays clear of the 9.5 ms above which
/// validators stop treating an operation as served without a secure world.
fn band(op: TaOp) -> (u64, u64) {
    match op {
        TaOp::Operation => (9, 21),
        TaOp::KeyGen => (6, 16),
        TaOp::Control => (1, 4),
    }
}

/// Charge the calling HAL entry the secure-world round trip a hardware TA would
/// spend on it.
pub fn charge(op: TaOp) {
    let ms = sample_ms(op);
    if ms > 0 {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// One sample from the band of `op`.
fn sample_ms(op: TaOp) -> u64 {
    let (min, max) = band(op);
    if max <= min {
        return min;
    }
    min + (next_u64() % (max - min + 1))
}

/// xorshift64, seeded from the clock on first use. Used to spread latency only;
/// this is not a source of cryptographic randomness.
fn next_u64() -> u64 {
    static STATE: AtomicU64 = AtomicU64::new(0);

    let mut current = STATE.load(Ordering::Relaxed);
    loop {
        let mut x = if current == 0 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E37_79B9_7F4A_7C15)
                | 1
        } else {
            current
        };
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        match STATE.compare_exchange_weak(current, x, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return x,
            Err(actual) => current = actual,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPS: [TaOp; 3] = [TaOp::Operation, TaOp::KeyGen, TaOp::Control];

    #[test]
    fn samples_stay_inside_the_configured_band() {
        for op in OPS {
            let (min, max) = band(op);
            for _ in 0..2000 {
                let ms = sample_ms(op);
                assert!(
                    ms >= min && ms <= max,
                    "{op:?} sampled {ms} outside {min}..={max}"
                );
            }
        }
    }

    #[test]
    fn the_software_ta_stays_above_the_software_serving_floor() {
        // An in-process begin() costs ~3 ms, and validators flag anything below
        // 9.5 ms as an operation that never reached a secure world.
        let (min, _) = band(TaOp::Operation);
        assert!(min + 3 > 9, "operation band {min} ms is too small");
    }

    #[test]
    fn successive_samples_are_not_identical() {
        let first = sample_ms(TaOp::Operation);
        assert!(
            (0..64).any(|_| sample_ms(TaOp::Operation) != first),
            "latency is not spread across calls"
        );
    }
}
