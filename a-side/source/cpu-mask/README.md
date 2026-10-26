# Ommega CPU and device properties (exp6)

Settings has an independent **CPU and device properties** entry. Its first dialog
contains the enable switch, scope picker and read-only B properties. The scope
dialog supports app-name/package search, user apps, optional system apps and
multiple selections. Confirming a scope updates the parent draft; Save settings
commits the enable switch and entire scope together. Going back discards the
current dialog's unconfirmed changes. Existing dialog history and WebView back
handling own both levels and restore focus to their opener.

The scope defaults to disabled and empty. It does not follow KeyMint's scope or
global switch. Zygisk is required. Reopen the selected A apps after saving;
already-running processes retain their original view until they exit. Disable
the standalone `zygisk_maskcpu` module before using the integrated copy.

The existing B profile response supplies the brand/device/product/manufacturer/
model tuple, SoC model, Oppo CPU alias and device-family alias. A caches that
snapshot in `data/cpu-mask-profile.conf`; changing the relay configuration
invalidates it. App initialization only reads local configuration and that
snapshot. The scope writer cannot restart services or perform hardware calls.

The exp6 engine uses pre-specialize companion authentication, private mount
namespaces, read-only property snapshots and inherited mapping replacement.
Post-specialize completion uses its shared status page and pidfd lifetime check;
there is no post-specialize socket exchange. Java Build caches are written only
after property-view completion. Host properties remain in their original view.

The accepted layout is exp6's short readonly `prop_info` layout (at most 91 value
bytes), with empty dedicated `*_for_attestation` properties and the original
partition-alias checks. B properties may have different lengths from A; the
existing complete-record writer updates the value union and serial length.
Missing/invalid snapshots, unsupported layouts and an enabled standalone copy
skip the application view. No property records are inserted. The integration
does not perform key generation or hardware-attestation self-tests.

Production files are `data/cpu-mask.conf` and `data/cpu-mask-profile.conf` below
`/data/misc/keystore/ommega/`. The complete A ZIP contains the ABI-specific Zygisk
library and `ommega-cpu-policy`; the complete B ZIP contains profile reporting.
