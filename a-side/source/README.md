# Ommega A-side Relay

Custom keystore implementation for remote TEE attestation relay (A-side).

This provides Ommega's Keystore2/KeyMint implementation through AOSP AIDL
interfaces. It runs on the A-side device and, when remote mode is enabled,
forwards attestation / sign / decrypt to a B-side real hardware TEE through the
relay_server.

## How it works

- **Local mode** (`remote: false`): uses the bundled software keybox to mint
  attestation chains, like a regular keystore spoofer.
- **Remote mode** (`remote: true`): attestation (tag 709) is minted by the
  B-side real hardware TEE via the relay_server. Sign/decrypt for remote keys
  are forwarded too. Before the software TA starts, A obtains and freezes the
  B-side stable-AIDL version/hash, canonical profile version, vendor hardware
  version, security level, and StrongBox availability. Every attestation result
  must match that profile. If the relay is temporarily unavailable during boot,
  startup retries until the remote profile is available instead of freezing a
  mismatched local profile. Profile or certificate mismatches fail closed;
  enabling local fallback does not bypass identity checks.
- **Auth-bound signing keys** (502/504/505/506/507/508 + `PURPOSE_SIGN`) are
  minted locally, even in remote mode, except for packages that need B's
  hardware chain (Google/GMS, Google Wallet/Vending, TrustAttestor,
  KeyAttestation, DuckDetector, and EngineerMode). Auth-bound decrypt/agreement
  keys stay remote. B has no A-side HardwareAuthToken and cannot validate A's
  Gatekeeper token, so a remote auth-bound key would reject every signature
  with `KeyUserNotAuthenticated` (-26). The local keybox chain keeps the
  requested authorization list and the local Keystore performs the fingerprint
  or credential check. `UNLOCKED_DEVICE_REQUIRED` (509) still uses B when its
  user is unlocked. Caller-supplied attestation-key delegation keeps its
  existing remote path.

The opt-in [Auth A / Sign B experiment](docs/EXPERIMENTAL-REMOTE-AUTH-SIGNING.md)
keeps new authenticated signing keys on B with a no-auth hardware policy and
enforces the original authentication policy in A. Build with
`--experimental-remote-auth-signing`; ordinary builds leave it disabled.

The native Soter HAL has an independent relay. Enable the software Soter TA in
the WebUI to take over `vendor.qti.hardware.soter.ISoter/default`; then configure
its own server, token, B device ID and optional `A=B` UID mapping in the Soter
dialog. These settings are stored in `/data/adb/ommega/soterta/remote.conf`,
separate from the KeyMint/Integrity relay config. With Soter remote disabled
(the default), the same HAL service uses its persistent local ASK/AuthKey ledger.
With Soter remote enabled, it forwards native HAL calls to `/api/soter/`, tagging
each request with the local ledger's `cpu_id` and, when the calling uid resolves
to installed packages, the caller's `caller_pkg` list; the relay server mints
its built-in fallback answers with that id and keeps real SOTER applications on
the real device by the package list. A transport or server failure returns a
failed HAL result rather than silently switching that key slot to local
material. The stock Soter HAL is restored when the software TA switch is turned
off.

## Install and configure

**Android 12 or above required.**

1. Install this module (KernelSU/APatch, or Magisk).

2. Open **Settings → Remote configuration** in the WebUI. The config file is
   `/data/misc/keystore/ommega/config`; the WebUI accesses the same file through
   `/data/adb/ommega/ommegadata/config`. Do not create a separate legacy
   `/data/adb/ommega/config`:

   ```
   url: http://<relay-server>:<port>
   device_id: <b-side-device-id>
   token: <relay-token>
   remote: true
   local_hw: false
   tls_insecure: true
   debug_logging: false
   ```

