# Optional per-application local-keygen EWMA

The A app detail page has a separate "Local keygen timing (EWMA)" card.
It is off by default and is intended for timing-sensitive detectors, not daily use.

Policy file: /data/misc/keystore/ommega/data/local-keygen.txt
One package per line; empty and # lines ignored. Missing file = off.

TA handles challenge-bearing local generation via the caller AAID.
Injector handles no-challenge local generation via caller UID package resolution.
Remote attested generation is never delayed. Successful remote samples feed a
global 3/4-old + 1/4-new EWMA; targets below 50ms are ignored and samples above
250ms are discarded as queue-inflated.

Device evidence with aprobes.agap enabled on the home-router path:
T01 126/118ms, T02 112/111ms, T03 plain/child 119/218 and 115/233ms,
T04 115/117 and 122/116ms, diff -1/5ms. After removing the policy file,
T01 returned 24ms, T03 28/38ms, T04 114/36ms (diff 77).
