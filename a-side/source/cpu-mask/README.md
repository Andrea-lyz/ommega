# Scoped B identity view

Settings > CPU and device properties enables a B identity view for selected
application processes. It is independent of the KeyMint scope and global switch.
The scope defaults to disabled and empty. Zygisk is required. Existing processes
retain their original view; reopen selected applications after saving. Disable
the standalone zygisk_maskcpu module before using the bundled implementation.

## Property snapshot

The B profile response includes a version-2 snapshot of 95 allowlisted properties:
product and attestation tuples, SoC aliases, build identity and timestamps,
SDK/release/patch levels, ABI lists, partition fingerprints and partition product
tuples. B reads properties only; this snapshot does not generate keys or invoke
a HAL. A caches it in data/cpu-mask-profile.conf. Relay configuration changes
invalidate the cache. App startup reads only local scope and snapshot files.
The scope writer never restarts services or performs hardware calls.

A and B must use the same snapshot schema. Missing, duplicate, malformed,
oversized or unsupported-version snapshots are rejected. Dedicated
*_for_attestation fields must be empty, allowing AndroidKeyStore to use the
ordinary B tuple. Empty optional B properties remain empty; partition product
values use B's exact partition values rather than inventing a global tuple.
A nonempty B property absent from A's property area rejects native installation
before mounting. No property records are inserted.

## Native and Java view

The engine uses authenticated pre-specialize companion IPC, private mount
namespaces, read-only snapshots and inherited mapping replacement. Completion
uses a shared status page and a pidfd lifetime check, not post-specialize socket
IPC. Host and unselected-process properties remain unchanged. Readonly short
prop_info records support at most 91 value bytes. Longer records fail explicitly;
the engine never writes them using the short layout.

After the property view completes, the Java cache update resolves and allocates
all replacements before writing any field. It covers product identity, SoC,
fingerprint, build identifiers, timestamp, Build.VERSION strings/integers and
process-appropriate ABI arrays. Full SDK and resource SDK values follow Android's
major.minor encoding. Fields absent on older Android releases are optional only
when JNI reports NoSuchFieldError. Each write is read back; a Java failure restores
all Java fields already written and reports incomplete status. Native property
verification uses direct/get/find/read/callback paths and a bounded getprop child.
The module does not generate a key or run a TEE self-test at app startup.

## Boundaries

This is a process-local identity projection, not an Android downgrade, hardware
virtualization or a hardware attestation of A. Framework APIs, VM behavior,
resource Configuration, kernel/proc/sysfs, build-property files and actual 32-bit
execution support remain A's. A failed Java phase must not be interpreted as a
complete view even when native installation succeeded. Existing global verified-
boot/SPL overrides are separate and are not changed by this feature.

Requested device properties are forwarded unchanged to B. An unmasked A tuple
cannot be attested by a different B device; rejection is not evidence that B can
be made to sign A's provisioned identifiers. Validate device-property attestation
in a coherent scoped B view when using B's chain, and retain unmasked A results as
a separate diagnostic. Likewise an older B KeyMint does not gain ML-DSA hardware
support because the application sees another SDK number.

## Tests and files

- tests/test_contract.py checks identical A/B Rust schema and C++ wire order.
- tests/profile_test.cpp tests parser rejection, atomic failure, full SDK and
  readonly-record replacement in memory. Its Android executable has no Binder,
  network, namespace or physical TEE operations.
- tests/run_rust_contracts.py imports the production Rust auth/profile modules
  into an optional host harness with property reads replaced by a forbidden mock.
  It is supplemental and never replaces Android validation.

Persistent scope/profile files remain below /data/misc/keystore/ommega/data/.
The A ZIP contains the ABI-specific Zygisk library and ommega-cpu-policy; B's ZIP
supplies matching property-profile reporting. Deployment is a separate authorized
operation; building either ZIP does not deploy or restart any device.