3. Add the apps you want to intercept to `/data/misc/keystore/ommega/target.txt`, one
   plain package name per line. The WebUI (`webroot/`) manages this and the
   optional per-package `global_default` / `strongbox` / `tee` policy stored in
   `/data/misc/keystore/ommega/target-security.toml`.

   The WebUI's **Global scope** switch takes effect without clearing the list.
   It is off by default. Turning it on sets `[filter].enabled = false` in
   `injector.toml`: user apps and ordinary system apps with an application ID
   from 10000 through 19999 in each Android user are in scope. Their package
   lists and denylist are bypassed without querying the package manager. Root,
   native core services (including system_server, Keystore, Gatekeeper and Vold),
   SDK sandboxes and isolated processes stay on System. A UID-1000 application
   is included only after its configured Binder SID and exact full process
   name match; its saved scope and denylist are then bypassed. All included
   callers use the global security-level policy.
   Turning it off restores `[filter].enabled = true`, the saved lists and their
   per-app policies. The WebUI disables list editing while global scope is on.
   Existing keys and descriptors are not migrated between System and Ommega.

   In list mode, two lists feed routing and are merged when the injector config is read:
   `scoop` in `/data/misc/keystore/ommega/injector.toml` (the shipped defaults
   plus anything added by hand) and `target.txt` (what the WebUI writes). A
   package listed in either one is routed; `target.txt` entries are appended to
   `scoop`, so the effective list is the union. `target-security.toml` only
   carries the per-app StrongBox / TEE / global-default choice.

   In list mode, shared system UID apps (app id 1000) require a process binding
   while `block_android_package = true`. The default `[filter].system_app_contexts`
   contains only the verified `com.oplus.engineermode` main process bound to
   Binder SELinux context `u:r:engineermode_app:s0`. The shipped `scoop` and
   SELinux rules already include this adapter; no manual binding or permission
   setup is needed. A matching process must still be in `scoop` or `target.txt`,
   and the denylist still applies. Configurations that omit the field inherit
   this default. An explicit
   `system_app_contexts = {}` disables the exception, and a custom map replaces
   the default bindings. Other bindings require verifying the app's UID, exact
   process name and Binder SID on the target OS. Missing process/SID evidence,
   other shared-UID processes and native domains such as `system_server` keep
   the ordinary system-UID block.
   Only application domains ending in `_app` can use this binding. This selects
   the whole keystore2 surface for that caller; it does not change authentication
   or maintenance mirroring. Clearing the map restores the system-UID block;
   keys created in Ommega stay in its separate backend. Passing a factory key
   status test also requires the selected backend to support its RSA/EC request.

   The remote settings dialog also provides a global "disable native
   StrongBox" switch. It only affects target apps whose policy is
   `global_default`; explicit StrongBox or TEE choices take precedence. All
   target and policy changes are read live by the injector.

   Keystore2 routing is all-or-nothing per caller: keys, listings and
   operations of an allowed caller always use the same backend, so the
   `[intercept]` switches in `injector.toml` select the whole surface (any
   enabled switch routes everything to ommega, none enabled passes everything
   to System).

   The detector compatibility switch in
   `/data/misc/keystore/ommega/target-compat.toml` (`[positive_key_id]`) is
   global as well: while it is on, newly created app keys in Ommega get positive
   key ids, for clients that treat a non-positive namespace as an unspecified
   key id. It is off by default, and a policy written before the switch became
   global (a nonempty `packages` list) still enables it. Existing key IDs are
   not migrated.

4. Replace the template `keybox.xml` if you want local-mode attestation with
   your own keys.

WebUI overlay values (verified boot hash, verified boot key, security patch)
are written to `/data/misc/keystore/ommega/webui-props.sh` and mirrored into
`config.toml` `[trust]` (`vb_hash`, `vb_key`, `security_patch`,
`vendor_patchlevel`, `boot_patchlevel`). Patch levels update the live TA;
verified boot hash/key changes automatically recycle only Ommega's keymint
child and wait for RPC recovery. No device reboot is required. Overlay-installing
the module zip does not wipe these values.

