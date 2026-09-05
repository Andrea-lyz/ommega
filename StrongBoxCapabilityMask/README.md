# StrongBox Capability Mask

Minimal libxposed API 102 module that runs only in `system_server` and removes
`android.hardware.strongbox_keystore` from Android's global `SystemConfig`
feature map.

The module does not load into application processes. Version 1.4.1 is
intentionally global; package-specific filtering is not implemented.

## Why this hook point

Android 16 can publish SDK system features to applications through
`SystemFeaturesCache` during process binding. Masking only
`PackageManagerService.hasSystemFeature()` can therefore be too late. Removing
the feature from `SystemConfig.getAvailableFeatures()` keeps PackageManager,
feature enumeration, and the process-start cache on the same global view.

The PJZ110 Android 16 target exposes the exact device method:

```text
com.android.server.SystemConfig.getAvailableFeatures(): android.util.ArrayMap
```

Its generated `RoSystemFeatures.maybeHasFeature()` returns `null`, so it does
not hard-code StrongBox ahead of the runtime feature map.

## Build

Set `ANDROID_HOME` or create `local.properties`, then run:

```powershell
.\gradlew.bat testDebugUnitTest assembleDebug
```

The Ommega root CI also runs `:app:testDebugUnitTest :app:assembleRelease` and
bundles `StrongBoxCapabilityMask-1.4.1-debug-signed.apk` with A/B/Server artifacts.
The Release build uses the standard Android debug signing configuration.
This module shares the product version; its runtime feature-mask behavior stays global.

## Install and enable

1. Install the generated APK.
2. Enable the module in an API 102 compatible LSPosed implementation.
3. Keep the static scope at `system` only.
4. Reboot the device. Restarting an application is not enough because the
   feature cache is populated during system boot and process binding.

## Verify

```text
adb shell pm has-feature android.hardware.strongbox_keystore
adb shell pm list features
adb logcat -d -s StrongBoxCapabilityMask
```

Expected results:

- `pm has-feature` prints `false`.
- `pm list features` does not contain `android.hardware.strongbox_keystore`.
- logcat contains `event=hook_registered` and one `event=feature_removed`.

The module masks the PackageManager capability only. It does not stop the
vendor StrongBox HAL or change direct KeyStore/KeyMint routing.

## Device validation

Validated on PJZ110, Android API 36, LSPosed 2.2.0 on 2026-09-04:

- `system_server` loaded the module with libxposed API 102.
- `SystemConfig.getAvailableFeatures()` was hooked before feature publication.
- The StrongBox feature was removed while the original ODM XML stayed unchanged.
- Shell PackageManager queries returned StrongBox `false` and app-attest-key `true`.
- A real instrumentation application process returned the same values, confirming
  that Android 16's process-local `SystemFeaturesCache` received the masked map.
- No new `system_server` crash or ANR was observed after reboot.
- After clearing Paytm 10.83.12 data, a cold start reached the normal login flow
  and then the main page without warning `00000`, `61007`, or `61007-ISTxx`.
  No StrongBox request, StrongBox fallback, or self-exit appeared in the aligned
  runtime log window, and the Paytm activity remained resumed.

## Rollback

Disable the module in LSPosed and reboot. No system partition file is changed.
