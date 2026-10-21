# Jetpack Compose Audit Report

Target: `b-app/source` · Date: 2026-10-21 · Confidence: Medium

Scope: production `RelayScreen.kt`, `RelayViewModel.kt`, `MainActivity.kt` and their service/client integration. `RelayScreenPreview` and the disabled-service preview variant are excluded from production behavior scoring. This is a small app; the assessment covers source, compiler reports and a native UI preview, not production device frame timings.

Overall Score: **87/100**

## Scorecard

| Category | Score | Weight | Status | Notes |
| --- | --- | --- | --- | --- |
| Performance | 8/10 | 35% | solid | Stable state and measured skipping; release shrinking remains disabled |
| State management | 9/10 | 25% | excellent | Immutable UI data, separate drafts, StateFlow, saved navigation state |
| Side effects | 9/10 | 20% | excellent | Lifecycle-aware collection; event-only I/O; non-interrupting diagnostic cleanup |
| Composable API quality | 9/10 | 20% | excellent | Stateless screen seam, explicit callbacks, outermost modifiers and content slots |

Weighted result: `8×.35 + 9×.25 + 9×.20 + 9×.20 = 8.65`, rounded to 87/100. No category renormalization.

## Critical Findings

No unresolved critical Compose defect was found. Three initial issues were corrected and independently re-reviewed:

1. **Connection checks could persist drafts.** `RelayViewModel.kt:74` now validates a draft and passes an explicit connection snapshot to `ServerClient.kt:63`; it does not call the saving path. The HTTP connection closes in `finally`.
   References: <https://developer.android.com/develop/ui/compose/state>
2. **Listener replay could overwrite a newer service state.** `MainActivity.kt` collects `RelayService.connectionUpdates` under `repeatOnLifecycle(STARTED)`. The replay and updates now share the same StateFlow.
   References: <https://developer.android.com/kotlin/flow/stateflow-and-sharedflow>
3. **Diagnostic text could retain the previous language.** `RelayViewModel.kt` stores certificate count, invariant body and failure status; `RelayScreen.kt` renders translated labels with `stringResource()`.
   References: <https://developer.android.com/develop/ui/compose/resources>

## Adjacent Findings

### Android Launch UX

No custom `windowSplashScreenAnimatedIcon` is configured in the app themes. The specific static custom-splash upscaling condition is therefore not asserted. Both normal and night themes use `NoActionBar`, and Compose owns the toolbar. Manual theme changes also update system-bar icon appearance.

References: <https://developer.android.com/develop/ui/views/launch/splash-screen>, <https://developer.android.com/develop/ui/views/layout/edge-to-edge>

## Category Details

### Performance — 8/10

Qualitative score 8; ceiling none; applied score 8. The Kotlin 2.3 Compose Compiler reports Strong Skipping enabled. Release reports generated with the audit skill's `compose-reports.init.gradle` show:

- Named-only: 10/10 restartable functions skippable (100%). Excluding the preview function: 9/9.
- Module-wide: 67/76 restartable composables skippable (88.2%), including compiler-generated lambdas. This lower figure is not used as the named-only ceiling.
- `RelayConfig` and `RelayUiState` are inferred stable. The unstable Activity, Service and ViewModel classes are not passed as shared composable data.

Evidence: `app/build/compose_audit/app_release-composables.csv`, `app_release-classes.txt`, `release/app_release-module.json`; `RelayScreen.kt:57`, `RelayViewModel.kt:18`.

References: <https://developer.android.com/develop/ui/compose/performance/stability/strongskipping>, <https://developer.android.com/develop/ui/compose/performance/bestpractices>

No per-frame composition reads, cross-phase writes, or offscreen infinite transitions were observed. Navigation uses `AnimatedContent`; lists have explicit stable item keys. Paging is absent.

References: <https://developer.android.com/develop/ui/compose/animation/composables-modifiers>, <https://developer.android.com/develop/ui/compose/lists>

