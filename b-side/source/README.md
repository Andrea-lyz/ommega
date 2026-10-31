# ommegaclient-b (B-side relay agent)

B-side relay agent for the ommega remote-TEE attestation setup.

This module contains the **native B-side relay agent**, for a dedicated one-A /
one-B deployment. It does not ship A's software keystore or injector. It receives
tasks from Server, calls the real KeyMint/SOTER hardware path and returns results,
including attestation chains embedding the caller-supplied app ID (tag 709).

## What is this?

The device this module is installed on acts as the **B-side** of a remote TEE
attestation setup. A companion A-side device routes selected app requests through
the relay_server for hardware-TEE
attestation. The server pushes the request as a task to this device; the `relay`
daemon executes it against the real TEE and returns the certificate chain so the
A-side device can validate and return them. Unlocking A's bootloader does not
disable its TEE, and receiving a chain does not guarantee a Play Integrity or
payment-application verdict.

The relay also serves a `profile` task. It reads the real default KeyMint HAL's
stable-AIDL version/hash and `getHardwareInfo()` values, reports StrongBox
availability, and freezes that identity for the relay process. Attestation
results carry the same profile so A can reject mixed A/B identities.
The profile includes `version_tags` from the default TEE's hardware-enforced
key characteristics: OS version, OS patch level and optional vendor/boot patch
levels. Once per relay process, a temporary AES-256/GCM key reads these values
from `generateKey`; `deleteKey` releases it immediately, including when parsing
fails. It has no challenge, attestation, alias or persisted session. The profile
does not infer these values from properties and does not change SPL. After an
approved HAL/SPL reconfiguration, refresh the relay process and A's profile
cache so the snapshot describes the current HAL state.

The same response includes an `app_identity` version-2 snapshot of 95
allowlisted product, build, SDK, ABI, SoC and partition properties for A's scoped
B identity view. This component reads properties only and adds no HAL calls to
profile sampling. A and B must use the matching snapshot schema; an unsupported
snapshot disables the scoped property view, not KeyMint routing. Empty optional
fields remain empty. The snapshot does not describe kernel or actual API/ABI
execution capabilities on A. See [Scoped B identity view](../../a-side/source/cpu-mask/README.md).

App AttestKey delegation uses `device_attest_context.attestation_key` containing
the existing session alias and a base64 DER `issuer_subject_name`. B requires
that session to belong to the selected HAL and its certificate to have the
requested subject. Generation checks the HAL's returned hardware ATTEST_KEY
authorization; native delegation enforces the existing keyblob's purpose.
Certificate CA/KeyUsage extensions are preserved without a generic PKIX gate.
It passes the existing keyblob to
native `generateKey(..., AttestationKey)` and appends its unmodified issuer
chain to the returned child. It does not issue certificates with `begin(SIGN)`.
Hardware-requested issuer keyblob upgrades are persisted before one retry.

B forwards the requested user-authentication and authorization tags (502-509)
to the real HAL, including user SIDs, authentication type, timeout and
`UNLOCKED_DEVICE_REQUIRED`. It adds `NO_AUTH_REQUIRED` only when no explicit
authentication policy was supplied; unsupported tags fail rather than silently
weakening the policy. A also enforces its original authentication policy.
B cannot authenticate A's user token, so operations on auth-bound hardware keys
can still require authentication the relay cannot provide. The certificate
reports B's actual key policy; it does not prove B accepted A's fingerprint.

## Install and configure

**Android 12 or above required.**

1. Install this module.
2. Open the module WebUI's **Connection** page (server URL, device
   ID, optional machine ID, token, and file/logcat settings). Editing
   `/data/adb/ommega/relay.conf` directly is also supported:

```ini
OMMEGA_RELAY_SERVER=https://<relay-server>:8443
OMMEGA_RELAY_DEVICE_ID=device-b-2
OMMEGA_RELAY_MACHINE_ID=
OMMEGA_RELAY_TOKEN=<relay-token>
OMMEGA_RELAY_LOG_ENABLED=true
OMMEGA_RELAY_LOG_LEVEL=debug
OMMEGA_RELAY_LOGCAT_ENABLED=true
OMMEGA_RELAY_LOGCAT_LEVEL=info
```

- `OMMEGA_RELAY_SERVER` — base URL of the relay_server (required; both `http://`
  and `https://` are supported, and the relay accepts self-signed certs).
