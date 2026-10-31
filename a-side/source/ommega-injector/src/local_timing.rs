//! Optional per-application EWMA equalization for injector-observed keygen.
//!
//! The injector sees the caller package even when a no-challenge generateKey
//! carries no AAID. Remote-shaped samples are challenge+no-attestation-key;
//! no-challenge local requests are held to that EWMA when the app opts in.

use std::{
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    sync::Mutex,
    time::{Duration, Instant},
};

const PATH: &str = "/data/misc/keystore/ommega/data/local-keygen.txt";
const MAX_BYTES: u64 = 64 * 1024;
const EWMA_MAX_NS: u64 = 2_000_000_000;
const SAMPLE_MAX_NS: u64 = 250_000_000;
const TARGET_DELTA_NS: u64 = 110_000_000;
type Stamp = (u64, u64, u64, i64, i64);
type Cached = Option<(Stamp, Vec<String>)>;

static EWMA_NS: Mutex<u64> = Mutex::new(0);
static PACKAGES: Mutex<Cached> = Mutex::new(None);

fn enabled(packages: &[String]) -> bool {
    if packages.is_empty() {
        return false;
    }
    let Ok(mut cache) = PACKAGES.lock() else {
        return false;
    };
    let file = match fs::File::open(PATH) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            *cache = None;
            return false;
        }
        Err(_) => return false,
    };
    let Ok(meta) = file.metadata() else {
        return false;
    };
    let stamp = (
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
    );
    let configured = if let Some((previous, values)) = cache.as_ref() {
        if *previous == stamp {
            values.clone()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let configured = if configured.is_empty() && cache.as_ref().is_none_or(|(p, _)| *p != stamp) {
        if !meta.is_file() || meta.len() > MAX_BYTES {
            *cache = Some((stamp, Vec::new()));
            return false;
        }
        let mut contents = String::new();
        if file
            .take(MAX_BYTES + 1)
            .read_to_string(&mut contents)
            .is_err()
        {
            return false;
        }
        let values = contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#') && line.len() <= 255)
            .map(str::to_string)
            .collect::<Vec<_>>();
        *cache = Some((stamp, values.clone()));
        values
    } else {
        configured
    };
    packages
        .iter()
        .any(|package| configured.iter().any(|entry| entry == package))
}

fn record_sample(elapsed: Duration) {
    if elapsed.as_nanos() > u128::from(SAMPLE_MAX_NS) {
        log::warn!(
            "event=local_keygen_sample_dropped elapsed_us={} reason=queue_inflated",
            elapsed.as_micros()
        );
        return;
    }
    let sample = elapsed.as_nanos().min(u128::from(EWMA_MAX_NS)) as u64;
    let Ok(mut ewma) = EWMA_NS.lock() else {
        return;
    };
    *ewma = /* α = 1/4 */
        if *ewma == 0 { sample } else { ewma.saturating_mul(3) / 4 + sample / 4 };
}

fn equalize(packages: &[String], started: Instant) {
    if !enabled(packages) {
        return;
    }
    let Ok(ewma) = EWMA_NS.lock() else {
        return;
    };
    let target = ewma.saturating_sub(TARGET_DELTA_NS);
    drop(ewma);
    if target == 0 {
        return;
    }
    let elapsed = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
    if elapsed < target {
        log::debug!(
            "event=local_keygen_equalize source=injector target_us={} elapsed_us={} sleep_us={} packages={:?}",
            target / 1000,
            elapsed / 1000,
            (target - elapsed) / 1000,
            packages
        );
        std::thread::sleep(Duration::from_nanos(target - elapsed));
    }
}

/// Call after a successful GenerateKey. Remote-shaped samples (challenge, no
/// attestation key) update the EWMA; no-challenge local requests may sleep.
pub(crate) fn after_generate(
    packages: &[String],
    challenge: bool,
    has_attestation_key: bool,
    started: Instant,
) {
    log::info!(
        "event=local_keygen_observe challenge={} attestation_key={} elapsed_us={} packages={:?}",
        challenge,
        has_attestation_key,
        started.elapsed().as_micros(),
        packages
    );
    if challenge && !has_attestation_key {
        record_sample(started.elapsed());
    }
    if !challenge || has_attestation_key {
        equalize(packages, started);
    }
}
