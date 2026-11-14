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
tuples. The 95-field wire snapshot includes B's security patch, but the scoped
projection retains A's captured ro.build.version.security_patch instead of using
that B field. Wire order and cached snapshot validation remain unchanged.
B reads properties only; this snapshot does not generate keys or invoke
a HAL. A caches it in data/cpu-mask-profile.conf. Relay configuration changes
invalidate the cache. App startup reads only local scope and snapshot files.
The scope writer never restarts services or performs hardware calls.

A and B must use the same snapshot schema. Missing, duplicate, malformed,
oversized or unsupported-version snapshots are rejected. Dedicated
*_for_attestation fields must be empty, allowing AndroidKeyStore to use the
ordinary B tuple. Empty optional B properties remain empty; partition product
values use B's exact partition values rather than inventing a global tuple.
Properties absent from A's property area remain absent; the identity five and
ro.soc.model remain mandatory. No property records are inserted.

## Native and Java view

The engine uses authenticated pre-specialize companion IPC, private mount
namespaces, read-only snapshots and inherited mapping replacement. Completion
uses a shared status page and a pidfd lifetime check, not post-specialize socket
IPC. Host and unselected-process properties remain unchanged. Readonly short
prop_info records support at most 91 value bytes. Longer host records are retained;
the engine never writes them using the short layout.

After the property view completes, the Java cache update resolves and allocates
all replacements before writing any field. It covers product identity, SoC,
fingerprint, build identifiers, timestamp, Build.VERSION strings/integers and
process-appropriate ABI arrays. Build.VERSION.SECURITY_PATCH is never written:
its existing A value is checked against the captured native A SPL before and after
the other Java writes. Native get/find/read/callback and child-listing checks also
require the captured A SPL, including its empty/absent state; a mismatching B SPL
or missing nonempty A SPL cannot count as a complete view. Full SDK and resource
SDK values follow Android's major.minor encoding. Fields absent on older Android releases are optional only
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

- tests/test_contract.py checks identical A/B Rust schema, all 95 C++ wire slots,
  the single host-SPL projection, and native/child/Java verification call sites.
- tests/profile_test.cpp tests parser rejection, atomic failure, full SDK, every
  effective direct-property index, and readonly-record replacement in memory.
- tests/host_spl_test.cpp includes the production planner/native policy and Java
  transaction with memory-only property/JNI/logging mocks. Distinct A/B SPLs in
  both date orders and empty A/B SPLs cover directFound, zero SPL patches, native
  get/find/read/callback, child-listing policy, shared-area record preservation,
  Java success/failure/rollback and older optional fields. Other B product/SoC/SDK fields still update. It never
  calls namespace preparation/installation, the companion, or the getprop child.
  Both C++ tests can be crosscompiled for aarch64-linux-android without executing
  them or accessing any device.
- tests/run_rust_contracts.py imports the production Rust auth/profile modules
  into an optional host harness with property reads replaced by a forbidden mock.
  It is supplemental and never replaces Android validation.

From this directory, run the static and Linux-host mock tests:

```sh
python3 -B tests/test_contract.py
JNI_INCLUDE_DIR="<NDK>/toolchains/llvm/prebuilt/<host>/sysroot/usr/include" sh tests/run-pure-tests.sh
```

JNI types come from the NDK; all property and JNI operations in the regression
test use memory-only mocks. Both test translation units can also be built with
NDK clang++ using `--target=aarch64-linux-android31 -std=c++17 -Wall -Wextra -Werror`:
link the profile test with src/runtime_profile.cpp; link the SPL mock test with
src/property_storage.cpp and src/runtime_profile.cpp, plus include paths tests/mocks
and third_party. Crosscompilation does not execute the binaries. Host execution
supplements, but does not replace, the Android gate. No test invocation authorizes
installation or device execution.

Persistent scope/profile files remain below /data/misc/keystore/ommega/data/.
The A ZIP contains the ABI-specific Zygisk library and ommega-cpu-policy; B's ZIP
supplies matching property-profile reporting. Deployment is a separate authorized
operation; building either ZIP does not deploy or restart any device.
