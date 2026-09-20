# Ommega — Andrea-lyz Fork

[![中文](https://img.shields.io/badge/语言-中文-lightgrey)](README.md)
[![English](https://img.shields.io/badge/Language-English-blue)](README.en.md)
[![中文使用说明](https://img.shields.io/badge/使用说明-中文-lightgrey)](ommega远程转发密钥说明.txt)
[![English guide](https://img.shields.io/badge/Guide-English-blue)](ommega-remote-key-guide.en.txt)

> [!IMPORTANT]
> Community fork of [jiyin004-jpg/ommega](https://github.com/jiyin004-jpg/ommega),
> not an official upstream release or hosted service. Source version: **1.4.6**.
> Module authors: `jiyin004, Andrea-lyz`.

> [!WARNING]
> **Run B on a spare device that holds no personal data, and keep an off-device backup.**
> The B module rewrites the System/Boot/Vendor security patch level (SPL) and reloads
> native KeyMint. That touches credential encryption and keyblob upgrade: once a local
> keyblob is upgraded under the higher SPL, it may require that higher value from then on.
> If root disappears after a restart (power loss, a wrong step, or the module not loading
> at boot), the SPL override is not reapplied, and the device can end up with **a correct
> password that no longer unlocks and credential-encrypted data that is unavailable**,
> leaving a factory reset as the only way out.
> Treat the B device as disposable: keep no primary or irreplaceable data on it, and do
> not assume root will be available. The current boot-time reapply and script guards do
> not guarantee native unlock once root is lost.

Ommega connects A, Server and B for remote KeyMint operations. A intercepts Android
Keystore requests for selected applications. Eligible generation and subsequent
remote signing, decryption and agreement use B hardware through Server.
A also provides the software TA, local storage and application authentication checks.

## Test environment and support boundaries

This is a **2026-09-05 test snapshot**, not a supported-device list. Firmware values
were read from the devices; app versions and results belong to earlier test windows
that day. Updating a component does not automatically preserve those results.

| Item | A | B |
|---|---|---|
| Model | OnePlus 13, PJZ110 | OnePlus 11, PHB110 |
| Android / API / ABI | Android 16 / API 36 / arm64-v8a | Android 16 / API 36 / arm64-v8a |
| Firmware | `PJZ110_16.0.10.501(CN01)` | `PHB110_16.0.2.400(CN01)` |
| ColorOS property `ro.build.version.oplusrom` | `V16.1.0` | `V16.0.0` |
| Tested hardware path | Native StrongBox + Android RKP tested separately; disabled for remote acceptance | Default TEE KeyMint through native relay |
| StrongBox | Native implementation exists; Mask makes the feature query false | Query false; historical profile `strongbox=false`; B StrongBox unvalidated |
| `app_attest_key` query | true; retained by Mask | true |

Mask was validated with LSPosed 2.2.0 / libxposed API 102. A/B run rooted; this does not
establish compatibility with every root manager or ROM. Server is Linux x86_64,
physical mode; CI provides a musl binary.

Remote acceptance used A `remote=true`, `local_hw=false`,
`disable_native_strongbox=true`, `use_native_strongbox=false`, GlobalDefault
policies and Mask enabled. This tests **A → Server → B TEE**, not StrongBox on both
devices. Historical versions: Integrity Checker 2.2, Paytm 10.83.12, GMS 26.34.31,
Play Store 52.9.21-34. Three green Integrity results were observed. Paytm reached
phone-number entry but also intermittently reported `00000`. Improvement after reboot
does not prove a false positive or permanent fix, or establish payment/card compatibility.

A's installer and Mask have an API 29 installation floor. This is not a promise for
all Android 10+ devices: keystore2, KeyMint AIDL and injector/firmware compatibility
still matter. CI distributes arm64-v8a modules. Other versions, architectures and B
StrongBox require separate validation.

## StrongBoxCapabilityMask: use, compatibility and risks

This optional LSPosed/libxposed APK is separate from the A/B ZIPs and is not installed
on Server. Use it on **A when native StrongBox exists and feature-based application
selection should treat it as unadvertised**. Devices without that declaration
normally do not need it. It is not a general Paytm fix.

The module loads only in `system_server`, hooks
`com.android.server.SystemConfig.getAvailableFeatures()` at startup, and removes
`android.hardware.strongbox_keystore` from its returned map. It preserves
`android.hardware.keystore.app_attest_key`. This is global, including apps outside
Ommega's target list. Package-specific masking is not implemented.

Compatibility requires the ROM to retain the AOSP method, a mutable Map and the
feature publication/cache path, without vendor bypasses or hard-coded answers.
Android 16's application feature cache motivates this early hook; see
[AOSP ActivityManagerService](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android16-release/services/core/java/com/android/server/am/ActivityManagerService.java).
Following AOSP is an implementation prerequisite, not certification or a guarantee
for every AOSP-based ROM. Only the A firmware above has device evidence.

1. Install `StrongBoxCapabilityMask-1.4.6-debug-signed.apk` on A.
2. Enable it in an LSPosed implementation supporting libxposed API 102; keep the
   static scope at `system` (System Framework) only, without application scopes.
3. Reboot and unlock A where normal reboot is permitted, rebuilding the feature
   caches. Closing an application cannot activate or remove this hook.
4. Check `pm has-feature android.hardware.strongbox_keystore` returns false and the
   previously available app-attest-key remains true. Check Mask logs for
   `event=hook_registered` and `event=feature_removed`; a normal test app can also
   verify process-local queries.
5. To undo, disable in LSPosed and reboot A. APKs are debug-signed; check certificates
   before upgrading because different signatures cannot overwrite each other.

**Do not apply these reboot instructions to B when hard reboot is prohibited.
The tested B has no need for Mask.** No system XML is changed, no HAL is stopped,
and no existing key is deleted or migrated. Mask does not change real attestation
Bootloader state or block explicit StrongBox requests; those still follow Android
or Ommega routing. Native attestation may expose an unlocked state. A false feature
query is not proof of hiding it or passing integrity checks.

Apps may choose TEE/other implementations, refuse to run, lose features, change
store availability, or detect inconsistent capability declarations. Existing keys
are not converted; whether apps keep using them is application-dependent. System
hooks can cause crashes or boot problems. **The project and Mask are provided as is,
without guarantees of fitness, security or detection results. You assume all risks.
The authors accept no liability for data loss, device problems, account restrictions,
business or financial loss, to the extent permitted by applicable law. Do not use
them if you do not accept these risks.**

## Main differences from upstream

### 1. A/B KeyMint identity

B supplies a profile with default HAL Stable AIDL version/hash, hardware information,
security level and StrongBox availability. A obtains and freezes it before starting
the TA, retrying while networking is unavailable. Later identities must match.
Server accepts an empty `keymint_author`, but rejects missing/non-string/whitespace-only
values. A checks profile and leaf challenge, App ID, version and security level.
Full trusted-root, chain-signature and revocation verification is unfinished.
`fallback_local` only permits eligible remote-unavailable fallbacks; identity errors fail.

### 2. Native A StrongBox and RKP

`config.toml [main].use_native_strongbox=true` selects A's native StrongBox HAL and
Android RKP `IRemotelyProvisionedComponent/strongbox`; TEE can still use B. Actual
hardware levels are checked. `ATTESTATION_KEYS_NOT_PROVISIONED` does not mean absent
hardware. Apply backend changes with `touch /data/adb/ommega/restart.keymint`,
which restarts only Ommega's keymint child.

### 3. Application security policy

`target.txt` contains plain package names. WebUI provides:

| Policy | Behavior |
|---|---|
| Global default | Follows capability and the global native-StrongBox disable switch |
| StrongBox | Explicit StrongBox path, overriding global disable |
| TEE | Rewrites security-level requests, including explicit StrongBox, to TEE |

Policies reside in `/data/misc/keystore/ommega/target-security.toml`. The disable
switch defaults off and affects only GlobalDefault targets. Old
`strongbox_unavailable_packages` is only migrated when the new policy file is absent;
`!`/`?` suffixes are no longer the format. Mask changes advertisement, not routing.
An explicit StrongBox request with GlobalDefault plus disable returns
`HARDWARE_TYPE_UNAVAILABLE`; explicit TEE rewrites it.

### 4. Live A configuration

Targets, policies and the disable switch are read live. Connection, Token, TLS,
logging and ordinary routing changes need no device reboot. WebUI synchronizes
Hash, Boot Key and SPL into `[trust]`. SPL updates the running TA; Hash/Key recycle
only Ommega keymint and wait for RPC recovery. UTF-8 Base64 protects config writes
from shell heredoc termination. Module upgrades preserve saved WebUI trust parameters.

### 5. Cold boot and authentication

Binder hooks install before RPC readiness; background `ommega-rpc-warmup` avoids
blocking them. Unlock, auth-token and maintenance events are mirrored and replayed
in order. User 0 super-key initialization was fixed. Legacy P-521 raw and SEC1 DER
keys are readable with curve/public-key consistency checks. Historical 64-byte
PBKDF2 extract remains only for old data; new data keeps standard HKDF and the
existing P-521 write format.

### 6. B SPL and service lifecycle

WebUI edits System/Boot/Vendor SPL and the OS version in
`/data/adb/ommega/spl.conf`; changes reload
native KeyMint and keystore2. Empty fields choose the first saved baseline, which
may be too low for already-upgraded keyblobs. `service.sh`, not `post-fs-data.sh`,
reapplies SPL and starts relay, cleaning old processes by exact path. Properties
alone do not prove hardware acceptance; check fresh attestation through a safe,
established application path. SPL affects credential encryption: verify unlocked
state, no in-flight TEE calls and a viable recovery path. Never combine SPL changes
with reboot. B's hard-reboot prohibition remains; soft restart is not risk-free.
If root is lost after a restart the device falls back to its native SPL, while upgraded
keyblobs do not revert with it: treat such a device as disposable data-wise and see the
B data risk warning at the top of this file.
The OS version writes `ro.build.version.release`, which decides OS_VERSION
(tag 705) in the attestation certificate. Like SPL it comes from a system
property rather than the hardware, and `ro.build.version.sdk` is left unchanged,
so the two values can disagree under a strict verifier.

### 7. Relay and Server

B WebUI edits connection parameters and four logging settings. Connections hot-reload;
logs change on the next start. Relay saves do not restart processes or apply SPL.
On `KEY_REQUIRES_UPGRADE`, persistent B blobs get one `upgradeKey` retry on their
original HAL, with atomic durable storage. Failed persistence blocks operation and
keeps the upgraded blob in memory for another save; this is not power-loss protection.
Alias, chain, algorithm and HAL ownership are retained.

Polling and TEE work are separate. Queue fixes cover waiter races, timeouts, repeated
request IDs and disconnected tasks. Server checks result structure, nonempty chain
fields, profiles and ownership, not certificate cryptography. Hardware service-specific
errors may carry integer `keymint_error_code` beside `error`; Server keeps HTTP 500,
A restores recognized negative codes. Unknown/old errors and HTTP auth/rate-limit
errors stay generic; error strings do not infer HAL errors or enable new fallback.

A StrongBox request is never silently downgraded to TEE on either side: A rejects
the relay's downgrade marker with `HARDWARE_TYPE_UNAVAILABLE` and Server forwards
the B-side capability error unchanged. This differs from A's native backend,
disable switch, per-app policies and Mask.

### 8. CI and versions

CI builds A, B, Linux musl Server and APKs in parallel, with A/B workspace, Server,
B ELF, WebUI, version-sync and Mask unit checks. Rust is pinned to
`nightly-2026-09-01`, with committed Cargo.lock and component Rust/Gradle caches.
Version is **1.4.6**, with no automatic bump commits; manual `release_version`
only overrides that build. Currently only Markdown-only changes are ignored; TXT
changes trigger CI. Artifacts include both ZIPs, Server, B-app, Mask,
`SHA256SUMS` and `build-info.json`. Both APKs use a cached Android debug signing
identity, not a formal release key. Its private key is not distributed; cache
eviction can change the signing identity.

## Components and remote-only boundaries

| Component | Role / artifact |
|---|---|
| A (`a-side`) | Target-app interception, software TA, remote TEE, optional native StrongBox; root module |
| B (`b-side`) | Real hardware `profile/attest/sign/decrypt/agree`; root module |
| B-app (`b-app`) | Optional Android client, not a native-B replacement |
| StrongBoxCapabilityMask | Global feature hook, libxposed API 102 APK |
| Server (`server`) | Authentication, scheduling and administration; Linux x86_64 musl |

Remote-only forbids implicit fallback for eligible generation, not every local
operation. Remote generation needs remote enabled, an attestation challenge and
no caller-provided attestation key. Other generation may remain local.
B handles remote signing/decryption/agreement; A stores state and enforces user
authentication. B translates auth requirements to `NO_AUTH_REQUIRED` and
`ATTEST_KEY` to `SIGN`: B hardware does not authenticate A's user token.
B-app supports only `attest/sign/decrypt`, lacks `profile/agree`, and uses its own
AndroidKeyStore identity. It cannot replace native B in the same acceptance suite.

## Deployment and configuration

Use your own Server and credentials; no public Token is distributed. The
[English guide](ommega-remote-key-guide.en.txt) provides complete setup examples.

| File / setting | Activation |
|---|---|
| A `target.txt`, `target-security.toml`, `/data/adb/ommega/config` | Live reads |
| A `webui-props.sh` / `config.toml [trust]` | Live SPL; Hash/Key may recycle Ommega keymint |
| A `[main].use_native_strongbox` | Restart Ommega keymint |
| B `relay.conf` connections | Hot reload; optional `touch /data/adb/ommega/restart.all` |
| B `relay.conf` logging | Next relay start; marker does not reinitialize logging |
| B `spl.conf` | SPL and OS version; applies on save, reapplied by `service.sh` |

Use matching A/B device IDs and appropriate role Tokens. Unselected apps keep
original Keystore routing. A `tls_insecure=true` skips TLS verification; B currently
accepts invalid certificates. HTTPS alone does not authenticate Server identity.
Before changing B `spl.conf`, read the **B data risk warning** at the top of this file:
keep the B device free of personal data and back it up off-device.

The mode dialog of a long-pressed app also offers a **Detector compat** switch: keys that the
selected package creates through the A-side Keystore implementation are allocated positive key
ids only (written to `/data/misc/keystore/ommega/target-compat.toml` when the WebUI saves, empty
by default). Leaving it unchecked keeps the stock random 64 bit ids, about half of which are
negative. The switch only changes the key id range; it changes no other behaviour and provides no
hardware security.

Server reads working-directory `.env`; HTTP defaults to 10886, HTTPS to 8443.
Missing certificates fall back to HTTP. Endpoints: `/api/health/`, `/status/`,
`/jiyin004/` (admin), `/login/`. Updates require temporary upload, SHA verification,
backup, service stop, atomic replacement, start and health/reconnection checks.
Never overwrite a running executable.

## Build

Run each block independently from the repository root with dependencies installed:

```sh
cd a-side/source
python build.py
```
```sh
cd b-side/source
python build.py
```
```sh
cd server/source
cargo build --locked --release
```
```sh
cd b-app/source
./gradlew assembleRelease
```
```sh
cd StrongBoxCapabilityMask
./gradlew :app:testDebugUnitTest :app:assembleRelease
```

Complete distributions: [Build workflow](https://github.com/Andrea-lyz/ommega/actions/workflows/build.yml).

## Validation limits

1.4.6 turns the error-E detector compatibility into an explicit per-package switch: A-side KeyMint keeps the stock key-id shape by default (random 64 bit ids, about half of them negative) and allocates positive ids only for packages listed in `target-compat.toml` when they create keys through the on-device Keystore implementation; the policy is hot-reloaded on mtime, and a missing, empty or malformed file disables it entirely at no extra cost. The WebUI exposes it as a Detector compat checkbox in the mode dialog of a long-pressed app and writes the file atomically on save. Compatibility affects only keys created afterwards; existing negative-id keys stay unchanged. The B module, the Server binary and both APKs differ from 1.4.5 by the version string only.
1.4.5 fixes two A-side issues: KeyMint now charges every HAL entry that reaches the software TA a secure-world round trip (9-21 ms for begin/update/finish/abort/getKeyCharacteristics, 6-16 ms for key generation, 1-4 ms for control entries), so intercepted timing is no longer visibly faster than hardware (re-verified on the A device, where the criterion no longer fires; measured T_triv 16.4-19.3 ms against the 11.93-25.00 ms hardware reference); and the remote `debug_logging` option, which was parsed and persisted but had no consumer, now records `event=remote_attest_chain` (certificate count, tail bytes, and each certificate's length, signature OID and SPKI OID). The B module, the Server binary and both APKs differ from 1.4.2 by the version string only.
1.4.2 aligns the A-side client-facing operation contract with AOSP: overlapping calls on the same operation return `OPERATION_BUSY` at the injector boundary instead of depending on backend queueing. Verified on A on 2026-09-13; B is unchanged.
The first six batches covered remote profile, attestation/signing/decryption,
P-256/P-384/P-521/X25519 agreement and authentication keys across A reboot.
NoPadding/SHA-224/NONE/OAEP MGF had targeted tests, not exhaustive algorithm coverage.
Batch six passed 516 A Android workspace tests plus normal-app remote checks.
B SPL and native A StrongBox/RKP were separate historical windows. CI success,
Server deployment and final-package app acceptance cannot substitute for each other.
Wrapped import is outside accepted support; full certificate trust-chain verification
and other unfinished work remain limited. No universal device, firmware, app,
algorithm or network compatibility is promised.

## Attribution and licenses

- Upstream: [jiyin004-jpg/ommega](https://github.com/jiyin004-jpg/ommega), original author jiyin004.
- Fork: [Andrea-lyz/ommega](https://github.com/Andrea-lyz/ommega), maintenance and 1.4.x work by Andrea-lyz.
- References: [Tricky Store](https://github.com/5ec1cff/TrickyStore) by 5ec1cff;
  [Tricky Addon](https://github.com/KOWX712/Tricky-Addon-Update-Target-List) by KOWX712;
  [OhMyKeymint](https://github.com/qwq233/OhMyKeymint) by James Clef (qwq233);
  [KeyAttestation](https://github.com/vvb2060/KeyAttestation) by vvb2060;
  [TEESimulator-RS](https://github.com/Enginex0/TEESimulator-RS) by Enginex0; AOSP.

Existing AGPL-3.0-or-later, Apache-2.0 and other component licenses still apply.
This documentation does not change upstream or third-party ownership or licenses.
