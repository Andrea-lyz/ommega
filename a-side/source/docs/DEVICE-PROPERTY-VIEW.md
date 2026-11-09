# Device-property view

Some applications attest *device properties* (brand, device, product,
manufacturer, model, SoC) together with their key. The kernel asks the
application for those identifiers before key generation. When an application
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
as errors. Partition aliases that remain uncovered are counted and logged as
informational. A successful view logs:

```
MaskCPU : private property view: ... error=none
MaskCPU : child getprop tuple=<brand>/<device>/<name>/<manufacturer>/<model>
```

Refusals are explicit. `experiment refused: ...` names the property and the
reason, and `B view requires absent property` no longer aborts the view; it is
reported as `host lacks property, keeping it absent`.

## Limits

The view does not change A's global properties, does not affect unselected
applications, and does not add properties the host does not define. An
application that reads the real build fingerprint still sees the host value.
