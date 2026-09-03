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

3. Add the apps you want to intercept to `/data/adb/ommega/target.txt`
   (one package per line; `!` = force generate, `?` = force patch). The WebUI
   (`webroot/`) manages this for you under KernelSU.

   To make a scoped app use its standard TEE fallback without disabling native
   StrongBox globally, list it under `[compat].strongbox_unavailable_packages`
   in `/data/misc/keystore/ommega/injector.toml`. For matching callers only,
   `getSecurityLevel(STRONGBOX)` returns `HARDWARE_TYPE_UNAVAILABLE (-68)`;
   TEE requests continue through the normal scoop route.

4. Replace the template `keybox.xml` if you want local-mode attestation with
   your own keys.

WebUI overlay values (verified boot hash, verified boot key, security patch)
are written to `/data/misc/keystore/ommega/webui-props.sh` and mirrored into
`config.toml` `[trust]` (`vb_hash`, `vb_key`, `security_patch`,
`vendor_patchlevel`, `boot_patchlevel`). `post-fs-data.sh` applies the
properties at boot; keymint uses the same `[trust]` values in the attestation
chain. Overlay-installing the module zip does not wipe them.

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
