# Ommega UI

The A/B module WebUIs share local CSS tokens and small JavaScript presentation helpers. The existing one-A / one-B routing and configuration formats are retained. The B Android app uses Compose Material 3 with the same colors and information hierarchy.

## Surfaces

| Surface | Navigation | Runtime data |
| --- | --- | --- |
| A module WebUI | Overview / Apps / Settings | Recorded KeyMint relay verdict, daemon PID, SOTER watchdog snapshot, installed version |
| B module WebUI | Overview / Connection / Settings | Local relay PID, current OS/SPL properties, saved connection summary |
| B Android app | Overview / Connection / Settings | Android relay StateFlow, configuration drafts, manual diagnostic results |

KeyMint and SOTER are separate paths from A through Server to B. A SOTER `ledger` value indicates that its ledger file exists; it is not a key count. The A relay health file is event driven, not a heartbeat. A running B process is not proof of a server connection. Missing data stays unknown, and expired SOTER snapshots are labeled stale.

Language defaults to the system: Chinese locales use simplified Chinese; other locales use English. System / Chinese / English and system / light / dark overrides are available. Only appearance preferences are written to WebView storage. Tokens are masked, and configuration credentials stay in the existing device configuration files. Language changes ask before discarding unsaved form values.

A places Global scope directly below the app search bar. Each application opens a policy page with individual scope, takeover mode, authentication signing and CPU/device-property controls. Global scope pauses the individual scope switch while app policies remain available. Browser history, native WebView back events and the Back button return to the list. An inherited auth policy remains effective until a specific mode is selected.

Positive key ID compatibility is a global switch in A's Settings, separate from app scope edits. It reads and verifies the saved value and retains unrelated configuration fields; legacy package lists are cleared when the global switch is turned off. Saving compares the file with the read snapshot and reports conflicts.

A's System capability mask entry opens a dialog with independent StrongBox and
device ID attestation mask switches. Both default to off. Saving verifies the
stored policy and keeps the restart notice visible. The dialog uses the existing
theme, scroll and back handling, wraps feature names on narrow screens and shows
when its system configuration is not mounted. See the
[capability mask contract](../a-side/source/docs/SYSTEM-CAPABILITY-MASK.md).

A's Apps overflow menu appears below global scope and contains refresh and add-system-app operations. Bulk select/deselect actions are not exposed: scope changes only through an application's own page. Each individual scope switch compares `target.txt` with the read snapshot before writing, changes only that package's entry, and verifies the result. The list is always the system's installed-app set, so an app that was never added to the scope is still visible with its switch off. Global scope ignores `target.txt` during interception and pauses individual scope switches. Adding or removing a system app only changes `system_app`; it never writes `target.txt`. If the CPU feature is disabled with other apps already saved, use the CPU settings available on that app page to avoid silently enabling the other apps. Global configuration, language, help and about are in Settings. The remote dialog masks the token by default and groups advanced options without changing their saved values. A packages only the English and simplified Chinese translation files.

App-list certificate badges count successful remote attestation requests after the chain is validated and the generated key is saved. Failed calls and internal retries are not counted. The counter begins with this version and is stored in the existing A-side `keymaster.db`; ordinary restarts preserve it, while resetting that database also resets the counters. Writes run asynchronously and never change the attestation result; a full queue, an interrupted pending write, or a persistently unavailable database can lose a sample. If the authenticated caller shares a UID, each package in that UID displays the same shared count with a shared-UID label; individual callers cannot be distinguished. Server-keybox responses count as remote requests too.

The A/B top refresh buttons await their complete status read, expose a busy state and queue one fresh follow-up when explicitly requested during a read. Automatic polling pauses in hidden pages; manual reads bypass polling throttles. Read-only callbacks stop waiting after 12 seconds without terminating any device process. Failed or partial reads are reported and release the action for retry. Writes and SPL apply operations do not gain a timeout, cancellation or automatic retry. Refresh motion follows the system's reduced-motion preference.

A's top refresh reads local health, SOTER, version, saved scope count, cached B properties and the configured B's live server status/battery. In Apps it also re-enumerates installed user apps and visible system apps, reloads policies/counters and preserves search and the editor's original compare-and-swap snapshot. The Apps menu refresh invokes this same reader and suppresses the vendor's duplicate loader. Startup and later legacy app-load entry points also join this queue, so a delayed bootstrap cannot clear a manually refreshed list or search. First-load app labels use the local dictionary, then small metadata batches; a rejected batch is retried package-by-package so one bad reply cannot turn a whole page of apps into raw package ids. If only global scope is unreadable, the readable list remains visible, scope controls stay disabled and refresh reports an incomplete read.

