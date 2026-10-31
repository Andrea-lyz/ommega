# Ommega

[![Version](https://img.shields.io/badge/version-2.2.0-blue)](https://github.com/Andrea-lyz/ommega/releases)
[![License](https://img.shields.io/badge/license-AGPL--3.0--or--later-green)](LICENSE)
[![中文](https://img.shields.io/badge/语言-中文-lightgrey)](README.md)
[![English](https://img.shields.io/badge/Language-English-blue)](README.en.md)
[![中文使用说明](https://img.shields.io/badge/使用说明-中文-lightgrey)](ommega远程转发密钥说明.txt)
[![English guide](https://img.shields.io/badge/Guide-English-blue)](ommega-remote-key-guide.en.txt)

> Ommega takes Android Keystore requests from apps on A, schedules them through a
> self-hosted Server, and executes them on the real KeyMint/TEE of the B device.
> Certificate chains, signatures, decryption and agreement are produced by B's
> hardware and returned to the calling app.

> [!IMPORTANT]
> This repository is a community-maintained fork of
> [jiyin004-jpg/ommega](https://github.com/jiyin004-jpg/ommega). It is not an
> official upstream release, and it ships no public Server or public token.
> Current version: **2.2.0**. Module authors: `jiyin004, Andrea-lyz`. The
> maintenance target is a dedicated **one-A / one-B** deployment, not one B
> serving several A devices.

> [!WARNING]
> **Run B on a spare device that holds no personal data, and keep an off-device
> backup.** The B module rewrites the System/Boot/Vendor security patch level
> (SPL) and reloads native KeyMint, which touches credential encryption and
> one-way keyblob upgrades. If root disappears after a restart, the SPL override
> is not reapplied, and the device can end up with **a correct password that no
> longer unlocks and credential-encrypted data that is unavailable**, with no
> guarantee of recovery without data loss. Do not use a primary phone as B, and
> do not assume root will always be available. See
> [Safety and risks](#safety-and-risks).

## What this is

Ommega is a three-part remote KeyMint system.

- **A** is a root module. It intercepts Keystore/KeyMint calls from target apps,
  provides the software TA, local key storage and authentication checks, and in
  remote mode forwards attestation, signing, decryption and agreement to Server.
- **Server** is a self-hosted Linux service. It handles token authentication, the
  task queue, A/B pairing and scheduling, online-device state and the admin UI.
- **B** is a root module. Its native relay polls tasks, calls the device's real
  KeyMint HAL (TEE or StrongBox) and returns hardware-minted chains and results.

This fork maintains remote identity consistency, native StrongBox/RKP, cold-boot
and biometric lifecycle handling, WebUI configuration and B-side service-level
operations. It does not target unrooted devices or non-Android platforms, and it
does not promise compatibility with every ROM, device or algorithm combination.

| Item | Value |
|---|---|
| Current version | **2.2.0** (A / B / Server / both APKs share one version) |
| Deployment | 1 A + 1 B + 1 self-hosted Server |
| A / B | Android 12 or later, arm64-v8a, KernelSU / APatch / Magisk |
| Server | Linux x86_64 (static musl binary) |
| License | A/B: AGPL-3.0-or-later; Server declares MIT; some sources Apache-2.0 |

## How it works

```mermaid
flowchart LR
    App["Target app"] -->|"Keystore / KeyMint Binder"| A["A module<br/>injector · software TA · relay client"]
    A -->|"HTTPS task + token"| S["Server<br/>auth · queue · scheduling"]
    S -->|"poll / result"| B["B module<br/>native relay"]
    B -->|"Stable AIDL"| HAL["Real KeyMint HAL<br/>TEE / StrongBox"]
    B -.->|"result"| S
    S -.->|"result"| A
    A -.->|"chain / signature / plaintext"| App
```

A typical remote request:

1. An app calls Keystore. A's injector decides from scope whether to take over;
   apps outside the scope keep the original system path.
2. A's software TA creates or selects the key. An eligible generation request
   carries the app ID (tag 709) to Server.
3. Server authenticates the token, queues the task, and B's relay polls it.
4. B calls the real HAL for `profile` / `attest` / `sign` / `decrypt` / `agree` /
   `soter` and posts the result back to Server.
5. A validates the remote profile and leaf-certificate fields, stores the key and
   returns to the app. Later signing, decryption and agreement for remote keys
   also run on B; A only stores state and enforces authentication.

`remote-only` means **no implicit fallback** when a request qualifies for remote
generation. It does not mean every operation runs on B:

- Remote generation needs `remote: true`, an attestation challenge, and either
  no caller-selected issuer or a remote App AttestKey. A caller-selected local
  issuer keeps generation on A.
- B preserves the requested authorization list (user SIDs, authentication type,
  timeout, `UNLOCKED_DEVICE_REQUIRED` 509), but it cannot validate A's user token;
  an auth-bound remote hardware operation can still be rejected by B.
- Without a caller-selected issuer, A returns B's full chain unchanged and does
  not add a synthetic child leaf for auth-bound keys.
- A StrongBox request is never silently downgraded to TEE: A rejects the relay's
  downgrade marker with `HARDWARE_TYPE_UNAVAILABLE`, and Server forwards B's
  capability error unchanged.

## Components

| Directory | Role | Form |
|---|---|---|
| [`a-side`](a-side/source/README.md) | Target-app interception; software TA; remote TEE; optional native StrongBox | Root module with WebUI; CI publishes arm64-v8a |
| [`b-side`](b-side/source/README.md) | Real hardware `profile/attest/sign/decrypt/agree/soter` | Root module with WebUI and SPL controls |
| [`server`](server/source/README.md) | Authentication, task queue, A/B scheduling, device and admin management | Linux x86_64 musl binary |
| [`b-app`](b-app/source) | Optional standalone B client and connection UI | Android APK; implements only `attest/sign/decrypt` |
| [`StrongBoxCapabilityMask`](StrongBoxCapabilityMask/README.md) | Globally hides StrongBox and device-ID attestation capability | libxposed API 102 APK, loaded only into `system_server` |
| [`ui`](ui/README.md) | Shared A/B WebUI styles, design previews and offline smoke tests | Development assets bundled into the modules |

**B-app cannot replace native B**: it lacks `profile` / `agree` and creates keys
with its own AndroidKeyStore identity, so it cannot pass the same acceptance
suite.

## Downloads

[Releases](https://github.com/Andrea-lyz/ommega/releases) ·
[Release CI](https://github.com/Andrea-lyz/ommega/actions/workflows/build.yml)

| Release asset | Purpose |
|---|---|
| `ommega-a-release-arm64-v8a-<version>-<commit>.zip` | A root module, including WebUI |
| `ommegaclient-b-release-arm64-v8a-<version>-<commit>.zip` | Native B relay module, including WebUI and SPL controls |
| `relay_rs-linux-x86_64-musl` | Self-hosted Linux Server, updated separately |
| `Ommega-B-app-<version>-debug-signed.apk` | Optional B client; neither the A app nor a native-B replacement |
| `StrongBoxCapabilityMask-<version>-debug-signed.apk` | Optional A-side LSPosed module |
| `SHA256SUMS` / `build-info.json` | Checksums, product version, source commit and signing notes |

GitHub's generated Source code archives are not installable modules. Both APKs are
debug-signed; compare certificates before upgrading. Updating an A/B ZIP does not
update the running Server or install the optional APKs.

Per-version notes live in [CHANGELOG.en.md](CHANGELOG.en.md).

## Quick start

Prerequisites:

- A working Linux x86_64 Server (or build `relay_rs` yourself).
- Two rooted Android 12+ arm64-v8a devices, ideally on a supported KernelSU /
  APatch / Magisk setup.
- Your own token and, preferably, a trusted TLS certificate. The repository
  distributes no public Server or public token.
- **B must be a disposable device with no personal data**, already backed up
  off-device.

### 1. Deploy Server

Put the musl binary in a working directory, create `.env`, then start it:

```sh
mkdir -p /opt/ommega && cd /opt/ommega
cp /path/to/relay_rs-linux-x86_64-musl ./relay_rs
chmod +x relay_rs
```

```ini
# /opt/ommega/.env
RELAY_BIND=0.0.0.0
RELAY_HTTP_PORT=10886
RELAY_HTTPS_PORT=8443
RELAY_USE_TLS=true
RELAY_TLS_CERTFILE=data/tls/server.crt
RELAY_TLS_KEYFILE=data/tls/server.key
RELAY_TOKEN=<random-static-token>
RELAY_ADMIN_USER=admin
RELAY_ADMIN_PASSWORD=<strong-password>
RELAY_ATTEST_SOURCE=physical
# RELAY_MYSQL_URL=mysql://user:password@127.0.0.1:3306/ommega
# RELAY_SECRET_KEY=<random-secret>
```

```sh
./relay_rs
```

Run it under systemd or an equivalent supervisor in production, and make sure the
working directory is the one holding `.env` and `data/`. Defaults and endpoints:

| Path | Purpose |
|---|---|
| `10886` | HTTP (default) |
| `8443` | HTTPS; missing certificates fall back to HTTP only |
| `/api/health/` | Health check |
| `/status/` | Public device status, including online B devices and battery telemetry |
| `/jiyin004/` | Admin console |
| `/login/` | Login page |

With MySQL enabled you can issue separate role tokens for A and B from the admin
console. To update `relay_rs`, upload a temporary file, verify its SHA-256, keep a
backup, stop the service, replace the file atomically, start it and check health.
Never overwrite a running executable.

### 2. Install B

Install `ommegaclient-b-*.zip`, then fill in the module WebUI's **Connection**
page or edit `/data/adb/ommega/relay.conf` directly:

```ini
OMMEGA_RELAY_SERVER=https://<server>:8443
OMMEGA_RELAY_DEVICE_ID=<b-device-id>
OMMEGA_RELAY_MACHINE_ID=
OMMEGA_RELAY_TOKEN=<b-token-or-static-token>
OMMEGA_RELAY_LOG_ENABLED=true
OMMEGA_RELAY_LOG_LEVEL=debug
OMMEGA_RELAY_LOGCAT_ENABLED=true
OMMEGA_RELAY_LOGCAT_LEVEL=info
```

- `SERVER`, `DEVICE_ID` and `TOKEN` are required; `MACHINE_ID` is optional.
- Connection settings hot-reload; logging settings take effect on the next relay
  start.
- To force a reread, run `touch /data/adb/ommega/restart.all`. The marker only
  asks for a config reread; it does not restart the relay or the device, and it
  does not reinitialize logging.
- SPL / OS values are edited and applied in the WebUI under **Settings → Device
  properties**, and stored in `/data/adb/ommega/spl.conf`. Dates and the OS version
  only move forward; **keep `spl-applied.conf` across module upgrades and
  rollbacks**.
- **Never hard-reboot B.** When a reload is required, use only the approved
  service-level `service.sh` flow.

The full SPL rules, floor calculation and risks are in the
[B-side documentation](b-side/source/README.md).

### 3. Install A

Install `ommega-a-*.zip`, then use **Settings → Remote configuration** in the
WebUI. The file is `/data/misc/keystore/ommega/config`; the WebUI accesses it
through `/data/adb/ommega/ommegadata/config`:

```yaml
url: https://<server>:8443
device_id: <b-device-id>
token: <a-token-or-static-token>
remote: true
local_hw: false
tls_insecure: true
debug_logging: false
disable_native_strongbox: false
```

- `device_id` must match B's `OMMEGA_RELAY_DEVICE_ID`; `token` is the role token
  issued by Server or the static token.
- `tls_insecure: true` skips A's certificate verification. Tighten this after
  deploying a trusted certificate. B currently accepts self-signed certificates,
  so **an HTTPS URL alone does not prove Server identity**.
- On the **Apps** page, enable an app's scope and choose its takeover mode
  (global default / StrongBox / TEE). The effective target set is the union of
  `target.txt` and `scoop` in `injector.toml`; apps in neither keep the original
  system Keystore path.
- Overlay-installing the module does not clear saved WebUI trust parameters
  (hash, boot key, SPL).

### 4. Check the link

1. B's WebUI **Overview** should show the relay running, "Server connected" and
   the last poll time.
2. Server `/status/` should list that B as online; the admin console shows tasks
   and device state.
3. A's **Overview** shows the last relay result. Start with a test app that
   creates and signs a key, then move to production apps.
4. Refresh buttons only reread status files; **they do not run hardware
   attestation**. The last result has a timestamp and is not a live connectivity
   probe, and a running B process alone does not prove a Server connection.

## Configuration

| File / switch | Purpose | Activation |
|---|---|---|
| A `target.txt` | Select apps to take over | Read live by the injector |
| A `target-security.toml` | Per-app security level (global default / StrongBox / TEE) | Read live by the injector |
| A `target-compat.toml` | Global positive-key-ID compatibility switch | Hot-reloaded; affects new keys only |
| A `config` (`/data/misc/keystore/ommega/`) | Remote connection and global StrongBox policy | Read on save; enabled remote settings persist at startup |
| A `soterta/remote.conf` (`/data/adb/ommega/`) | Independent SOTER relay settings | Maintained separately from KeyMint |
| A `webui-props.sh` / `config.toml [trust]` | Verified boot hash, boot key, SPL | Live; may recycle only Ommega keymint |
| A `config.toml [main].use_native_strongbox` | StrongBox backend selection | `touch /data/adb/ommega/restart.keymint` |
| B `relay.conf` connection keys | Server, device ID, machine ID, token | Hot-reloaded after a WebUI save |
| B `relay.conf` logging keys | File log and logcat switches/levels | Next relay start |
| B `spl.conf` | System/Boot/Vendor SPL and OS version | Applied on save; reapplied by `service.sh` |
| B `spl-applied.conf` | Applied targets and downgrade floor | Script-maintained; preserve across upgrades/rollbacks |

## WebUI

The A/B WebUIs offer simplified Chinese and English, follow the system language
and support light/dark themes. Device UIs use bundled local resources with no
online generation; the OpenUI preview in `ui/` is for development only.

| Side | View | Contents |
|---|---|---|
| A | Overview | Last relay result, KeyMint process, SOTER software TA and ledger state |
| A | Apps | Installed-app list; per-app scope, takeover mode, authentication signing, CPU/device properties |
| A | Settings | Remote configuration, SOTER, automation, attestation parameters, system capability mask, language, theme, help and about |
| B | Overview | Relay process, connection evidence, current properties and battery |
| B | Connection | Server, device ID, token and logging |
| B | Settings | Device properties (SPL / OS), language and theme |

The two TXT guides linked at the top retain older setup examples. Configuration
paths, navigation and SPL behavior are defined by this README,
[CHANGELOG.en.md](CHANGELOG.en.md) and the current source.

## Capabilities and boundaries

- **KeyMint identity alignment**: B's `profile` task reads the real default HAL's
  Stable AIDL version/hash, `getHardwareInfo()`, security level and StrongBox
  availability. A obtains and freezes it before starting the software TA, and
  every later attestation must carry the same profile. Boot-time network failures
  are retried instead of freezing a wrong identity. A checks the leaf challenge,
  app ID, version and security level; **full trusted-root, chain-signature and
  revocation verification is not implemented**.
- **Native StrongBox and RKP**: with `use_native_strongbox = true`, A uses the
  local StrongBox HAL and obtains attestation keys through
  `IRemotelyProvisionedComponent/strongbox`; the actual returned security level is
  checked. Switching the backend only restarts Ommega's keymint child, not the
  device.
- **Per-app security policy**: `Global default` follows device capability and the
  global native-StrongBox disable switch; `StrongBox` overrides that switch; `TEE`
  rewrites the app's requests to TEE. Policies live in `target-security.toml` and
  are read live.
- **Cold boot and biometrics**: Binder hooks install before remote RPC is ready;
  `ommega-rpc-warmup` initializes RPC in the background; `OnDeviceUnlocked`, auth
  tokens and user-maintenance events are queued in memory and replayed in order
  after RPC recovers.
- **B SPL and service operations**: SPL and the OS version come from system
  properties, not hardware; changing them reloads native KeyMint and `keystore2`.
  Dates only move forward, and a keyblob upgraded under a higher value may fail
  under a lower one. B ships only `service.sh`, not `post-fs-data.sh`.
- **Relay and Server stability**: short polling is separate from TEE workers, with
  an 80 ms polling floor and a 3-second per-request budget. Since 2.1.3,
  `b/poll` has its own `RELAY_POLL_RATE_LIMIT_REQUESTS` budget and no longer
  consumes the A-task / `b/result` work quota.
- **Battery telemetry**: B reads only `/sys/class/power_supply/battery`, samples
  at most every five seconds and reports on the existing authenticated poll.
  Server exposes `battery_percent`, `battery_charging` and
  `battery_updated_at_ms` on `/status/`; missing, invalid or stale data stays
  unknown instead of defaulting to 0.
- **SOTER**: the software SOTER TA is configured independently from KeyMint.
  Remote settings live in `soterta/remote.conf` and do not follow KeyMint
  changes. A remote failure never silently switches to local keys.
- **Timeouts and screen-off**: A's remote signatures share a 10-second HTTP
  budget across retries and backoff, while attestation keeps a separate longer
  budget. B's `OMMEGA_RELAY_WAKELOCK=true` keeps the CPU available with the
  screen off, renewing a 90-second lease every 30 seconds; it increases battery
  use and needs kernel `CONFIG_PM_WAKELOCKS` support.

## Build

Run each block from the repository root after entering the directory; CI pins the
verified `nightly-2026-09-01` toolchain.

```sh
# A module
cd a-side/source
python build.py
```

```sh
# B module
cd b-side/source
python build.py
```

```sh
# Server
cd server/source
cargo build --locked --release
```

```sh
# B app
cd b-app/source
./gradlew assembleRelease
```

```sh
# StrongBoxCapabilityMask
cd StrongBoxCapabilityMask
./gradlew :app:testDebugUnitTest :app:assembleRelease
```

For full distributions use the
[Build workflow](https://github.com/Andrea-lyz/ommega/actions/workflows/build.yml).
CI ignores Markdown-only changes (TXT changes still build), uses the source
version without bump commits, and lets a manual `release_version` override only
that build.

## Compatibility and validation

The following is a **2026-09-05 test snapshot**, not a supported-device list, and
later releases do not inherit it automatically.

| Item | A | B |
|---|---|---|
| Model | OnePlus 13, PJZ110 | OnePlus 11, PHB110 |
| Android / API / ABI | Android 16 / API 36 / arm64-v8a | Android 16 / API 36 / arm64-v8a |
| Firmware | `PJZ110_16.0.10.501(CN01)` | `PHB110_16.0.2.400(CN01)` |
| Validated hardware path | Native StrongBox + RKP tested separately; disabled for remote acceptance | Default TEE KeyMint through native relay |
| StrongBox | Native implementation; feature query false after Mask | Query false on that firmware; B StrongBox unvalidated |
| `app_attest_key` | true; retained by Mask | true |

- Remote acceptance used A `remote=true`, `local_hw=false`,
  `disable_native_strongbox=true`, `use_native_strongbox=false`, GlobalDefault
  policies and Mask enabled. It exercised **A → Server → B TEE**, not StrongBox
  on both devices.
- Mask was validated with LSPosed 2.2.0 / libxposed API 102. A and B run rooted;
  this does not establish compatibility with every root manager, ROM or device.
- The API 29 floor on A's installer and Mask is an installation threshold only;
  actual operation depends on keystore2, KeyMint AIDL and injector/firmware
  compatibility.
- CI publishes arm64-v8a modules. Other architectures, Android versions and B
  StrongBox require separate validation.
- Release assets are built by the
  [Build workflow](https://github.com/Andrea-lyz/ommega/actions/workflows/build.yml);
  CI runs host WebUI checks, Android workspace Clippy and test compilation.
  Per-version validation records are in [CHANGELOG.en.md](CHANGELOG.en.md); overall
  application flows still require installing the new build for acceptance.

Integrity three-green results, Paytm reaching the phone-number page and
intermittent `00000` belong to those historical test windows and do not guarantee
payments, card binding or any risk-control outcome.

## Safety and risks

- **B data**: SPL changes affect credential encryption. A keyblob upgraded under a
  higher SPL may require that value permanently; if root is lost the device
  returns to its native SPL while upgraded keyblobs do not revert. Treat B as a
  data-disposable device and keep an off-device backup. Boot-time reapply and
  script guards do not guarantee native unlock once root is lost.
- **SPL only moves up**: each floor is the highest of the device original, the
  currently reported value and the applied target. Lowering a reported value has
  produced `INVALID_ARGUMENT` (-38), and lowering A's date broke Wallet card
  binding. Deleting the config or rolling back the module is not a key rollback.
- **StrongBoxCapabilityMask**: it is a global `system_server` hook affecting every
  consumer of the system feature map, can cause compatibility issues, crashes or
  boot problems, and cannot prove the device hides its unlock state or passes
  integrity checks. Use it only on A with real StrongBox and read the
  [Mask documentation](StrongBoxCapabilityMask/README.md).
- **Transport and credentials**: `tls_insecure: true` skips A's certificate
  verification and B accepts invalid certificates. Public deployments must
  protect transport, tokens and Server themselves. An HTTPS URL alone is not
  proof of authenticated identity.
- **Release signing**: both APKs use the Android default debug signing identity,
  which CI caches. It is not a formal release key; cache eviction can change the
  identity, so verify certificates before upgrading.
- **No warranty**: the project is provided as is, without guarantees of fitness,
  security or any detection result. You assume the risks of data loss, device
  problems, account restrictions, business or financial loss, to the extent
  permitted by applicable law.

## Documentation

| Document | Contents |
|---|---|
| [A-side README](a-side/source/README.md) | Injection and routing, remote configuration, keybox, authentication signing, build and validation |
| [B-side README](b-side/source/README.md) | Relay protocol, SPL rules, battery telemetry, screen-off availability |
| [Server README](server/source/README.md) | Battery fields, rate limits and local tests |
| [UI README](ui/README.md) | Shared design system, previews, browser smoke tests, deployment and rollback |
| [Mask README](StrongBoxCapabilityMask/README.md) | Hook point, verification, compatibility and rollback |
| [CHANGELOG.en.md](CHANGELOG.en.md) | Per-version notes for 2.1.x / 2.0.x |
| [RELEASING.md](RELEASING.md) | Release steps and version maintenance (maintainers) |

## Credits and license

- Upstream: [jiyin004-jpg/ommega](https://github.com/jiyin004-jpg/ommega), original
  author jiyin004
- This fork: [Andrea-lyz/ommega](https://github.com/Andrea-lyz/ommega),
  maintained by Andrea-lyz

Referenced open-source implementations:

| Project | Author | GitHub |
|---|---|---|
| Tricky Store | 5ec1cff | [5ec1cff/TrickyStore](https://github.com/5ec1cff/TrickyStore) |
| Tricky Addon | KOWX712 | [KOWX712/Tricky-Addon-Update-Target-List](https://github.com/KOWX712/Tricky-Addon-Update-Target-List) |
| OhMyKeymint | James Clef (qwq233) | [qwq233/OhMyKeymint](https://github.com/qwq233/OhMyKeymint) |
| KeyAttestation | vvb2060 | [vvb2060/KeyAttestation](https://github.com/vvb2060/KeyAttestation) |
| TEESimulator-RS | Enginex0 | [Enginex0/TEESimulator-RS](https://github.com/Enginex0/TEESimulator-RS) |

Existing AGPL-3.0-or-later, MIT, Apache-2.0 and other source licenses still apply.
This README does not change any third-party ownership or license.
