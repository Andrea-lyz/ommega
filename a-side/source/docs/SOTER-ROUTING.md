# SOTER routing

SOTER has its own optional relay configuration in
`/data/adb/ommega/soterta/remote.conf`, separate from KeyMint authentication-signing
policy. The configuration selects the initial backend. No application package
name is hardcoded by A to select local signing.

An explicit relay `local_only` verdict selects the local ledger for the original
A UID. This choice covers ASK, AuthKey and sign initialization, and persists in
the existing `state.json`. It is scoped to the server URL, token, device ID, UID
mapping and TLS setting; only a configuration digest is stored. A network error,
malformed response or vendor failure does not select local signing.
If an active relay's configuration disappears, that failure remains closed
across a daemon reload. A saved, explicitly disabled configuration selects local
mode.

A newly local UID must receive a successful local ASK export before AuthKey
checks, creation, export or signing can succeed. Until then those AuthKey
operations return the existing no-key failure. `hasAsk` continues to report the
local ledger's actual key presence, including immediately after generation.
Generating an ASK alone does not clear the export barrier. Local ASK/AuthKey
material remains software-generated and is not a
hardware assertion by the remote device.

Every successful `initSign` receives a unique A-side opaque handle bound to its
UID, backend and original vendor handle. `finishSign` resolves that binding;
matching numeric handles on different backends cannot redirect an operation.
Unknown handles and relay refusals of remote sessions fail without local
signing. Remote authentication failures remain remote failures. Routing handles
survive a daemon reload with the ledger and are invalidated when its relay
configuration changes. The oldest abandoned bindings are pruned at 1024 entries.

An operation with no UID cannot establish a per-application local policy.
`getDeviceId` therefore fails if its relay replies `local_only`, instead of
returning another device identity. The ATTK transaction trio remains local and
does not participate in the relay protocol.

After a relay changes an application's backend, its existing remote ASK cannot
authorize local AuthKeys. The application must initialize and obtain the local
ASK again before continuing; A never rewrites a remote key or its signed data.
