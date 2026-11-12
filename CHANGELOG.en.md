# Changelog

This file records per-version notes for A / B / Server and the optional APKs from
2.0.0 onward. Full notes for earlier releases are on
[GitHub Releases](https://github.com/Andrea-lyz/ommega/releases). The last section
summarizes early fixes that still shape current behavior.

A / B / Server / B-app / StrongBoxCapabilityMask share one version number. Unless
stated otherwise, updating the module ZIPs from a release does not update the
running Server and does not install the optional APKs.

The README no longer embeds per-version notes: a release only bumps the version
and appends an entry here. See [RELEASING.md](RELEASING.md) for the steps.

## 2.4.2 — 2026-11-12

A-side automatic renewal, local attestation timing and a dark-mode WebUI fix
(A only; B and Server are unchanged, no Server update is required).

**Three independent defects in automatic renewal**

- **It could silently skip issuance.** The renewal thread waited with a relative
  `thread::sleep`, which on Android runs on a clock that excludes deep sleep. A
  wait of "seven awake hours" therefore stayed pending across a full night of
  suspension: a TEE chain could enter its renewal window, pass its due time, and
  never be re-issued, while the page and logs still looked healthy — because no
  attempt had started at all. That is the reported case: policy enabled, seed
  decryptable, TEE already `due`, and the thread still holding the wait it
  scheduled that morning. The thread now waits on an **absolute wall-clock
  deadline** (`clock_nanosleep` with `CLOCK_REALTIME` and `TIMER_ABSTIME`), so
  suspended time and clock corrections both count. An interrupted wait reuses the
  same deadline instead of restarting a delay, and an unexpected kernel error logs
  and backs off two minutes rather than busy-looping. The wait stays clamped
  between two minutes and one day. **It does not wake a suspended device**; an
  elapsed deadline is handled as soon as the device resumes.
- **It deadlocked on its own lock.** `auto-run` took the issuance lock and then
  called `issue()`, which took the same file lock again. `flock` is held per open
  file description, so the second acquisition inside one process always failed:
  every unattended renewal reported "an issuance is already in progress" and never
  issued, while the six-hour backoff pushed the retry even further out. It is now
  split into caller-locked `issue_locked()` and self-locking `issue()`.
- **It hid its failures.** An unattended failure was only written to
  `feedback.json`, which the next status read replaces, and `policy.json` kept an
  empty `last_error`, so the page showed a healthy card for days. The reason is
  now persisted next to the attempt time, exposed in `status` and rendered on the
  RKP card.

**Local attestation timing**

- A local attested `generateKey` spent the same secure-world band as an
  unchallenged one, so a validator pairing the two generations measured a
  near-zero increment: 4–6 of 10 paired deltas were non-positive and
  `dAtt/initSign` stayed near 0.1. Ordinary Operation latency cannot close that,
  because both paths pay it.
- A `RetrieveCertSigningInfo` hook now accounts one sampled 24–40 ms of
  certificate work after a local leaf is encoded and before the key can be sealed
  or observed. It covers a warm keybox cache, a caller-selected local issuer,
  locally certified authentication-compatible keys and permitted remote fallback;
  self-signed leaves, failed generation and remotely returned chains do not pay it.
- Measured on the A device: `dAtt` 2.0/−2.2/−2.9 ms became 32.9/39.2/35.0 ms, and
  non-positive pairs 4/6/5 became 0/0/0.

**WebUI dark mode**

- Diagnostics log boxes were unreadable in dark mode. Two mistakes compounded: the
  background referenced `--om-surface-alt`, which no theme block ever defined, so
  it always took a light literal fallback; and the only dark override was scoped to
  `.om-modern[data-theme=dark]` while the theme is written to `:root`, so it could
  never match. The measured dark contrast was about 1.04. The token is now defined
  for light and both dark paths, the box references the token alone, and the
  measured dark result is `rgb(240,239,247)` on `rgb(27,30,39)`, meeting WCAG AA.

**Bundled component**

- The bundled AProbe moves from 1.6 (versionCode 7) to 1.8.8-paytm
  (versionCode 19), carrying the focused timing diagnostics. The signing identity
  is unchanged (`CN=Ommega AProbe`), so it upgrades in place.

Upgrade notes: update the A-side module ZIP only; B and Server need no change.

- The renewal fixes live inside the keymint daemon, so they **require the module
  deployment flow to reload the daemon — replacing the `ommega-rkp` tool alone has
  no effect**. Existing policy, the imported seed and every issued keybox are kept.
- The attestation-timing change spans KeyMint and the TA and likewise needs the
  daemon reloaded.
- The WebUI fix needs the module to be repackaged and the page refreshed; a forced
  refresh avoids a stale WebView cache.
- If your TEE chain is already near or past expiry, this update does not
  retroactively renew it: check Settings → Automation after deploying and issue
  manually if needed. A manual issuance spends one registration identity and is a
  device-side operation.

Validation: CI is green (A, B, Server, APKs, UI). Seven renewal-timing
regressions, three theme-contrast regressions and the paired timing assertions
pass locally; reverting to the relative wait, restoring the literal fallback or
dropping the timing charge makes the matching regression fail. The dark log box
was measured with computed colours in desktop Chromium using the same CSS, and the
attestation timing was measured on the A device. **It has not been verified inside
a real-device WebView, and automatic renewal was not observed firing on a
device**; both still need on-device acceptance.

## 2.4.1 — 2026-11-09

Diagnostics accuracy in local mode (A only; B and Server are unchanged).

- The KeyMint page no longer reports relay mode while the daemon runs locally;
  the mode now comes from `data/relay-mode`.
- Toggling the debug-logging switch updates the unlock button immediately instead
  of waiting for the throttled refresh.
- Failures reported against the keystore proxy (uid 1017) are attributed to the
  requesting app instead of being hidden as a system identity.
- Shared-uid packages such as Google Play services (10120 / 99910120) resolve to
  their package name.
- Packages outside the managed list get their real label through the KernelSU
  bridge.
- Diagnostics tests grew to 10 cases covering failure attribution and shared-uid
  parsing.

Upgrade notes: update the A-side module ZIP only; installing over an older
version keeps data and every issued keybox.

## 2.4.0 — 2026-11-09

Tiered diagnostics pages and the in-app device physical on the A side (A only;
B and Server are unchanged).

- Settings → Diagnostics becomes a tiered page: one debug-logging master switch,
  a reproduce-then-unlock gate, and seven feature subpages (request failures,
  device check, KeyMint, injector, relay, property view, SOTER). Failures
  aggregate per app with label, package and reason, and the overview mirrors them.
- The device check is native inside the WebUI; AProbe ships as an optional
  signed component that installs in one tap and joins the property view and the
  automatic authorization policy.
- Fingerprint signing certificate issuance gains a keybox choice: either the TEE
  or the StrongBox slot, including a directly imported Samsung StrongBox keybox,
  with an explicit fallback notice when a slot is missing.
- Authentication-bound fingerprint signing is now attested locally on the A side,
  so relay mode no longer surfaces `KEY_USER_NOT_AUTHENTICATED`.
- Property view fixes: publish the target identity to `*_for_attestation`, skip
  long properties that cannot be rewritten, and no longer abort the view when the
  host lacks a property (resolves -66 for some apps).
- Log control: the master switch also drives local and relay KeyMint / injector
  levels, with log clearing, a rotation cap and a detailed SOTER narrative.
- Keybox slots and the issuance keybox selection file are owned by
  `keystore:keystore` at 0600 and normalized on import and at startup.
- The WebUI switches faster: diagnostics caching and throttling, bounded log
  reads and off-screen app-list rendering.

Upgrade notes: update the A-side module ZIP only.

## 2.3.1 — 2026-11-02

Fixes the missing `ommega-rkp` in the 2.3.0 package.

- The 2.3.0 ZIP contained `ommega-rkp`, but the installer extracts binaries by
  name and the new tool was never added to that list, so after installation the
  whole Settings → Automation page reported its backend as unavailable.
- The installer now extracts and verifies `ommega-rkp` per ABI and aborts instead
  of silently leaving it out.
- CI now checks in `prepare` that every binary packaged by `build.py` also appears
  in the installer list.
- Only the installer changed; A-side behaviour is identical to 2.3.0. Devices on
  2.3.0 can install 2.3.1 over it and keep all data and issued keyboxes.

## 2.3.0 — 2026-11-02

A-side automation page and RKP issuance (A only; B and Server are unchanged, no
Server update is required).

- Settings → Automation is a full page instead of a sheet. The four keybox
  download sources are gone: the page imports the user's own CDI material bundle
  and keybox files, one slot each for TEE and StrongBox, never shared.
- The bundle keeps the RKP workbench version 1 contract (`dice_chain_hex`,
  `signing`, `device`, `issuance`). It is stored AES-256-GCM encrypted at
  `data/rkp/seed.enc` with a per-install key at `data/rkp/master.key` (both 0600);
  a blob copied from another device reports as undecryptable instead of failing at
  issue time. The module ships no seed and no keybox.
- The RKP engine is ported from Duck ToolBox (MIT, `a-side/source/rkp-engine`):
  each issuance requests a fresh challenge, generates a new P-256 attestation key,
  signs CSR v3 with the imported leaf identity and exports the verified chain. An
  issued keybox passes the same slot checks as an imported one: the chain level and
  `O=` must match the slot.
- New `ommega-rkp` tool: `status`, `due`, `seed-import`, `seed-remove`,
  `keybox-import`, `policy`, `issue`, `auto-run`. Issuance rewrites
  `security_level` in the stored bundle to the target slot, so material captured
  for TEE cannot be spent on a StrongBox request.
- Automatic renewal follows the server-provided `time_to_refresh_hours`. The
  keymint daemon checks five minutes after start and then sleeps to the next
  renewal instant (capped at one day, so one check per day while idle); a failed
  attempt is not retried for six hours. It shares an flock with the manual path,
  and the network and keybox work run in a child process, never in the TA.
- The per-app policy page hides the authentication-signing, local-keygen EWMA and
  CPU/device-property cards in local mode; remote mode is unchanged.
- Fixed the top refresh reporting "some status could not be read" in local mode.
- Fixed keybox expiry parsing treating a two-digit UTCTime year as four digits,
  which displayed 2611.

Upgrade: update the A module ZIP only. B and Server need no change. Refresh the
WebUI afterwards; the deploy flow reloads keymint, no reboot needed.

Verified: CI green (A, B, Server, APKs, UI). On a PJZ110 device both TEE and
StrongBox issuance completed and installed into their slots (chains
`O=TEE` / `O=StrongBox`, four certificates each), the 29 engine unit tests pass on
the device, and the first automatic check reported "not due" without spending a
request.

## 2.2.2 — 2026-11-01

Per-application SOTER identities and request counters on A.

- The A-side SOTER TA resolves an application to a named identity: local random,
  imported private key, or remote relay, chosen per package. Packages without an
  assignment use the default identity. The store lives at
  `/data/misc/keystore/ommega/data/soter/identities.json` (0600) and any parse
  error fails closed.
- A master switch on the identity page writes only the watchdog flag: turning
  SOTER off keeps assignments but falls back to the stock HAL for every app. The
  separate "restart software TA" action is gone.
- The application list shows a "SOTER N" badge with the same pill shape as the
  certificate counter in a different colour, rendered only while SOTER is on.
- Identity management uses the Rust writer (`soter-read` / `soter-write` /
  `soter-import`) with a directory lock, compare-and-swap, atomic replace, fsync
  and readback; private keys are read on device by path and never enter the page.
- Relay identities work per application with optional UID mapping and TLS-skip,
  and a named relay no longer receives this device ledger id.
- The anti-rollback counter is monotonic per application with a clock floor,
  fixing `counter is not greater than that in server` when registering WeChat.
- The watchdog status note is derived from the published state, so a re-enable no
  longer repeats an old failure reason.
- Fixes a compatibility regression in automatic authentication signing.

Upgrade: only the A module and its WebUI change; B and Server have no code
changes in this release (versions stay in step). Installing the A module restarts
the injector and the bare service; SOTER state and identity assignments persist,
so bound applications such as WeChat do not need to register again.

Validation: local Rust (fmt / clippy `-D warnings` / 52 tests) and UI (148 tests,
including a packaged browser run) pass; on device A, WeChat fingerprint payment
registration, `SoterChecker`, and a loopback relay probe for one application were
verified. B was not re-tested because it does not change in this release.

## 2.1.4 — 2026-10-29

B WebUI server-status card fix.

- The Server connection card no longer depends on WebView-side curl/wget. With
  `curl`, it reads Server's `/api/status/`; without it, or when the request fails,
  it reads the last `b/poll status=` from the relay log and shows 200/204, 429,
  401/403, 5xx or stale.
- Fixes the false "Invalid server response" / "Some status reads failed" on stock
  Android without `/system/bin/curl`; a real poll of 204 now shows "Server
  connected" and the last poll time. The incompatible busybox wget status path is
  removed.
- Only the B WebUI changed; B relay code, A/Server, `service.sh` and TEE/KeyMint
  are untouched. The Server 2.1.3 poll-budget fix is retained.
- A/Server, B-app and StrongBoxCapabilityMask differ from 2.1.3 by the version
  string only. Updating the B module is enough for the status-card fix; both APKs
  remain debug-signed.
- Validation: all 26 B refresh/status tests pass, UI checks and master CI are
  green, and the customer test device shows "Server connected / Relay last poll
  succeeded (HTTP 204)".

## 2.1.3 — 2026-10-29

- **Server**: the B heartbeat `GET /api/b/poll/` moved out of the relay work
  budget onto its own `RELAY_POLL_RATE_LIMIT_REQUESTS` (default 0, disabled). The
  2.1.2 80 ms short poll (~12.5/s, ~45k/h) exhausted the default 20k/h work budget
  in about 27 minutes, then returned 429 with `Retry-After`; the device disappeared
  from the online list and tasks were delayed. Heartbeats no longer spend the
  A-task / `b/result` budget, while `RELAY_DATA_RATE_LIMIT_REQUESTS` still limits
  work in both directions.
- **A/B**: `version` authorizations now come from the real B TEE `profile`
  snapshot (a one-shot AES-256/GCM key, deleted immediately after the read), so
  local generation, import and remote attestation share one source. A certificate
  whose tags differ from the profile is rejected; older relays stay compatible
  with a warning. The tag 701 `creationDateTime` fix is retained.
- **A**: remote signing derives the algorithm from key characteristics, fixing
  `missing tag Algorithm` / `SERVICE_ERROR` / `Failed to begin operation`; the
  injector accepts Android's nonzero Parcelable presence markers (0 is still
  null), avoiding a split keystore view.
