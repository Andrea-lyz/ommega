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

A's Apps overflow menu contains only app operations. Global configuration, language, help and about are in Settings. The original dialog bindings stay hidden behind those Settings actions. A packages only the English and simplified Chinese translation files.

The A/B refresh buttons show a busy state while reading status and reject duplicate clicks until the read finishes. A failed read releases the button for retry. Refresh motion follows the system's reduced-motion preference.

B SPL and OS values can only increase. Their minimum is the highest of the device original, currently reported value and recorded applied target; blank fields keep the current effective value. The settings panel separately displays original values, `FLOOR_*` minimums and `APPLIED_*` targets. Recorded dates are retained. Restoring factory dates requires a factory reset; see [B date controls](../b-side/source/SPL-PERMANENT-DATE-IDENTITY.md).

## Build and previews

```sh
node ui/build.mjs
node --test ui/tests/*.test.mjs
npm ci --prefix ui
npm run --prefix ui design:build
node ui/preview.mjs
```

Open `http://127.0.0.1:4179/a/`, `/b/`, or `/openui/`. The A/B previews use inert fixtures and block mutation commands; no device is connected. The OpenUI preview uses actual `@openuidev/react-ui` Card, Button and ThemeProvider components and local sample data. It is a development artifact and is not included in either device module. OpenUI primitives work without an LLM; no cloud/model endpoint is required by these previews.

The shared assets are copied into both independent module packages. CI checks that they match their sources. A's existing prebuilt app list and all dialog IDs are preserved. Its compatibility glue is regenerated using `python a-side/source/scripts/webui-patch.py`; the baseline reflects the already-modified vendor bundle from master `1d214a4`, verified before updating it.

Browser smoke test (against the preview server, with Playwright installed or `PLAYWRIGHT_MODULE` set to a local Playwright package):

```sh
node ui/tests/browser-smoke.mjs
```

It checks Chinese/English, light/dark, 320–1024 px layout, refresh loading/duplicate-click/error/retry behavior, reduced-motion support, A remote/SOTER dialogs, B form loading and token visibility, B SPL minimum/applied-target display, and the OpenUI prototype. Screenshots and results are written to ignored `ui/test-results/`.

## Android preview and validation

```sh
cd b-app/source
./gradlew :app:assembleDebug -PuiPreview=true
```

The preview has application ID `org.ommega.deviceb.preview`. Relay service and boot receiver are disabled in its manifest, action callbacks do not run, and the UI clearly labels all state as sample data. Normal builds omit `-PuiPreview=true`; CI creates the regular B APK. Compose BOM 2025.12.01 intentionally matches the existing SDK 36 / AGP 8 toolchain.

Local validation on 2026-10-21:

- 47 host JavaScript tests passed: presentation 5, B configuration 10, A compatibility 11, stubbed B SPL controller 21. No real HAL/device calls were used.
- B Android Release assembled, lintVital passed, two native unit tests passed. Preview Debug assembled.
- Five browser smoke cases passed without JavaScript runtime errors, including a real OpenUI component render.
- Mobile MCP on A (`PJZ110`) verified the separate native preview, Chinese system adaptation, manual English switching, and dark theme. Production A/B modules and the real relay/TEE paths were not deployed or exercised as part of this UI change.
- Compose compiler: 10/10 named restartable composables skippable; all composables 67/76 (88.2%), Strong Skipping enabled. See [Compose audit](../b-app/source/COMPOSE-AUDIT-REPORT.md).

## Deployment and rollback

Keep the existing module/APK as the rollback artifact. For A, replace only the module UI through the normal tested module package, preserving its device configuration. B deployment requires the separate explicit approval and preflight in `AGENTS.md`; local implementation and CI are not B deployment approval. Do not run manual Keystore diagnostics or restart services merely to validate the new UI. A WebUI-only deployment can be rolled back by restoring the backed-up webroot; no reboot or HAL reload is needed for static assets.

Restoring B UI assets does not lower an applied date. Preserve B's `spl-applied.conf` and its recorded floor when changing module versions.

Design reference: [OpenUI standalone components](https://github.com/thesysdev/openui/tree/main/packages/react-ui), [Compose state](https://developer.android.com/develop/ui/compose/state), [Compose lifecycle](https://developer.android.com/develop/ui/compose/lifecycle).