Keep `[trust]` on `auto` for real use and align upwards only: lowering a device's reported patch level or OS version makes the hardware KeyMint refuse to upgrade the keyblobs minted under the newer value (`INVALID_ARGUMENT`, -38), and aligning A down to an older B date also breaks Google Wallet card binding.

Boot-state properties are normalized by `post-fs-data.sh` before the framework
starts: verified boot state, boot lock and verity mode (plus the vendor-namespace
copies some bootloaders publish), the anti-debug build properties and the
user-data encryption state. Vendor-private warranty and secure-boot flags are
left untouched, because rewriting them can abort startup on some devices.
Every write is read back and
a value that did not stick is reported on stderr; properties a bootloader does
not publish are left absent. The bootloader parameters the kernel exposes on its
own (`/proc/bootconfig`, `/proc/cmdline`) are outside the property area and keep
their original values.

The boot patch level always comes from the boot image metadata (the AVB
`com.android.build.boot.security_patch` property), which is the source the
bootloader itself consumes. Only when that metadata cannot be read does the
resolver fall back to a property, and it then reads the standard name a stock
device exposes (`ro.boot.image.build.security_patch`, the command-line export)
before the vendor-flavoured `ro.vendor.boot_security_patch`. The WebUI overlay
writes that same standard name, and clearing the override restores the value the
bootloader passed instead of dropping a property a stock device exposes.

Attestation material is selected per security level. The batch keybox in
`/data/misc/keystore/ommega/keybox.xml` signs TEE attestations only; StrongBox
attestations use a dedicated keybox at
`/data/misc/keystore/ommega/keybox-strongbox.xml` (same XML schema, optional).
When that file is absent the StrongBox security level still exists, but its
attestation fails with `ATTESTATION_KEYS_NOT_PROVISIONED` instead of relabelling
the TEE chain, and `DEVICE_UNIQUE_ATTESTATION` requests are rejected with
`CANNOT_ATTEST_IDS`, the way a StrongBox implementation without device-unique
support is allowed to. Reusing one chain for both levels is externally
observable: an app can create a TEE key and a StrongBox key and compare the two
chains. Installing dedicated StrongBox material, or enabling
`main.use_native_strongbox` so the device's real StrongBox HAL serves that level,
is what makes StrongBox attestation available again.

A StrongBox request is never silently served from the TEE. A relay may answer an
attestation with a `strongbox_demoted` marker, but that result is rejected with
`HARDWARE_TYPE_UNAVAILABLE`: a physical KeyMint serves a StrongBox request from
its StrongBox instance and a device without one fails the operation, so returning
a TEE chain while `KeyMetadata.keySecurityLevel` still reports StrongBox would
leave the two app-visible security-level sources disagreeing.

If the root of trust cannot be resolved from the device (no readable verified
boot property, no readable vbmeta image and no reachable system keystore to probe
the original value), the module no longer fabricates a random verified boot
hash/key: placeholder values would contradict the verified, locked claim, change
on every restart and invalidate every stored keyblob. The record is reported as
unverified and unlocked instead, the property space is left untouched, and an
error is logged. A configured `verified_boot_state = true` is likewise only
honoured together with `device_locked = true`, because `Verified` (green) implies
a locked bootloader on a real device.

The number of concurrently open KeyMint operations (the value behind
`TOO_MANY_OPERATIONS`) follows the mirrored implementation: the relay profile may
carry an optional integer `max_operations` (1..=1024), which is validated and applied
to the TA; when it is absent, or for a locally served level, the AOSP reference
limits stay in force (16 for TEE, 4 for StrongBox). An optional `[main]`
`max_operations` entry overrides both for local testing. KeyMint exposes no API for
this value, so the B side has to measure or configure it.

> **Path note**: `/data/adb/` is root-only, so the keystore process (uid 1017)
> cannot read `/data/adb/ommega/*` directly. `post-fs-data.sh` and the
> `daemon-injector` sync `config` and `target.txt` to
> `/data/misc/keystore/ommega/` automatically, so edits take effect without a
> reboot.

