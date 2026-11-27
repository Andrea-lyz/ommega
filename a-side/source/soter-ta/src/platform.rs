//! Platform facts the pure core cannot know.
//!
//! A stock Soter TA signs only inside a fresh biometric match, and it reads that
//! match where it lives: in the secure world, from the fingerprint TA. The
//! software TA is an ordinary user-space process, so the closest it can get is
//! the framework's accepted-capture counter, which the daemon reads through the
//! hooks registered here (`soterta_set_platform`).
//!
//! A mark packs the daemon's per-boot tag in the high [`BOOT_TAG_SHIFT`] bits and
//! the counter in the low 32, so two marks are only comparable inside one boot:
//! a counter that restarted with the device must never look like evidence. The
//! daemon keeps the packed value non-negative on purpose — -1 means the counter
//! has never been readable and -2 means it was readable earlier but is not now —
//! so the tag has to fit in 31 bits.

use std::sync::Mutex;

/// Bits of a mark that carry the per-boot tag.
pub const BOOT_TAG_SHIFT: u32 = 32;

/// What the platform can tell the software TA about one transaction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Platform {
    /// Milliseconds on the daemon's boot clock.
    pub boot_ms: Option<u64>,
    /// Fingerprint mark: accepted captures since boot, tagged with the boot.
    pub bio_mark: Option<u64>,
    /// The platform exposed the counter before but cannot be read now.
    pub bio_unreadable: bool,
}

/// A daemon hook. -1 means never readable; -2 means readable earlier but not now.
pub type Probe = unsafe extern "C" fn() -> i64;

#[derive(Clone, Copy)]
struct Hooks {
    boot_ms: Option<Probe>,
    bio_mark: Option<Probe>,
}

static HOOKS: Mutex<Hooks> = Mutex::new(Hooks {
    boot_ms: None,
    bio_mark: None,
});

/// Install the daemon's hooks; `None` leaves that fact unverifiable.
pub fn install(boot_ms: Option<Probe>, bio_mark: Option<Probe>) {
    let mut hooks = HOOKS.lock().unwrap_or_else(|error| error.into_inner());
    *hooks = Hooks { boot_ms, bio_mark };
}

/// Ask the hooks for the current values.
pub fn current() -> Platform {
    let hooks = *HOOKS.lock().unwrap_or_else(|error| error.into_inner());
    let bio = ask_with_status(hooks.bio_mark);
    Platform {
        boot_ms: ask(hooks.boot_ms),
        bio_mark: bio.0,
        bio_unreadable: bio.1,
    }
}

fn ask(hook: Option<Probe>) -> Option<u64> {
    ask_with_status(hook).0
}

fn ask_with_status(hook: Option<Probe>) -> (Option<u64>, bool) {
    let Some(hook) = hook else {
        return (None, false);
    };
    let value = unsafe { hook() };
    if value == -2 {
        (None, true)
    } else if value < 0 {
        (None, false)
    } else {
        (Some(value as u64), false)
    }
}

/// Whether two marks were taken inside the same boot.
pub fn same_boot(left: u64, right: u64) -> bool {
    left >> BOOT_TAG_SHIFT == right >> BOOT_TAG_SHIFT
}
