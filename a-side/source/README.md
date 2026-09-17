# Ommega A-side Relay

Custom keystore implementation for remote TEE attestation relay (A-side).

This is a full keystore implementation that fully implements the AOSP AIDL
interface. It runs on the A-side device and, when remote mode is enabled,
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
  mismatched local profile. Profile or certificate mismatches fail closed unless
  explicit local fallback is enabled.

## Install and configure

**Android 12 or above required.**

1. Install this module (KernelSU/APatch, or Magisk).

2. Configure `/data/adb/ommega/config` (the single shared A-side config dir):

   ```
   url: http://<relay-server>:<port>
   device_id: <b-side-device-id>
   token: <relay-token>
   remote: true
   local_hw: true
   tls_insecure: true
   debug_logging: false
   ```

3. Add the apps you want to intercept to `/data/adb/ommega/target.txt`, one
   plain package name per line. The WebUI (`webroot/`) manages this and the
   optional per-package `global_default` / `strongbox` / `tee` policy stored in
   `/data/misc/keystore/ommega/target-security.toml`.

   Two lists feed routing and they are merged when the injector config is read:
   `scoop` in `/data/misc/keystore/ommega/injector.toml` (the shipped defaults
   plus anything added by hand) and `target.txt` (what the WebUI writes). A
   package listed in either one is routed; `target.txt` entries are appended to
   `scoop`, so the effective list is the union. `target-security.toml` only
   carries the per-app StrongBox / TEE / global-default choice.

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
   global as well: while it is on, every app routed through Ommega gets positive
   key ids, for clients that treat a non-positive namespace as an unspecified
   key id. It is off by default, and a policy written before the switch became
   global (a `packages` list) still enables it.

4. Replace the template `keybox.xml` if you want local-mode attestation with
   your own keys.

WebUI overlay values (verified boot hash, verified boot key, security patch)
are written to `/data/misc/keystore/ommega/webui-props.sh` and mirrored into
`config.toml` `[trust]` (`vb_hash`, `vb_key`, `security_patch`,
`vendor_patchlevel`, `boot_patchlevel`). Patch levels update the live TA;
verified boot hash/key changes automatically recycle only Ommega's keymint
child and wait for RPC recovery. No device reboot is required. Overlay-installing
the module zip does not wipe these values.

Boot-state properties are normalized by `post-fs-data.sh` before the framework
starts: verified boot state, boot lock and verity mode (plus the vendor-namespace
copies some bootloaders publish), warranty and lock-state flags, the anti-debug
build properties and the user-data encryption state. Every write is read back and
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

New local child keys signed by a remote attestation key inherit its available
OS, vendor and boot version tags before certificate generation and keyblob
serialization. The certificate, returned authorizations and stored new keyblob
use the same values. Existing keys are not rewritten; their key material and
credential-encryption state are retained.

The relay's physical authorization policy is distinct from A-side application
policy: B translates `ATTEST_KEY` to `SIGN` and uses `NO_AUTH_REQUIRED`. A keeps
the application's original purposes and authentication requirements. Replacing
those with B's values would change application behavior and weaken local
authentication. This proxy limitation is not full hardware enforcement of A's
user authentication, nor full equality between physical and logical purposes.

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