The Apps list is rebuilt from the system's currently installed packages on every startup and manual refresh. It is not filtered or projected from `target.txt`: every installed app is shown, with the per-app switch reflecting whether its package is currently in `target.txt`. Refresh never writes `target.txt`, and an uninstalled package remains in the file until the user changes that app's scope; no uninstall cleanup runs. When global scope is enabled, the backend ignores `target.txt` for scope decisions and the individual switches are paused. The search field is a native `<input>` rather than a Material text field, so Android IME composition is handled by the platform instead of being reset by component re-renders. Settings refresh re-reads compatibility and system capability summaries; dialog reload actions remain separate. A local status read does not issue a new attestation or regenerate a property snapshot.

B's top refresh works on Overview, Connection and Settings and awaits saved relay configuration, process/version/OS/SPL status, read-only SPL minimum/applied-target status and the configured device's server status. It bypasses the 20-second server throttle. Connection reread, SPL reread and opening SPL/About also refresh these sources. Automatic and global reads preserve dirty relay/SPL drafts; explicit section rereads ask before replacing only that section's draft. A newer saved config refreshes summaries and the server target without advancing an unsaved editor's original CAS digest. `spl-control.sh status --readonly` computes the same floors without creating origin/baseline/applied/config files or applying properties/restarting a service.

