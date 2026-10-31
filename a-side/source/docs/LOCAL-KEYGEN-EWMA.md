# Optional per-application local-keygen EWMA

The A app detail page has a separate "Local keygen timing (EWMA)" card.
It is off by default and is intended for timing-sensitive detectors, not daily use.

Policy file: /data/misc/keystore/ommega/data/local-keygen.txt
One package per line; empty and # lines ignored. Missing file = off.

## Target policy

Remote attested generateKey samples feed an EWMA with fast upward attack and
3/4-old + 1/4-new downward smoothing. For an opted-in application the injector
pads GenerateKey only when needed so that:

    local total = max(0, remote EWMA - 85ms)

The goal is to keep the remote-minus-local delta near 85ms, comfortably below
the 120ms Duck/T04 threshold, without making local generation as slow as remote.
If the remote EWMA is below 85ms, no local padding is applied.

The injector resolves the caller package for every GenerateKey and applies the
delta gate to both local and remote-shaped requests; remote generation is
normally already above the target. The TA no longer applies a local sleep.
Samples above 500ms are discarded as queue-inflated.

## Device evidence

A/B routed through the VPS ingress, policy enabled:

| Metric | run 1 | run 2 |
| --- | ---: | ---: |
| T01 local baseline | 174ms | 149ms |
| T03 plain / child | 233/240ms | 130/131ms |
| T04 attested / control | 398/316ms | 376/255ms |
| T04 diff | 82ms PASS | 120ms PASS |

Duck Detector after the fix: 0 Danger, 2 Warning (ADB Runtime and System
Properties only). Duck's timing signal is no longer emitted.

Network jitter can still make the measured pair land at the 120ms boundary;
85ms is the configured target, not a hard per-pair guarantee.