- **A**: software crypto is aligned with KeyMint for 3DES ECB/CBC PKCS#7,
  NONEwithRSA, the 16-byte AES-CTR initial counter, AES-192-GCM and 12–16-byte GCM
  tags; KmProbe reaches 10/10 on A.
- Upgrade: replace the Server `relay_rs` separately (the poll fix is server-side);
  update the A/B modules as needed. B-app and StrongBoxCapabilityMask differ from
  2.1.2 by the version string only.

## 2.1.2 — 2026-10-29

- Fixes most Apps rows showing raw package ids instead of application names. The
  manager exposes its package bridge as an injected Android WebView object, and a
  method only works when it is called through that object. The old code took the
  method off the object and called it later, so WebView rejected every batch with
  "Java bridge method can't be invoked on a non-injected object"; only the
  packaged `applist.json` names survived. Batched and per-package lookups now keep
  the live `ksu` receiver, and the Apps page shows real names again.
- The browser preview bridge rejects detached calls the same way, so the smoke
  tests and the new regression case run under that rule instead of letting this
  class of bug reach a device.
- Only the A-side WebUI changed; B, Server and both optional APKs differ from
  2.1.1 by the version string only.

## 2.1.1 — 2026-10-29

- Fixes the A-side Apps search field that needed two taps before the keyboard
  appeared on a cold start and could not accept Chinese candidates. Android
  WebView does not establish a text-input connection for an input parsed with the
  document (the system reports `TYPE_NULL`), so committed Chinese text was dropped
  while key-based English still worked. The field is now recreated when the Apps
  view opens, without taking focus, so one tap raises the keyboard and Chinese
  input works.