The existing `app/build.gradle.kts:41` retains `isMinifyEnabled = false`; there is no baseline profile or measured release-frame profile. This limits release performance hygiene evidence. R8 can be assessed separately with keep rules and a production-path validation plan; it is not enabled speculatively by this UI change.

References: <https://developer.android.com/topic/performance/app-optimization/enable-app-optimization>, <https://developer.android.com/develop/ui/compose/performance/baseline-profiles>

### State Management — 9/10

`RelayViewModel.kt:47` exposes `asStateFlow()`; state is updated through immutable copies. `MainActivity.kt` uses `collectAsStateWithLifecycle()`. Tabs, expanded sections and per-tab scroll state use saveable state; tokens are intentionally not serialized into SavedState or browser storage. Saving configuration is an explicit user action. Diagnostic results are structured so language changes re-render labels.

References: <https://developer.android.com/develop/ui/compose/state>, <https://developer.android.com/develop/ui/compose/state-saving>

The assessment is limited by the small state surface; process-death UI tests are not present. No paging load-state surface was observed.

### Side Effects — 9/10

`MainActivity.kt` collects service state only while STARTED. `RelayViewModel.kt:74` performs network work only after a user action. `RelayViewModel.kt:84` serializes existing Keystore diagnostics; `onCleared()` calls `shutdown()`, never interrupting an in-flight real TEE call. Composables do not start network or TEE calls during composition. System-bar appearance is published through `SideEffect`.

References: <https://developer.android.com/develop/ui/compose/side-effects>, <https://developer.android.com/topic/libraries/architecture/coroutines>

Real service tasks and Keystore diagnostics were not run during this audit. Their runtime correctness is outside the UI preview evidence.

### Composable API Quality — 9/10

`RelayScreen.kt:57` exposes data and callbacks as a stateless screen boundary. Shared `OmmegaCard`, `ConnectionBadge` and `ChoiceRow` accept an outermost `modifier`; `OmmegaCard` has a trailing content slot. Theme colors are consistently consumed by components. Theme and app-locale work is wired at the Activity boundary.

References: <https://developer.android.com/develop/ui/compose/modifiers>, <https://developer.android.com/develop/ui/compose/designsystems/anatomy>

These are app-internal APIs, not public-library interfaces. There is no evidence requiring further API abstraction.

## Prioritized Fixes

1. **Resolved:** separate connection probing from saved configuration — `RelayViewModel.kt`, `ServerClient.kt`. References: <https://developer.android.com/develop/ui/compose/state>
2. **Resolved:** use one lifecycle-aware StateFlow for service state — `MainActivity.kt`, `RelayService.kt`. References: <https://developer.android.com/kotlin/flow/stateflow-and-sharedflow>
3. **Resolved:** render translated diagnostic labels from structured state — `RelayViewModel.kt`, `RelayScreen.kt`. References: <https://developer.android.com/develop/ui/compose/resources>
4. Optional future work: evaluate R8 and baseline profiles against an explicitly authorized production validation plan.

## Notes And Limits

- Compiler diagnostics used: **yes**, Release and Debug. Named-only and module-wide metrics are available. The Strong Skipping ceiling table imposes no binding cap; qualitative evidence controls the score.
- UI preview tested on A with Mobile MCP: Chinese adaptation, English switch, dark/light appearance and bottom navigation. Preview service and boot receiver are manifest-disabled. This does not validate B deployment or live TEE behavior.
- Two native JVM tests cover configuration validation and overview URL credential redaction. Compose instrumentation, frame timing, process-death and screenshot-regression tests are not present.
- Accessibility: labeled native controls, password masking and system text scaling are implemented. This is adjacent coverage, not an accessibility score.
- No KMP, Paging, TV or Wear surfaces observed. Navigation is three local tabs, so a navigation framework is unnecessary.
- Default weights 35/25/20/20. Medium confidence reflects the small module and absence of release performance traces.

## Suggested Follow-Up

Production WebView acceptance can use the normal user app path after the relevant device deployment approval. Frame timing and R8/baseline-profile work should be separate from this UI update.
