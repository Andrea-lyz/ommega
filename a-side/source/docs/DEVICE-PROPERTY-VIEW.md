# Device-property view

Some applications attest *device properties* (brand, device, product,
manufacturer, model, SoC) together with their key. Android's keystore client
collects those identifiers from the application-visible properties before key
generation. When an application
runs on A but its attestation is signed by the relay peer B, the identifiers A
supplies do not belong to B, and B answers `CANNOT_ATTEST_IDS` (-66). The
device-property view closes that gap: for selected applications only, the
properties A reports are replaced with the identity recorded in the relay
profile.

## Scope

The view is applied inside the selected application's mount namespace. Host
properties are not modified, and applications that are not selected keep the
host identity.

Enable it per package and choose the profile in **Settings → App policies →
CPU and device properties**. The configuration lives in
`/data/misc/keystore/ommega/data/cpu-mask.conf`:

```
version=1
enabled=true
package=net.one97.paytm
```

The profile is the snapshot the relay published at
`/data/misc/keystore/ommega/data/cpu-mask-profile.conf`. The view only applies
to processes started after it is enabled; restart the target application.

## What is replaced

- The identity five (`ro.product.brand`, `device`, `name`, `manufacturer`,
  `model`) and `ro.soc.model` are mandatory. A profile missing any of them is
  rejected before anything is mounted.
- Partition aliases (`ro.product.vendor.*`, `ro.product.system.*` and the other
  product partitions) follow the identity values.
- The dedicated attestation properties (`ro.product.*_for_attestation`) receive
  the target identity instead of being cleared. Vendors disagree here: many
  leave them unset, while some (Xiaomi, for example) keep the real values. The
  application reads these properties first, so publishing the target identity
  makes it request exactly what the relay can attest.
- `ro.build.version.security_patch` and `Build.VERSION.SECURITY_PATCH` retain
  A's original values, even when B's 95-field identity snapshot contains a
  different SPL. The wire slot is retained but is not a target for a B SPL write.
  Other supported B identity, SoC and build fields keep their existing projection.
- Values longer than the 92-byte short-property form (build fingerprints,
  `ro.build.version.known_codenames`) are left untouched. The readonly property
  area cannot carry them, and the host snapshot cannot represent them either, so
  they are reported as `long property retained` and skipped.
- Properties the host never defines (the `ro.product.odm_dlkm.*` family on
  devices without that partition) stay absent. The readonly area cannot gain
  entries, so they are reported and skipped rather than failing the view.

## Self-check

After the mount the companion compares the native and child tuples with the
expected values. The check mirrors the injection policy: long values, values
absent on the host, and dedicated properties without a target are not treated
as errors for B-projected fields. The retained A SPL is stricter: native
get/find/read/callback and child-listing checks must agree with the captured A
value; a missing nonempty A value or a substituted B value fails the check. Java
SECURITY_PATCH is read before and after the other Java writes, never written.
A Java failure rolls back those Java writes and reports an incomplete view; a
successful native phase alone is not complete. Partition aliases that remain
uncovered are counted and logged as informational. A successful view logs:

```
MaskCPU : private property view: ... error=none
MaskCPU : child getprop tuple=<brand>/<device>/<name>/<manufacturer>/<model>
```

Refusals are explicit. `experiment refused: ...` names the property and reason.
An optional B property absent on the host is reported as
`host lacks property, keeping it absent`. No property record is inserted.

## Limits

The view does not change A's global properties, does not affect unselected
applications, and does not add properties the host does not define. Build-property
files, kernel and hardware behavior remain A's; a long unpatchable fingerprint
stays a host value. This view neither changes native firmware version sources
nor bypasses the remote `a-native-v1` admission contract. B must independently
match A's native four-tag target and revision; a product-identity view is not
proof of hardware acceptance, Wallet compatibility or payment/card support.

The SPL status page reads only
`/data/misc/keystore/ommega/data/spl-status.json`; opening or refreshing it does
not acquire a B profile or invoke a HAL. Firmware, Android properties, frozen
software-TA configuration, cached B profile and certificate observations are
distinct sources. Unknown TA/certificate observations stay unknown. See
[Software keyblob version checks](KEYBLOB-VERSIONS.md) and
[Relay integrity](RELAY-INTEGRITY.md).