- The Apps search is an independent native input with its own controller: it no
  longer reuses the old Material field's events, styles or clear behaviour, list
  refreshes never read or write editor text, filtering waits for IME composition,
  and closing the app menu no longer pulls focus back to its button.
- Apart from search and this focus fix, every component differs from 2.1.0 by the
  version string only.

## 2.1.0 — 2026-10-29

- Both WebUIs share a Material You design system: tonal colors, rounded surfaces,
  status cards, bottom navigation/side rail and common form controls across
  Overview, Apps, Settings and Connection, including nested dialogs. Simplified
  Chinese / English and light / dark themes are retained, with responsive layouts
  from 320 to 1024 px.
- A's Apps page is now the system's installed-app list, re-enumerated at startup
  and on manual refresh instead of being filtered by `target.txt`. Each app opens a
  policy page for individual scope, takeover mode (global default / StrongBox /
  TEE), authentication signing (Auto / Payment compatibility / Preserve) and
  CPU/device properties.
- Scope writes now come only from one app's own switch: enabling or disabling its
  scope edits `target.txt`; there is no bulk write, "show all" filter or
  uninstalled-app cleanup. Refreshing after an uninstall removes the row but keeps
  the file entry. Global scope ignores `target.txt` and pauses individual scope
  switches, while other per-app policies remain available.
