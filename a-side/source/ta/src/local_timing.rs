//! Optional per-application EWMA equalization for local key generation.
//!
//! Remote attested generation records its wall time globally. When an
//! application is listed in `local-keygen.txt`, every local generateKey in
//! that application waits until the same EWMA target. The feature is opt-in,
//! per application, and off when the file is absent.

use kmr_wire::keymint::KeyParam;
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
const EQUALIZE_MIN_NS: u64 = 50_000_000;
type Stamp = (u64, u64, u64, i64, i64);
type Cached = Option<(Stamp, Vec<Vec<u8>>)>;

static EWMA_NS: Mutex<u64> = Mutex::new(0);
static PACKAGES: Mutex<Cached> = Mutex::new(None);

fn packages() -> Vec<Vec<u8>> {
    let Ok(mut cache) = PACKAGES.lock() else {
        return Vec::new();
    };
    let file = match fs::File::open(PATH) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            *cache = None;
            return Vec::new();
        }
        Err(error) => {
            log::warn!("local-keygen policy open failed: {error}");
            return Vec::new();
        }
    };
    let Ok(meta) = file.metadata() else {
        return Vec::new();
    };
    let stamp = (
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
    );
    if let Some((previous, values)) = cache.as_ref() {
        if *previous == stamp {
            return values.clone();
        }
    }
    if !meta.is_file() || meta.len() > MAX_BYTES {
        log::warn!("invalid local-keygen policy file");
        *cache = Some((stamp, Vec::new()));
        return Vec::new();
    }
    let mut contents = String::new();
    if file
        .take(MAX_BYTES + 1)
        .read_to_string(&mut contents)
        .is_err()
    {
        return Vec::new();
    }
    let values = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| line.len() <= 255)
        .map(|line| line.as_bytes().to_vec())
        .collect::<Vec<_>>();
    *cache = Some((stamp, values.clone()));
    values
}

fn enabled(params: &[KeyParam]) -> bool {
    let Some(app_id) = params.iter().find_map(|param| match param {
        KeyParam::AttestationApplicationId(value) => Some(value.as_slice()),
        _ => None,
    }) else {
        return false;
    };
    packages().iter().any(|package| {
        !package.is_empty()
            && package.len() <= app_id.len()
            && app_id
                .windows(package.len())
                .any(|window| window == package.as_slice())
    })
}

/// Record a successful remote attested generateKey wall time.
pub fn record_remote_attest_duration(elapsed: Duration) {
    let sample = elapsed.as_nanos().min(u128::from(EWMA_MAX_NS)) as u64;
    let Ok(mut ewma) = EWMA_NS.lock() else {
        return;
    };
    *ewma = if *ewma == 0 {
        sample
    } else {
        ewma.saturating_mul(3) / 4 + sample / 4
    };
}

/// If this caller opted in, hold local generateKey to the current EWMA target.
pub fn equalize_if_enabled(params: &[KeyParam], started: Instant) {
    if !enabled(params) {
        return;
    }
    let Ok(ewma) = EWMA_NS.lock() else {
        return;
    };
    let target = *ewma;
    drop(ewma);
    if target < EQUALIZE_MIN_NS {
        return;
    }
    let elapsed = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
    if elapsed < target {
        log::debug!(
            "event=local_keygen_equalize target_us={} elapsed_us={} sleep_us={}",
            target / 1000,
            elapsed / 1000,
            (target - elapsed) / 1000
        );
        std::thread::sleep(Duration::from_nanos(target - elapsed));
    }
}
