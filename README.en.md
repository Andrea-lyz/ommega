# Ommega — Andrea-lyz Fork

[![中文](https://img.shields.io/badge/语言-中文-lightgrey)](README.md)
[![English](https://img.shields.io/badge/Language-English-blue)](README.en.md)
[![中文使用说明](https://img.shields.io/badge/使用说明-中文-lightgrey)](ommega远程转发密钥说明.txt)
[![English guide](https://img.shields.io/badge/Guide-English-blue)](ommega-remote-key-guide.en.txt)

> [!IMPORTANT]
> Community fork of [jiyin004-jpg/ommega](https://github.com/jiyin004-jpg/ommega),
> not an official upstream release or hosted service. Product version: **2.1.2**.
> Module authors: `jiyin004, Andrea-lyz`.

> [!WARNING]
> **Run B on a spare device that holds no personal data, and keep an off-device backup.**
> The B module rewrites the System/Boot/Vendor security patch level (SPL) and reloads
> native KeyMint. That touches credential encryption and keyblob upgrade: once a local
> keyblob is upgraded under the higher SPL, it may require that higher value from then on.
> If root disappears after a restart (power loss, a wrong step, or the module not loading
> at boot), the SPL override is not reapplied, and the device can end up with **a correct
> password that no longer unlocks and credential-encrypted data that is unavailable**,
> with no guarantee of recovery without data loss.
> Treat the B device as disposable: keep no primary or irreplaceable data on it, and do
> not assume root will be available. The current boot-time reapply and script guards do
> not guarantee native unlock once root is lost.

Ommega connects A, Server and B for remote KeyMint operations. A intercepts Android
Keystore requests for selected applications. Eligible generation and subsequent
remote signing, decryption and agreement use B hardware through Server.
A also provides the software TA, local storage and application authentication checks.
This fork targets a dedicated **one-A / one-B** deployment; adapting one B for
multiple A devices is outside its maintenance target.

## 2.1.2 downloads and interface

[Download and bilingual release notes](https://github.com/Andrea-lyz/ommega/releases/tag/ommega_fix-2.1.2) ·
[Release CI](https://github.com/Andrea-lyz/ommega/actions/workflows/build.yml)

| Release asset | Purpose |
|---|---|
| `ommega-a-release-arm64-v8a-2.1.2-<commit>.zip` | A root module, including WebUI |
| `ommegaclient-b-release-arm64-v8a-2.1.2-<commit>.zip` | Native B relay module, including WebUI and SPL controls |
| `relay_rs-linux-x86_64-musl` | Self-hosted Linux Server; updated separately |
| `Ommega-B-app-2.1.2-debug-signed.apk` | Optional B client, separate from both A and the native B module |
| `StrongBoxCapabilityMask-2.1.2-debug-signed.apk` | Optional A-side LSPosed module |
| `SHA256SUMS` / `build-info.json` | Download checksums, version, source commit and signing information |

GitHub's generated Source code archives are not installable modules. Both APKs
are debug-signed; compare certificates before upgrading. Updating A/B module ZIPs
does not update the running Server or install optional APKs.

### What is new in 2.1.2

- Fixes most Apps rows showing raw package ids instead of application names. The manager exposes
  its package bridge as an injected Android WebView object, and a method only works when it is called
  through that object. The old code took the method off the object and called it later, so WebView
  rejected every batch with "Java bridge method can't be invoked on a non-injected object"; only the
  packaged `applist.json` names survived. Batched and per-package lookups now keep the live `ksu`
  receiver, and the Apps page shows real names again.
- The browser preview bridge rejects detached calls the same way, so the smoke tests and the new
  regression case run under that rule instead of letting this class of bug reach a device.
- Only the A-side WebUI changed; B, Server and both optional APKs differ from 2.1.1 by the version
  string only.

### What is new in 2.1.1

- Fixes the A-side Apps search field that needed two taps before the keyboard appeared on a cold
  start and could not accept Chinese candidates. Android WebView does not establish a text-input
  connection for an input parsed with the document (the system reports `TYPE_NULL`), so committed
  Chinese text was dropped while key-based English still worked. The field is now recreated when the
  Apps view opens, without taking focus, so one tap raises the keyboard and Chinese input works.
- The Apps search is an independent native input with its own controller: it no longer reuses the
  old Material field's events, styles or clear behaviour, list refreshes never read or write editor
  text, filtering waits for IME composition, and closing the app menu no longer pulls focus back to
  its button.
- Apart from search and this focus fix, every component differs from 2.1.0 by the version string only.

- Both WebUIs now share a Material You design system: tonal colors, rounded surfaces, status
  cards, bottom navigation/side rail and common form controls across Overview, Apps, Settings and
  Connection, including nested dialogs. Simplified Chinese / English and light / dark themes are
  retained, with responsive layouts from 320 to 1024 px.
- A's Apps page is now the system's installed-app list, re-enumerated at startup and on manual
  refresh instead of being filtered by `target.txt`. Each app opens a policy page for individual
  scope, takeover mode (global default / StrongBox / TEE), authentication signing (Auto / Payment
  compatibility / Preserve) and CPU/device properties.
- Scope writes now come only from one app's own switch: enabling or disabling its scope edits
  `target.txt`; there is no bulk write, "show all" filter or uninstalled-app cleanup. Refreshing
  after an uninstall removes the row but keeps the file entry. Global scope ignores `target.txt`
  and pauses individual scope switches, while other per-app policies remain available.
- A adds per-app certificate counts and the latest B-side latency. A count is recorded only after
  the certificate chain and key storage succeed, in the existing `keymaster.db`; shared UIDs are
  marked, and failed calls or internal retries are not counted.
- A adds a system-capability mask dialog for independently hiding StrongBox and device-ID
  attestation capability while retaining `app_attest_key`. CPU/device properties continue to use
  exp6's Zygisk view with values supplied by B and no manual value editing. Authentication-signing
  mode is saved per app on its policy page; positive-key-ID compatibility remains a separate global
  switch.
- B's WebUI is rebuilt and reads SPL/OS state read-only; connection drafts, token masking, language
  and theme behavior are retained. B adds read-only battery telemetry to its existing authenticated
  poll, and Server exposes `battery_percent`, `battery_charging` and `battery_updated_at_ms` on its
  status response. Missing, invalid or stale values remain unknown; zero and last-known values are
  never substituted.
- Server validates battery fields independently and updates queue metadata while remaining
  compatible with older B relays and servers; protocol, bounds/malformed-state tests and
  `server/source/README.md` are included. A/B top refresh now awaits the complete read, exposes a
  busy state, queues one explicit follow-up, and releases the action after failure.
- Added shared UI sources, a design preview and offline browser smoke coverage for A/B locales and
  themes, 320–1024 px layouts, the app list/scope flow, refresh, battery and capability dialogs,
  plus WebUI bundle-patch and release-package verification scripts.
- Upgrade A/B modules and Server separately. Restart A after installing so Zygisk loads the new
  library. Existing `target.txt` entries are retained and can be toggled per app in the Apps page.

### 2.0.0 highlights (previous release)

- The authentication signing policy decides per app whether an auth-bound key stays local.
  Global scope covers ordinary apps only (app ids 10000-19999 per user), and engineering mode
  requires its matching Binder SELinux context and exact process name. The policy and global
  scope live in Settings; the Apps page and Overview no longer repeat those cards.
- SOTER keeps ASK, AuthKey and sign sessions on one backend. A `local_only` choice is written
  to the ledger and survives reloads, and sessions use an opaque handle assigned by A. Network
  and remote authentication errors never turn into a local signature.
- A valid grant brings an isolated process into the same Ommega caller scope. The delegation
  binds the real UID, anchor PID, start time and boot ID, and key access still enforces the
  grant `GET_INFO` / `USE` vector. Revocation, process exit or UID reuse ends it.
- Also: App AttestKey returns only the delegated leaf, local auth-key signing, GMS auth
  windows, system_server grant probing, nested settings dialogs and back gestures, and faster
  CI caching and toolchain setup.

The WebUIs offer simplified Chinese and English, system-aware language selection,
and light/dark themes. OpenUI is used for development previews; device UIs use
bundled local resources without online UI generation.

| Side | View | Contents |
|---|---|---|
| A | Overview | Last relay result, KeyMint process, SOTER software TA and ledger status |
| A | Apps | Installed-app list with per-app scope, takeover mode, authentication signing and CPU/device properties |
| A | Settings | Remote configuration, SOTER, automation, attestation parameters, system capability mask, language, theme, help and about |
| B | Overview | Local relay process, connection evidence, current properties and B battery |
| B | Connection | Server, device ID, token and logging |
| B | Settings | Read-only SPL / OS status, language and theme |

A/B refresh reloads status files; **it neither runs hardware attestation nor clears
failure history**. A read exposes a busy state, queues one explicit follow-up and
releases the action after failure. The last request result has a timestamp and is
not a live connectivity probe. A running B process alone does not prove a Server
connection. 2.1.0 also removes the Apps save button: scope and policy switches save
directly and verify the readback.

The TXT guides linked above retain older setup examples. Current configuration
paths, navigation and SPL behavior are defined by this README and current source.

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
`android.hardware.strongbox_keystore` and `android.software.device_id_attestation`
from its returned map. It preserves `android.hardware.keystore.app_attest_key`.
This is global, including apps outside Ommega's target list. Package-specific
masking is not implemented.

Masking device ID attestation is deliberate: an Ommega-routed caller's chain is
minted by the remote real KeyMint. The tested B TA rejects `ATTESTATION_ID_*` tags,
and injecting either the remote device's identifiers or local substitutes into that
chain would break the caller's own integrity verdicts. No usable ID attestation
was available on the tested B path, so the declaration matches that limitation instead of letting every
identifier request fail. The change only alters feature declarations; it touches no
certificate, key or route. Masking alone does not guarantee an application's
integrity verdict.

Compatibility requires the ROM to retain the AOSP method, a mutable Map and the
feature publication/cache path, without vendor bypasses or hard-coded answers.
Android 16's application feature cache motivates this early hook; see
[AOSP ActivityManagerService](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android16-release/services/core/java/com/android/server/am/ActivityManagerService.java).
Following AOSP is an implementation prerequisite, not certification or a guarantee
for every AOSP-based ROM. Only the A firmware above has device evidence.

1. Install `StrongBoxCapabilityMask-1.9.0-debug-signed.apk` on A.
2. Enable it in an LSPosed implementation supporting libxposed API 102; keep the
   static scope at `system` (System Framework) only, without application scopes.
3. Reboot and unlock A where normal reboot is permitted, rebuilding the feature
   caches. Closing an application cannot activate or remove this hook.
4. Check `pm has-feature android.hardware.strongbox_keystore` and
   `pm has-feature android.software.device_id_attestation` both return false and
   the previously available app-attest-key remains true. Check Mask logs for
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

## Main features and behavior

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
logging and ordinary routing changes need no device reboot.
The remote profile is frozen at keymint startup: selecting a different B identity
requires restarting A's Ommega keymint to obtain its profile. Hot reload does not
bypass identity checks. WebUI synchronizes
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
native KeyMint and keystore2. **Values only increase; empty fields keep the current
effective value.** The minimum is the highest of the original device value,
current reported value and recorded applied target. `service.sh`, not `post-fs-data.sh`,
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

`spl-origin.conf` records originals from read-only firmware property files;
`spl-baseline.conf` is a runtime snapshot and may already contain overrides.
**Preserve `/data/adb/ommega/spl-applied.conf` across upgrades and rollbacks**:
it records applied targets for reapplication after property fallback. That does
not guarantee unlock if root is lost or credentials are needed before reapplication.
The interface shows originals, minimums and applied targets separately
(`ORIGIN_*` / `FLOOR_*` / `APPLIED_*`). Firmware updates can raise originals;
temporarily unreadable files do not lower recorded values.

Keep A's real OS/SPL and `[trust]` on `auto`; do not lower A to match an older B.
Historical reductions caused `INVALID_ARGUMENT` (-38) and Wallet card-binding
failures. Passing Integrity checks does not replace application/card testing.

### 7. Relay and Server

B WebUI edits connection parameters and four logging settings. Connections hot-reload;
logs change on the next start. Relay saves do not restart processes or apply SPL.
On `KEY_REQUIRES_UPGRADE`, persistent B blobs get one `upgradeKey` retry on their
original HAL, with atomic durable storage. Failed persistence blocks operation and
keeps the upgraded blob in memory for another save; this is not power-loss protection.
Alias, chain, algorithm and HAL ownership are retained.

Short polling and TEE work are separate, with an 80ms polling floor and a
three-second per-request budget. Relay reports its product version, supports
physical uplink binding and reselects a disappeared/switched interface.
Queue fixes cover waiter races, timeouts, repeated
request IDs and disconnected tasks. Server checks result structure, nonempty chain
fields, profiles and ownership, not certificate cryptography. Automatic-keybox and
serverbox switches persist independently in `data/runtime-state.json` across
Server restarts. Hardware service-specific
errors may carry integer `keymint_error_code` beside `error`; Server keeps HTTP 500,
A restores recognized negative codes. Unknown/old errors and HTTP auth/rate-limit
errors stay generic; error strings do not infer HAL errors or enable new fallback.

A StrongBox request is never silently downgraded to TEE on either side: A rejects
the relay's downgrade marker with `HARDWARE_TYPE_UNAVAILABLE` and Server forwards
the B-side capability error unchanged. This differs from A's native backend,
disable switch, per-app policies and Mask.

### 8. SOTER and the local ledger

Software SOTER and KeyMint have independent configuration. After enabling the
software TA in Settings, choose the local ledger or remote relay. SOTER remote
parameters live in `/data/adb/ommega/soterta/remote.conf` and do not automatically
follow KeyMint settings. Its local ledger is
`/data/adb/ommega/soterta/state.json`; software keys are not equivalent to real
B-side SOTER hardware keys.

Remote failures do not silently substitute local keys. Software-TA takeover or
restart rebinds the SOTER host, and B serializes SOTER TA calls. 1.9.0 supports
Java/native replies and different transaction-number layouts.

### 9. CI and versions

CI builds A, B, Linux musl Server and APKs in parallel, with A/B workspace, Server,
B ELF, WebUI, version-sync and Mask unit checks. Rust is pinned to
`nightly-2026-09-01`, with committed Cargo.lock and component Rust/Gradle caches.
Version is **2.1.2**, with no automatic bump commits; manual `release_version`
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
| StrongBoxCapabilityMask | Global StrongBox and device-ID-attestation capability mask, libxposed API 102 APK |
| Server (`server`) | Authentication, scheduling and administration; Linux x86_64 musl |

Remote-only forbids implicit fallback for eligible generation, not every local
operation. Remote generation needs remote enabled and an attestation challenge;
the issuer may be absent or a caller-selected remote App AttestKey. A local
caller-selected issuer keeps local generation.
B handles remote signing/decryption/agreement; A stores state and enforces user
authentication with the application's original policy and timeout. B preserves
the requested authorization list, including user SIDs, authentication type,
timeout and `UNLOCKED_DEVICE_REQUIRED` (509). It adds `NO_AUTH_REQUIRED` only
when no authentication policy was specified. B cannot validate A's user token,
so an auth-bound remote hardware operation can still be rejected by B.
Without a caller-provided issuer, A returns B's full certificate chain unchanged,
reporting B's actual authorizations without adding a synthetic child leaf for
auth-bound keys. `ATTEST_KEY` reaches
the HAL unchanged. B checks the returned hardware purpose; A checks the
certificate's hardware ATTEST_KEY authorization. Explicit
delegation uses B's native `generateKey(..., AttestationKey)`. B appends the
parent chain; A verifies the selected parent public key and child signature, then
returns only the new child certificate. The caller appends its existing issuer chain.
Native delegation permissions come from the hardware keyblob. This path does
not impose a generic PKIX CA gate or rewrite certificate extensions.
Both A and B must support delegation. A rejects an old relay that ignores the
issuer and never falls back to another signing key. Remote SIGN cannot issue
certificates.
B-app supports only `attest/sign/decrypt`, lacks `profile/agree`, and uses its own
AndroidKeyStore identity. It cannot replace native B in the same acceptance suite.

## Deployment and configuration

Use your own Server and credentials; no public Token is distributed. The
[English guide](ommega-remote-key-guide.en.txt) provides complete setup examples.

| File / setting | Activation |
|---|---|
| A `target.txt`, `target-security.toml` | Live reads |
| A `target-compat.toml` | Global positive-key-ID switch, hot-reloaded for new keys |
| A `/data/misc/keystore/ommega/config` | WebUI accesses it through `/data/adb/ommega/ommegadata/config`; enabled remote settings are persisted at startup |
| A `/data/adb/ommega/soterta/remote.conf` | Independent SOTER relay settings |
| A `webui-props.sh` / `config.toml [trust]` | Live SPL; Hash/Key may recycle Ommega keymint |
| A `[main].use_native_strongbox` | Restart Ommega keymint |
| B `relay.conf` connections | Hot reload; optional `touch /data/adb/ommega/restart.all` |
| B `relay.conf` logging | Next relay start; marker does not reinitialize logging |
| B `spl.conf` | SPL and OS version; applies on save, reapplied by `service.sh` |
| B `spl-applied.conf` | Script-maintained applied targets; preserve across upgrades/rollbacks |

Use matching A/B device IDs and appropriate role Tokens. Apps outside both
`target.txt` and the injector's `scoop` scope keep original Keystore routing.
A `tls_insecure=true` skips TLS verification; B currently
accepts invalid certificates. HTTPS alone does not authenticate Server identity.
Before changing B `spl.conf`, read the **B data risk warning** at the top of this file:
keep the B device free of personal data and back it up off-device.

**Detector compatibility is global**, not a per-package policy. When enabled,
all apps creating keys through Ommega's local Keystore implementation receive
positive key IDs. `/data/misc/keystore/ommega/target-compat.toml` stores
`[positive_key_id].enabled`, off by default; an old nonempty `packages` list
is read as enabling the global switch. Disabled means the default random 64-bit
IDs remain. Existing keys are not migrated; algorithms and chains do not change,
and this provides no hardware security.

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

2.1.2 passed host WebUI checks, CPU config and property-record tests, Android workspace Clippy
and test compilation, and the [Build workflow](https://github.com/Andrea-lyz/ommega/actions/workflows/build.yml)
builds the release artifacts. These are build and presentation checks. App names were verified on
an Android 17 device: all 396 apps showed real names with no package-id fallback. Device
gestures, network behavior and application flows still require device acceptance.

<details>
<summary>Historical implementation and test records; not 2.1.2 overall device acceptance</summary>

1.6.0 fixes three A-side KeyMint implementation problems that broke applications outright: an ommega-routed call is no longer answered with `SYSTEM_ERROR` while a mirror replay is in flight (the gate now waits, bounded, and stays fail-closed only for poisoned lanes); EC key generation restores AOSP semantics (EC plus ENCRYPT warns instead of failing with `INCOMPATIBLE_PURPOSE`); and RSA and ECDSA select their signature scheme from the digest the key was created for (the PKCS#1/PSS scheme was hardcoded to SHA-256 and ECDSA signed with the curve default digest, so a non-SHA-256 RSA key could not sign at all and a mismatched digest produced a silently wrong signature). The B side refuses SPL and OS version downgrades, and the A side logs `event=local_sign` with the padding and digest of every local signing operation. Verified on devices: HDFC RSA-SHA512 biometric login, Google Pay EC key generation and fingerprint flows interrupted by a lock-state replay all work again; A-side mirror 12/12, tag 12/12 and crypto 7/7 unit tests plus 13 B-side spl-control cases pass and CI is green. The Server differs from 1.5.3 by the version string only; the B module additionally ships the downgrade guard.

1.4.6 turns the error-E detector compatibility into an explicit per-package switch: A-side KeyMint keeps the stock key-id shape by default (random 64 bit ids, about half of them negative) and allocates positive ids only for packages listed in `target-compat.toml` when they create keys through the on-device Keystore implementation; the policy is hot-reloaded on mtime, and a missing, empty or malformed file disables it entirely at no extra cost. The WebUI exposes it as a Detector compat checkbox in the mode dialog of a long-pressed app and writes the file atomically on save. Compatibility affects only keys created afterwards; existing negative-id keys stay unchanged. The B module, the Server binary and both APKs differ from 1.4.5 by the version string only.
1.4.5 fixes two A-side issues: KeyMint now charges every HAL entry that reaches the software TA a secure-world round trip (9-21 ms for begin/update/finish/abort/getKeyCharacteristics, 6-16 ms for key generation, 1-4 ms for control entries), so intercepted timing is no longer visibly faster than hardware (re-verified on the A device, where the criterion no longer fires; measured T_triv 16.4-19.3 ms against the 11.93-25.00 ms hardware reference); and the remote `debug_logging` option, which was parsed and persisted but had no consumer, now records `event=remote_attest_chain` (certificate count, tail bytes, and each certificate's length, signature OID and SPKI OID). The B module, the Server binary and both APKs differ from 1.4.2 by the version string only.
1.4.2 aligns the A-side client-facing operation contract with AOSP: overlapping calls on the same operation return `OPERATION_BUSY` at the injector boundary instead of depending on backend queueing. Verified on A on 2026-09-13; B is unchanged.
The first six batches covered remote profile, attestation/signing/decryption,
P-256/P-384/P-521/X25519 agreement and authentication keys across A reboot.
NoPadding/SHA-224/NONE/OAEP MGF had targeted tests, not exhaustive algorithm coverage.
Batch six passed 516 A Android workspace tests plus normal-app remote checks.
B SPL and native A StrongBox/RKP were separate historical windows. CI success,
Server deployment and final-package app acceptance cannot substitute for each other.

</details>

Wrapped import is outside accepted support; full certificate trust-chain verification
and other unfinished work remain limited. No universal device, firmware, app,
algorithm or network compatibility is promised.

## Remote request latency and screen-off availability

The B relay defaults to `OMMEGA_RELAY_WAKELOCK=true`. It keeps the CPU available
with the screen off and increases battery use. On kernels providing
`CONFIG_PM_WAKELOCKS`, a 90-second lease is renewed every 30 seconds and expires
if the process stops. Disabling it releases only the relay lock within 30
seconds. Missing or denied interfaces produce a warning; screen-off availability
is not guaranteed in that case. Updates add missing configuration keys while
preserving existing values, credentials and device identity.

A-side remote signatures share a 10-second HTTP budget across attempts and
backoff; attestation retains a separate longer budget. The injector uses up to
four connections in one RPC session. Ordinary remote signatures release the TA
lock during network work. Pending authentication, trusted confirmation/presence
and single-use deletion retain the serialized path. Server signature waits are
capped at 10 seconds and cancel only unassigned work, never an in-flight HAL call.

## Attribution and licenses

- Upstream: [jiyin004-jpg/ommega](https://github.com/jiyin004-jpg/ommega), original author jiyin004.
- Fork: [Andrea-lyz/ommega](https://github.com/Andrea-lyz/ommega), maintained by Andrea-lyz.
- References: [Tricky Store](https://github.com/5ec1cff/TrickyStore) by 5ec1cff;
  [Tricky Addon](https://github.com/KOWX712/Tricky-Addon-Update-Target-List) by KOWX712;
  [OhMyKeymint](https://github.com/qwq233/OhMyKeymint) by James Clef (qwq233);
  [KeyAttestation](https://github.com/vvb2060/KeyAttestation) by vvb2060;
  [TEESimulator-RS](https://github.com/Enginex0/TEESimulator-RS) by Enginex0; AOSP.

Existing AGPL-3.0-or-later, Apache-2.0 and other component licenses still apply.
This documentation does not change upstream or third-party ownership or licenses.