- `OMMEGA_RELAY_DEVICE_ID` — this B-side device's id as registered on the
  relay_server (required).
- `OMMEGA_RELAY_MACHINE_ID` — machine id used in the `b/poll` query (optional).
- `OMMEGA_RELAY_TOKEN` — B-side token sent as `X-Relay-Token` (required).
- All four logging keys are managed in one place (`relay.conf`) and read at
  relay startup. Saving them does not change the running logger; they take
  effect on the next approved relay start. The WebUI does not restart services:
  - `OMMEGA_RELAY_LOG_ENABLED` — `true` writes `/data/adb/ommega/logs/relay.log`
    (default), `false` disables the file log.
  - `OMMEGA_RELAY_LOG_LEVEL` — file log level when enabled:
    `off|error|warn|info|debug|trace` (default `debug`).
  - `OMMEGA_RELAY_LOGCAT_ENABLED` — `true` keeps Android logcat output (tag
    `ommegaclient-b`, default), `false` silences logcat completely.
  - `OMMEGA_RELAY_LOGCAT_LEVEL` — logcat level:
    `off|error|warn|info|debug|trace` (default `info`).

3. The relay **hot-reloads connection config at runtime**: a background thread inside the
   `relay` process watches `relay.conf` for changes and the `restart.all`
   marker, and updates the live config in place — **no process restart needed**.
   To force an immediate reload
   after editing the file:

```sh
touch /data/adb/ommega/restart.all
```

## Relay and SPL WebUI

The connection and logging form reads all eight keys from the existing file.
Token is masked by default. Saving validates the fields, preserves comments
and unknown keys, consolidates duplicate known keys, and atomically replaces
`relay.conf` with mode `0600`. The previous file is retained at
`relay.conf.webui.bak`; stale saves are rejected if the file changed since it
was loaded. Configuration contents are data, never executed as shell code.

Connection edits notify the existing relay watcher via `restart.all`; in-flight
tasks retain their original connection configuration. Logging edits are marked
as taking effect on the next relay start. Saving Relay settings does not call
`service.sh`, change SPL, or restart any process.

Local config parsing and shell-write tests (Node.js plus POSIX sh / Git Bash):

```sh
node --test scripts/webui.test.mjs
```

KernelSU/APatch exposes the bundled `webroot/` UI. Its separate SPL section stores optional system,
boot and vendor SPL overrides in `/data/adb/ommega/spl.conf`. Saving invokes
`spl-control.sh` immediately; changed properties recycle the native KeyMint
service and `keystore2`, then wait for the default KeyMint Binder to return.

Patch levels and the Android release are one-way ratchets for the hardware
KeyMint: a key blob minted or upgraded under a newer value refuses to be
upgraded once the device reports an older one, after which every sign and
attestation for those keys fails with `INVALID_ARGUMENT` (-38) and nothing on
the module side can undo it. A value may therefore only move up, never below
the device's own original value and never below a date that was already
reported. `spl-control.sh` reads the original from the read-only firmware
property files (`/system/build.prop`, `/vendor/build.prop`, ...) and records it
in `/data/adb/ommega/spl-origin.conf`: the live property may already carry an
override and `/data/adb/ommega/spl-baseline.conf` may have been captured while
one was active, so neither of them is authoritative. Of the readings the newest
wins, a firmware update raises the floor, and a recorded floor is never dropped
because a file cannot be read.

The floor is the newest of the device's own value, the value the device
currently reports and the target the module already applied, which is recorded
in `/data/adb/ommega/spl-applied.conf`. A date that was ever reported is never
taken back, and an emptied field keeps what the device reports instead of
restoring the device's own value: lowering the reported date is what makes the
key blobs minted under it unusable. Preserve `spl-applied.conf` across module
upgrades and rollbacks. If properties fall back, the next `apply` reapplies the
recorded target; this is not a guarantee of recovery if root is unavailable or
unlock fails before application. A refused write only warns on stderr at boot
time, while an interactive `save` returns non-zero so the WebUI shows the
floor and the reason.

The B-side module intentionally uses `service.sh` as its only lifecycle entry
point. It does not ship `post-fs-data.sh` and never requests a hard reboot.
`service.sh` reapplies the persisted SPL configuration before starting relay.
Whether a boot SPL override is accepted must be verified in a newly generated
hardware attestation certificate; a changed Android property alone is not
proof that the TEE accepted it.

## Relay daemon

