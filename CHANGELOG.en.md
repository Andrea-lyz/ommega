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