- A adds per-app certificate counts and the latest B-side latency. A count is
  recorded only after the certificate chain and key storage succeed, in the
  existing `keymaster.db`; shared UIDs are marked, and failed calls or internal
  retries are not counted.
- A adds a system-capability mask dialog for independently hiding StrongBox and
  device-ID attestation capability while retaining `app_attest_key`. CPU/device
  properties continue to use exp6's Zygisk view with values supplied by B and no
  manual value editing. Authentication-signing mode is saved per app on its policy
  page; positive-key-ID compatibility remains a separate global switch.
- B's WebUI is rebuilt; SPL/OS state is read through a read-only query while
  settings still save and apply, and connection drafts, token masking, language and
  theme behavior are retained. B adds read-only battery
  telemetry to its existing authenticated poll, and Server exposes
  `battery_percent`, `battery_charging` and `battery_updated_at_ms` on its status
  response. Missing, invalid or stale values remain unknown; zero and last-known
  values are never substituted.
- Server validates battery fields independently and updates queue metadata while
  remaining compatible with older B relays and servers; protocol, bounds and
  malformed-state tests plus `server/source/README.md` are included. A/B top
  refresh now awaits the complete read, exposes a busy state, queues one explicit
  follow-up and releases the action after failure.
- Added shared UI sources, a design preview and offline browser smoke coverage for
  A/B locales and themes, 320–1024 px layouts, the app list/scope flow, refresh,
  battery and capability dialogs, plus WebUI bundle-patch and release-package
  verification scripts.