The module ships one daemon, `relay`. It is started directly by
`service.sh` (which kills stale instances first, then spawns a fresh one). The
`relay` process itself monitors `/data/adb/ommega/relay.conf` and reloads on
change; the wrapper never kills it, so the two never conflict. One thread
long-polls with `timeout=20` and a 25-second HTTP request deadline. Enqueued
work wakes a held poll immediately; 20 seconds is the maximum idle hold, not
a per-task delay. A fast empty 204 has an 80ms minimum cadence for compatibility
with servers/proxies that do not hold the request. A successful task response
is followed by an immediate new poll. Three worker threads run TEE
`generateKey` / sign / decrypt with a zero-capacity dispatch channel; the
concurrency limit and backpressure remain fixed. A held poll and a result
upload use the shared client's independent connections, not a global request lock.

Logs go to logcat (tag `ommegaclient-b`) and `/data/adb/ommega/logs/relay.log`.
Each poll, config reload, task receipt and result submission is logged.
`event=relay_task` records monotonic `dispatch_us`, `execute_us`, `upload_us`
and `upload_ok` after the upload attempt. Execution includes parsing, local
policy/profile work and HAL time; it is not a pure TEE timer.
`event=relay_upload` includes task ID, attempts and cumulative upload time
(including retries). A held idle poll's HTTP duration is not network RTT.
No task payload, key or new persistent metric file is added by these events.

B-side protocol implemented: `GET /api/b/poll/` + `POST /api/b/result/`.

### Battery heartbeat telemetry

The existing authenticated poll carries optional `battery_percent=0..100` and
`battery_charging=true|false` query fields. The relay reads only the standard
`/sys/class/power_supply/battery/{present,capacity,status}` sysfs attributes in
Rust. It samples at most once every five seconds (an in-memory cache, not a new
thread or persistent file), including when a busy queue causes rapid polls.
Unreadable, absent, unsupported and malformed attributes are omitted; a failed
refresh clears the previous reading. Both 0% and 100% are valid. `Charging`
means true; `Discharging`, `Not charging` and `Full` mean false (active charging,
not simply plugged in); `Unknown` and unrecognized states remain unknown. There
are no shell commands, Binder/HAL/TEE calls, sysfs writes or configuration changes.

The server publishes these additive fields on `GET /api/status/` in each
`online_b_devices` row, and on the admin connected-device list:

- `battery_percent`: integer 0..100, or JSON null.
- `battery_charging`: boolean, or JSON null, independent of percent availability.
- `battery_updated_at_ms`: server Unix-millisecond receipt time of the accepted
  poll when at least one battery value is valid; otherwise JSON null. It is not
  the B clock or a measurement timestamp (the sample may be up to five seconds
  old), and long-poll wake-ups must not refresh it.

A later missing/invalid attribute clears that attribute instead of retaining a
last-good value. Old relays and the B APK may omit both query fields and continue
to receive the unchanged task response; their battery fields are null. Older
servers ignore the added query fields. Consumers must select the exact configured
B device, enforce freshness against server time, and never substitute zero or
another device's battery. Devices absent from the online list have no current
battery report. B and server binaries both need an explicitly approved update
before this telemetry is available; UI changes alone cannot create readings.

Task types handled: `profile`, `attest`, `sign`, `decrypt`, `agree` and `soter`.
SOTER calls are serialized. Protocol entries are not a guarantee that every
algorithm/firmware combination has device acceptance; new direct attested
`AGREE_KEY` experiments and wrapped import are outside the supported validation scope.

## License

`AGPL-3.0-or-later`

```plaintext
ommegaclient-b - B-side relay agent for the ommega remote-TEE attestation setup
Copyright (C) 2026 ommegaclient

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

## Screen-off relay availability

`OMMEGA_RELAY_WAKELOCK=true` keeps the CPU available for relay traffic while the
screen is off. It increases battery use and requires the kernel's
`CONFIG_PM_WAKELOCKS` sysfs interface. The relay renews a 90-second named lease
every 30 seconds. A stopped or aborted process cannot leave this lease active
indefinitely. Setting the option to false releases the relay's lock within
30 seconds; other wake locks are untouched. Unsupported or denied interfaces
produce a warning, so screen-off availability is not claimed in that case.

Updates add missing supported settings to `relay.conf`, preserving existing
values, credentials, device identities and any network binding preference.
SOTER mutation and capability probes remain opt-in. A refused sign attempt does
not advertise permanent inability; successful signing is the positive evidence.
