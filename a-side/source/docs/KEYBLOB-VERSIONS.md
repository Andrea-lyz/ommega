# Software keyblob version checks

Software keyblob generation, import, normal loading and explicit upgrade use
A's frozen native firmware version target, in both local and remote modes. B's
profile is an admission check against that target, not a source that replaces A's
versions. The key's origin, alias, descriptor and remote/local material never
select a different backend or version policy. All-or-nothing per-caller routing
is unchanged.

## Native firmware target

A reads OS version and System SPL from the System partition's build-property
files, Vendor SPL from the Vendor partition's build-property files, and Boot SPL
from the active boot image's AVB metadata or native boot header. It does not use
mutable Android properties, a UI value, B's date or another partition's date as
a substitute. The startup snapshot retains value, raw representation, source and
known/unknown state. An unavailable or invalid required source fails closed; it
does not become a fabricated date or a default zero.

The four authorizations are OS_VERSION (705), OS_PATCHLEVEL (706, YYYYMM),
VENDOR_PATCHLEVEL (718, full YYYYMMDD) and BOOT_PATCHLEVEL (719, native raw
precision). A Boot header value such as YYYYMM00 stays YYYYMM00; it is not given
an invented day or borrowed from System.

A's `[trust]` fields `os_version`, `security_patch`, `os_patchlevel`,
`vendor_patchlevel` and `boot_patchlevel` accept only `auto` (native-only). An
explicit override produces `migration-required`. Preserve the original config,
keys and credential-encryption state for an explicit human migration review;
do not automatically replace an override with `auto`, lower bound values, clear
databases or delete keys. Non-SPL verified-boot hash/key and boot-state controls
remain independent.

## Remote admission

A requires B to publish `version_policy = a-native-v1`, a valid
`version_revision` and all four version tags exactly matching A's native target.
Each attestation request carries that target and expected revision. A compatible
B checks the same revision and tags again before its normal HAL call; a changed,
missing or mismatched snapshot is rejected, not silently refreshed or accepted.
The signed certificate and returned authorizations must also agree. B adapts to
A only through a separately reviewed and authorized B-side procedure; A never
lowers its own target to accommodate B. Relays without the capability fail closed
and need a separate upgrade. Remote StrongBox cannot borrow a default TEE
profile, and `server_keybox` does not supply this physical-B guarantee.

## Loading and upgrade

- Equal recorded/current values are accepted.
- Older recorded values cause KEY_REQUIRES_UPGRADE on normal loading.
- Future recorded values cause INVALID_KEY_BLOB on normal loading.
- Explicit upgrade can increase available version values and add new published
  tags. A requested numeric downgrade returns INVALID_ARGUMENT.
- Android's OS-version-zero exception is retained: a nonzero OS version with
  current OS version zero requires explicit upgrade to zero. This exception does
  not apply to any system/vendor/boot patch level.
- Absence of a value in a blob comparison is not permission to delete an existing
  bound tag. Such a tag is retained and logged as unavailable for comparison; this
  does not relax the complete four-tag native target required for normal startup
  and remote admission. An upgrade that changes nothing returns the existing
  empty-result/no-op signal.
- A rejected operation returns no new blob and never rewrites the caller's input.
  Successful upgrade produces a new encrypted software blob with the same key
  material and existing secure-deletion slot. Current KEK-context upgrade behavior
  is preserved.

These checks govern A's software envelope, not B's physical keyblob ratchet,
and do not create hardware rollback resistance. B keeps its actual hardware
keyblob; matching A-side metadata does not rewrite, downgrade or prove the state
of that blob. A does not change A/B SPL properties, boot/root-of-trust encryption
inputs, Android credentials, synthetic-password/FBE state or backend routing as
part of version admission. Any B-side adaptation is a separate operation.

## Operational limits

A blob bound above the native target can legitimately be unusable; strict checks
do not repair it by lowering its recorded values. Preserve the module, original
config and metadata state for recovery review. Existing explicit overrides and
mixed-source blobs require a human migration decision, never automatic config
rewriting or database/key deletion. B hardware keyblobs may already be ratcheted
above their firmware baseline; that baseline is not automatically a safe rollback.
Do not combine SPL changes with reboot or use credential/Keystore data deletion
as a test or repair.

The current deployment plan is A local mode only. It authorizes no B upgrade,
property change, service reload or device test; remote acceptance requires a
separately authorized compatible B. Neither a status JSON value nor `getprop`
proves physical TA acceptance, and no Wallet, payment or hardware guarantee is
implied. The UI reads existing `spl-status.json` only and does not fetch a profile
or invoke hardware. Unknown TA/certificate evidence remains unknown.

Pure ordering tests run without dependencies, Binder or a device. Software TA
fixtures cover successful forward upgrade with unchanged key material, no-op
upgrade, each future tag, error/input preservation, missing optional partition
values and the OS-zero exception. These fixtures use the software crypto backend
and mock remote metadata only; Android device execution is a separate gate.

AOSP contract: android17-release system/keymint/ta/src/lib.rs and ta/src/keys.rs.
