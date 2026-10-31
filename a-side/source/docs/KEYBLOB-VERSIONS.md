# Software keyblob version checks

Software keyblob generation, import, normal loading and explicit upgrade resolve
version tags from one current implementation source. With an enabled remote
profile, OS version and system/vendor/boot patch levels use that profile. Without
it, available A-side HAL and boot metadata are used. The key's origin, alias,
descriptor and remote/local key material never select a different backend or
version policy. B's metadata must not be compared to unrelated A-local SPLs.

## Loading and upgrade

- Equal recorded/current values are accepted.
- Older recorded values cause KEY_REQUIRES_UPGRADE on normal loading.
- Future recorded values cause INVALID_KEY_BLOB on normal loading.
- Explicit upgrade can increase available version values and add new published
  tags. A requested numeric downgrade returns INVALID_ARGUMENT.
- Android's OS-version-zero exception is retained: a nonzero OS version with
  current OS version zero requires explicit upgrade to zero. This exception does
  not apply to any system/vendor/boot patch level.
- Missing optional current values are not permission to delete an existing bound
  tag. Such a tag is retained and logged as unavailable for comparison. An upgrade
  that changes nothing returns the existing empty-result/no-op signal.
- A rejected operation returns no new blob and never rewrites the caller's input.
  Successful upgrade produces a new encrypted software blob with the same key
  material and existing secure-deletion slot. Current KEK-context upgrade behavior
  is preserved.

These checks govern Ommega's software envelope, not B's physical keyblob ratchet,
and do not create hardware rollback resistance. Updating current metadata here
never changes B properties/SPL, the boot/root-of-trust encryption inputs, Android
credentials, synthetic-password/FBE state or backend routing.

## Operational limits

A blob minted with future tags can legitimately become unusable when the current
profile is lowered; strict code does not repair it by lowering the recorded values.
Keep both the pre-update module and metadata state for recovery. Do not use SPL
changes, reboots or deletion of app/Keystore data as a test or repair. Deployment
must review existing software-key dates and the effective remote profile before
running an upgrade. Old A/B mixed-metadata blobs require an explicit migration
review rather than silent downgrade.

Pure ordering tests run without dependencies, Binder or a device. Software TA
fixtures cover successful forward upgrade with unchanged key material, no-op
upgrade, each future tag, error/input preservation, missing optional partition
values and the OS-zero exception. These fixtures use the software crypto backend
and mock remote metadata only; Android device execution is a separate gate.

AOSP contract: android17-release system/keymint/ta/src/lib.rs and ta/src/keys.rs.