A's relay badge and icon report the recorded outcome of the last request; the record time is shown in the card footer. A certificate response that fails validation or key storage also records a failure after the HTTP response, while a more specific server error is retained. Only a recent failure raises an alarm; an older one stays visible as a record, and a recovered relay is not reported as a failure. The daemon publishes the verdict of the last request next to the timestamps, because a failure and a success inside the same second cannot be ordered by time. The reason the relay gave is shown below the result. The B overview reads the locally saved property snapshot and shows the device ID configured in the WebUI file; this ID does not prove a connection. The connection badge checks the server's public status for that device's recent polling and becomes unknown if the status cannot be read. Complete-request latency is shown only for a successful validated response carrying a B task ID; a server-keybox result clears it, and a failed latest relay verdict displays no latency. B battery data is taken from the exact configured device row in `GET /api/status/`: integer `battery_percent` (0–100), nullable boolean `battery_charging` and `battery_updated_at_ms` (server receipt time). A shows percentage and known active charging state only for an online device whose report is at most 90 seconds old and no more than 60 seconds ahead of the response's `server_time_ms` (falling back to A's clock only for older servers without that field). Missing, unsupported, malformed, offline or stale data clears the display instead of keeping an old value or inventing 0%. B samples read-only battery sysfs with a five-second in-memory cache and carries optional telemetry on its existing authenticated poll. Neither a new HAL/TEE request nor a persistent runtime file is needed. Both the B relay and server must support telemetry; older servers/APK producers remain compatible but show no battery data. See the [B telemetry protocol](../b-side/source/README.md#battery-heartbeat-telemetry).

The side rail and desktop navigation start at 761px; narrower WebViews keep the bottom dock. The B connection log controls stack in one column so their level selects stay readable beside the rail.

B's Overview, Connection and Settings use the shared dashboard, form-card and action-row surfaces. The relay, SPL and configuration commands and their element IDs are unchanged.

Legacy setting dialogs keep their original handlers while the simple forms use the design field and switch markup: boot hash/key, SOTER toggles, help and about are rebuilt on the sheet surface, and the remote/automation vendor controls use the shared field skin. On narrow screens they open as a bottom sheet with a handle, header and close control. The app list keeps the vendor DOM and uses the design panel with flush rows and scope hints. There is no save button: the detail switches and global scope persist themselves. The search field is a native input (not a Material text field) so Android IME composition for Chinese/Japanese/Korean is not reset by component re-rendering. Keybox, mode, file selector, uninstall and add-system-app dialogs use the same sheet surface and design controls.

While a dialog is open the navigation and floating actions fade with the covered page and stop looking actionable. The back gesture closes the topmost dialog instead of switching the dashboard view. The app list keeps enough room to scroll its last card clear of the navigation dock.

B SPL and OS values can only increase. Their minimum is the highest of the device original, currently reported value and recorded applied target; blank fields keep the current effective value. The settings dialog separately displays original values, `FLOOR_*` minimums and `APPLIED_*` targets. Recorded dates are retained. Restoring factory dates requires a factory reset; see [B date controls](../b-side/source/SPL-PERMANENT-DATE-IDENTITY.md).

## Build and previews

```sh
node ui/build.mjs
node --test ui/tests/*.test.mjs
npm ci --prefix ui
npm run --prefix ui design:build
node ui/preview.mjs
```

Open `http://127.0.0.1:4179/a/`, `/b/`, or `/openui/`. The A/B previews use inert fixtures and block mutation commands; no device is connected. The OpenUI preview uses actual `@openuidev/react-ui` Card, Button and ThemeProvider components and local sample data. It is a development artifact and is not included in either device module. OpenUI primitives work without an LLM; no cloud/model endpoint is required by these previews.

The shared design layer lives in `ui/shared/material-you.css` (tokens and components) and `ui/shared/icons.js` (the 31 line glyphs copied from `ui/material-you-preview`). `ui/shared/modern.css` only aliases those tokens for the older `om-*` selectors, so the design preview and the packaged pages keep one visual source.

The shared assets are copied into both independent module packages. CI checks that they match their sources. A's existing prebuilt app list and all dialog IDs are preserved. Its compatibility glue is regenerated using `python a-side/source/scripts/webui-patch.py`; the baseline reflects the already-modified vendor bundle from master `1d214a4`, verified before updating it.

Browser smoke test (against the preview server, with Playwright installed or `PLAYWRIGHT_MODULE` set to a local Playwright package):

```sh
node ui/tests/browser-smoke.mjs
```

It checks Chinese/English, light/dark, 320–1024 px layout, refresh loading/duplicate-click/error/retry behavior, reduced-motion support, A remote/SOTER dialogs, B form loading and token visibility, B SPL minimum/applied-target display, and the OpenUI prototype. Screenshots and results are written to ignored `ui/test-results/`.

`node ui/tests/refresh-battery-smoke.mjs` loads the actual packaged A/B pages with in-memory bridge fixtures and local browser routes, without a listening server or device. It checks percentage boundaries, charging/unknown/stale/offline/device isolation, app list and settings rereads, delayed-bootstrap and cross-control queuing, retained drafts/search, forced server polling, completion ordering, missing-callback retry and 320–1024 px layouts in Chinese/dark and English/light. `ui/tests/refresh.test.mjs` covers shared forced/queued/hidden/error semantics; `ui/tests/b-refresh.test.mjs` executes the B controller with mock DOM/bridge and virtual timers.

`node ui/tests/capability-smoke.mjs` checks the A capability dialog with the same
Playwright setup. It loads packaged assets directly with an in-memory bridge,
requires no preview server and writes screenshots to `ui/test-results/`.

## Android preview and validation

```sh
cd b-app/source
./gradlew :app:assembleDebug -PuiPreview=true
```

The preview has application ID `org.ommega.deviceb.preview`. Relay service and boot receiver are disabled in its manifest, action callbacks do not run, and the UI clearly labels all state as sample data. Normal builds omit `-PuiPreview=true`; CI creates the regular B APK. Compose BOM 2025.12.01 intentionally matches the existing SDK 36 / AGP 8 toolchain.

Local validation on 2026-10-21:

- 50 host JavaScript tests passed: presentation 8, B configuration 10, A compatibility 11, stubbed B SPL controller 21. No real HAL/device calls were used.
- B Android Release assembled, lintVital passed, two native unit tests passed. Preview Debug assembled.
- Five browser smoke cases passed without JavaScript runtime errors, including a real OpenUI component render and nested-dialog closure through buttons, browser back and native WebView back.
- Mobile MCP on A (`PJZ110`) verified the separate native preview, Chinese system adaptation, manual English switching, and dark theme. Production A/B modules and the real relay/TEE paths were not deployed or exercised as part of this UI change.
- Compose compiler: 10/10 named restartable composables skippable; all composables 67/76 (88.2%), Strong Skipping enabled. See [Compose audit](../b-app/source/COMPOSE-AUDIT-REPORT.md).

## Deployment and rollback

Keep the existing module/APK as the rollback artifact. For A, replace only the module UI through the normal tested module package, preserving its device configuration. B deployment requires the separate explicit approval and preflight in `AGENTS.md`; local implementation and CI are not B deployment approval. Do not run manual Keystore diagnostics or restart services merely to validate the new UI. A WebUI-only deployment can be rolled back by restoring the backed-up webroot; no reboot or HAL reload is needed for static assets.

Restoring B UI assets does not lower an applied date. Preserve B's `spl-applied.conf` and its recorded floor when changing module versions.

Design reference: [OpenUI standalone components](https://github.com/thesysdev/openui/tree/main/packages/react-ui), [Compose state](https://developer.android.com/develop/ui/compose/state), [Compose lifecycle](https://developer.android.com/develop/ui/compose/lifecycle).