- Upgrade A/B modules and Server separately. Restart A after installing so Zygisk
  loads the new library. Existing `target.txt` entries are retained and can be
  toggled per app in the Apps page.

## 2.0.1 — 2026-10-26

- A adds an independent "CPU and device properties" settings entry. A secondary
  app list supports search by name or package, shows app icons and lets you set a
  custom scope and include system apps; this scope is independent of the KeyMint
  app list and the global switch.
- Property values are supplied by B automatically; A no longer hardcodes a device
  model and offers no manual value entry. The exp6 Zygisk property view logic is
  retained; the configuration defaults to off with an empty scope, and missing or
  invalid configuration skips the app.
- App icon loading and default icons were adjusted, and dialogs now share the
  system back gesture, browser back and Escape behavior.
- When migrating from the standalone `zygisk_maskcpu` module, disable it first,
  then enable the built-in Ommega feature; restart A after installation so Zygisk
  loads the new library.

## 2.0.0 — 2026-10-23

- The authentication signing policy decides per app whether an auth-bound key
  stays local. Global scope covers ordinary apps only (app ids 10000–19999 per
  user), and engineering mode requires its matching Binder SELinux context and
  exact process name. The policy and global scope live in Settings; the Apps page
  and Overview no longer repeat those cards.
- SOTER keeps ASK, AuthKey and sign sessions on one backend. A `local_only` choice
  is written to the ledger and survives reloads, and sessions use an opaque handle
  assigned by A. Network and remote authentication errors never turn into a local
  signature.
- A valid grant brings an isolated process into the same Ommega caller scope. The
  delegation binds the real UID, anchor PID, start time and boot ID, and key access
  still enforces the grant `GET_INFO` / `USE` vector. Revocation, process exit or
  UID reuse ends it.
- Also: App AttestKey returns only the delegated leaf, local auth-key signing, GMS
  auth windows, system_server grant probing, nested settings dialogs and back
  gestures, and faster CI caching and toolchain setup.

## Early fixes

Full notes for 1.9.0 and earlier are on Releases. These fixes still directly shape
current behavior:

- **1.6.0**: an ommega-routed call is no longer answered with `SYSTEM_ERROR` while
  a mirror replay is in flight (bounded wait; poisoned lanes stay fail-closed); EC
  key generation restores AOSP semantics (EC + ENCRYPT warns instead of failing);
  RSA and ECDSA select their signature scheme from the digest the key was created
  for, fixing non-SHA-256 RSA signing failures and silently mismatched signatures.
  B's `spl-control.sh` refuses SPL/OS downgrades, and A logs `event=local_sign`.
- **1.4.6**: detector compatibility became a global positive-key-ID switch in
  `target-compat.toml` (`[positive_key_id]`), off by default and affecting only
  keys created afterwards.
- **1.4.5**: every HAL entry reaching the software TA now records a secure-world
  round trip; `debug_logging` records `event=remote_attest_chain`.
- **1.4.2**: overlapping calls on the same operation return `OPERATION_BUSY` at the
  injector boundary, matching the AOSP concurrency contract.
- **1.4.1**: covered remote profile, attestation/signing/decryption,
  P-256/P-384/P-521/X25519 agreement, and authentication keys across an A reboot.