## Restarting keymint and injector

The module ships two background daemons: one for `keymint`, one for `injector`.
Restart them with:

```sh
touch /data/adb/ommega/restart.keymint
touch /data/adb/ommega/restart.injector
touch /data/adb/ommega/restart.all
```

On each keymint start, its watchdog removes the previous RPC socket inode. The
injector installs its Binder hooks immediately so boot-time authorization and
maintenance events are captured even while remote identity and RPC startup are
still pending. RPC warm-up runs in the keystore2 process, and the in-memory
mirror queue replays captured state changes after the service becomes ready.

## Key consistency and lifecycle

Remote keys use B's certificate chain unchanged and inherit its available OS,
vendor and boot version tags in their returned characteristics and new keyblob.
A does not add a synthetic child leaf for authentication or unlock constraints.
Existing keys retain their key material and credential-encryption state.

The relay's physical authorization policy is distinct from A-side application
policy: B preserves the requested purposes and uses `NO_AUTH_REQUIRED` because
it cannot validate A's user token. B forwards `UNLOCKED_DEVICE_REQUIRED` (509)
independently. A enforces the application's original
authentication policy and timeout without alias-specific extensions.
This proxy does not provide hardware enforcement of A's user authentication;
the certificate reports B's actual policy. An `ATTEST_KEY` certificate must
report an exclusive hardware ATTEST_KEY purpose. A caller-selected remote App AttestKey is resolved
from its keyblob and forwarded as `device_attest_context.attestation_key` with
its alias and DER issuer subject. B uses native `generateKey` with that existing
keyblob and appends the parent chain. No private keyblob crosses the relay.
A verifies the parent public key, signed hardware purpose, issuer name and child
signature before storing the child. An unavailable issuer never falls back to
local signing, and responses from relays that ignore the issuer are rejected.

Native KeyMint delegation is authorized by the keyblob's ATTEST_KEY purpose,
independently of generic PKIX CA constraints. Legacy AOSP C++ certificate
generation can use digitalSignature without BasicConstraints for ATTEST_KEY;
these certificate fields are preserved. B checks the hardware characteristics
returned by the HAL, and A rejects signed SIGN-only or software purposes.

The delegated-response verifier supports ECDSA P-256/P-384 with SHA-256/SHA-384,
RSA 2048-8192 PKCS#1 signatures with SHA-256/SHA-384/SHA-512, and Ed25519.
Other issuer signature algorithms are rejected. This path is for key generation;
import with a remote issuer and arbitrary remote SIGN certificate issuance are
unsupported. Native B is required; the server-keybox and B-app backends do not
provide this delegation protocol. Existing legacy SIGN-backed issuers must be
regenerated by the application as native ATTEST_KEYs.

Keybox rotation retires only dedicated `ATTEST_KEY` entries bound to the old
keybox. Ordinary signing keys, including entries with legacy keybox metadata,
are excluded from retirement. This does not recover previously deleted keys.

After System succeeds, `AddAuthToken` mirroring is best-effort: a failed or lost
mirror can be dropped without poisoning the global replay state. Direct token
calls retain their validation; lock, user and maintenance events remain ordered
and fail-closed. A dropped token does not authorize an operation and may require
a later authentication to replenish the cache.

## License

`AGPL-3.0-or-later`

```plaintext
ommega - Custom keymint implementation for Android Keystore Spoofer
Copyright (C) 2025 jiyin004

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
```

## Credit

Some code from [AOSP](https://source.android.com/)

License: `Apache-2.0`

```plaintext
Copyright 2022, The Android Open Source Project

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

## Request latency

Remote signatures share a 10-second HTTP budget across both attempts and any
Retry-After delay. Attestation retains its longer budget. The injector uses up
to four connections in one RPC session, preserving Binder identities. Ordinary
remote signatures without pending authentication, trusted presence, confirmation
or single-use key deletion execute network work outside the TA lock. Restricted
operations retain the serialized finish path and existing authorization checks.
