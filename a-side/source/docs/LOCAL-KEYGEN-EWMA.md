# Optional per-application local-keygen EWMA

The A app detail page has a separate "Local keygen timing (EWMA)" card.
It is off by default and is intended for timing-sensitive detectors, not daily use.

Policy file: /data/misc/keystore/ommega/data/local-keygen.txt
One package per line; empty and # lines ignored. Missing file = off.

## Target policy

Remote attested generateKey samples (challenge, no attestation key) feed a
global 3/4-old + 1/4-new EWMA. For an opted-in application, the injector pads
local GenerateKey only when needed so that:

    local total = max(0, remote EWMA - 110ms)

The goal is to keep the remote-minus-local delta near 110ms (under the 120ms
Duck/T04 threshold), not to make local generation as slow as remote. If the
remote EWMA is below 110ms, no local padding is applied.

All local GenerateKey paths are handled by the injector, including no-challenge
keygen and challenge+ATTEST_KEY local generation. The TA no longer applies a
local sleep. Samples above 250ms are discarded as queue-inflated.

## Device evidence

Policy enabled on the home-router path:

| Metric | value |
| --- | ---: |
| T01 local baseline | 28ms |
| T03 plain / child | 28/37ms |
| T04 attested / control | 142/37ms |
| T04 diff | 105ms PASS |

Policy file removed (default off):

| Metric | value |
| --- | ---: |
| T01 | 18ms |
| T03 plain / child | 31/42ms |
| T04 attested / control | 131/31ms |
| T04 diff | 100ms PASS |

The 110ms target is intentionally below the 120ms detector threshold; EWMA
smoothing still means a sudden remote latency jump takes several samples to
converge.
