# Relay attestation integrity and performance

Remote key creation validates the returned challenge, application ID, security
level, KeyMint profile and version authorization tags. Certificate chains must
contain 2 through 16 DER certificates in leaf-to-root order. Each adjacent issuer
name and signature is checked, including the final root's self-signature. Native
caller-selected ATTEST_KEY delegation additionally requires the expected public
key, issuer name and exclusive hardware ATTEST_KEY authorization.

## Native-version admission

A's immutable target comes from read-only native System/Vendor firmware and
Boot AVB/header sources, not mutable properties or B's advertised dates. Required
unknown native fields fail closed. A remote profile is eligible only with
`version_policy = a-native-v1`, a valid `version_revision` and an exact complete
match for OS_VERSION (705), OS_PATCHLEVEL (706), VENDOR_PATCHLEVEL (718) and
BOOT_PATCHLEVEL (719). Vendor keeps its full day, and Boot keeps native raw
precision, including a header's YYYYMM00.

The request carries A's four-tag target and `expected_version_revision`. A
compatible B must reject stale or mismatched state before its normal HAL call
using that same revision; accepting a new snapshot after admission is not
permitted. A also checks the returned signed certificate and authorizations
against the admitted target. B must adapt to A through separately authorized
B-side work; this contract never lowers or rewrites A's SPL. An old B or a relay
without the capability is rejected until separately upgraded. Remote StrongBox
is not admitted using a TEE profile, and `server_keybox` cannot establish this
physical-B version guarantee.

A's encrypted software envelope and B's real hardware keyblob are different
objects. Matching the profile or a certificate does not relax software-blob
monotonic version checks, downgrade B's ratchet or guarantee hardware rollback
resistance. Routing remains all-or-nothing for the current allowed caller, not
selected by descriptor, key origin or version tags.

Keyblob version checks run under the per-security-level TA lock and read only
the B profile already cached for the current relay route. The TEE wrapper
fetches that profile before taking the lock; one fetch runs at a time and
concurrent callers wait for it outside the lock. A failed fetch is retried for the same route and
configuration after 2 s, doubling to at most 30 s, and a route or configuration
change retries immediately. Without a cached profile, local keys keep the native
A snapshot when `fallback_local` is enabled and otherwise fail with
`HARDWARE_NOT_YET_AVAILABLE`. Remote `generateKey` validates and prepares the
request under the TA lock, releases it for the relay attestation call, then
relocks to verify the result and seal the keyblob, or to apply the local
fallback rules.

The UI reads `/data/misc/keystore/ommega/data/spl-status.json` only. It does not
fetch a profile or invoke a HAL. Firmware, observed Android properties, frozen
A TA configuration, cached B profile and signed-certificate evidence are separate
sources; absent TA/certificate observations remain unknown. A matched cached
profile, process status or `getprop` is not fresh physical TA acceptance.

Only A local-mode deployment is currently planned; no B deployment, reload or
device execution is authorized. All A SPL/OS configuration is `auto`/native-only;
explicit overrides require human migration review with original config and keys
preserved, not database clearing. Non-SPL verified-boot controls remain separate.
This contract is not a Wallet, payment, compatibility or hardware guarantee. See
[Software keyblob version checks](KEYBLOB-VERSIONS.md).

## Request and certificate checks

Generation parameters are validated locally before a request reaches the relay.
After chain and record verification, the leaf's public-key family and the signed
algorithm, key size, EC curve and RSA exponent are checked against the effective
B request when supplied. Hardware authentication tags 503/504/505 must match the
request, including timeout absence; they cannot move to the software list. Signed
presence/confirmation/unlock constraints must match in one enforcement list.
Requested public product IDs must match hardware-list bytes. SID 502 is not
required in an attestation and is not fabricated. Purpose/digest/padding/MGF tag
parity is not added here without a verified vendor baseline; existing forwarding
and operation checks remain in place.

An explicit A-authorized compatibility key is checked against the no-auth policy
actually sent to B, not against a false claim that B enforces A's HAT. Preserve
requests cannot accept a validly signed no-auth substitute. An empty, malformed
or mismatched response is an error and is never treated as transport unavailability
or permission to mint a local replacement. No returned certificate is rewritten.

Supported certificate signature families are ECDSA P-256/P-384, RSA PKCS#1
SHA-256/384/512 and Ed25519, reusing the native delegation verifier. Unsupported
signature families fail explicitly. Integrity checks do not by themselves select
a trusted root, establish manufacturer identity, implement a revocation service,
or prove certificate validity against trusted time. Those policies remain outside
this chain-integrity check; a self-signed arbitrary root is not made trusted.

By default, local key creation and local caller-supplied attestation issuers do
not wait for a remote latency sample; there is no EWMA-based sleep unless an
application is explicitly listed in the optional per-app local-keygen policy.
See LOCAL-KEYGEN-EWMA.md. Remote attested creation still pays real network,
queue and hardware execution costs.
This EWMA policy is independent of software-TA certificate-work accounting.
Every successfully encoded local attestation leaf pays one sampled 24-40ms
issuance cost in addition to the ordinary key-creation cost, before key sealing
and keystore publication. This includes a warm keybox, a caller-selected local
issuer, local authentication-compatible signing and permitted remote fallback.
Self-signed certificates, failed certificate generation and remotely returned
attestation chains do not pay this additional cost. No caller name, alias or
remote latency sample selects the cost; no attestation fields or key policies
are changed. This is a bounded software timing model, not hardware execution
or a guarantee about application-level scheduling or remote RTT.

A verifies the relay server certificate before sending `X-Relay-Token`.
`remote.tls_pin_sha256` accepts only a server whose certificate
SubjectPublicKeyInfo SHA-256 is pinned, and the TLS handshake signature is
checked against that key; without a pin the certificate must chain to the
bundled Web PKI roots and match the host name. `remote.tls_insecure` disables
verification only when set explicitly and no pin is configured. An invalid
pin or an unreadable config fails the request instead of downgrading.

A keeps one pooled HTTP client for the current TLS policy. At debug level,
`event=relay_roundtrip` reports the endpoint, request ID, returned task ID,
attempt count, monotonic `elapsed_us`, HTTP status and decode outcome. The
timer includes transport, server/B waiting, retry backoff and JSON decoding;
it excludes the later certificate validation and keystore storage. It does
not contain tokens, challenges, key material or new on-device persistent files.
Transport exhaustion retains the existing health/error path.

B holds idle polls for up to 20 seconds with a 25-second request deadline.
Task arrival wakes the held poll; a fast empty response retains an 80ms floor.
Server results wake the exact A task, not all B polls. These transport choices
do not weaken the chain/policy checks described above or change key routing.

Software-only regression fixtures cover valid EC/RSA chains, tampered signatures,
reordered or truncated chains and rejection before remote-key storage. They use
no Binder, network or physical TEE. Android cross-compilation is mandatory, and
on-device execution remains a separate gate.

## Remote key deletion

When keystore2 garbage collection calls `deleteKey` for a remote
(`KeyMaterial::Remote`) keyblob, the TA cannot decrypt the blob to recover the
remote alias because hidden parameters are unavailable at delete time. The TA
therefore reports the SHA-256 digest of every serialized remote blob it emits
or sees, and of every blob passed to `deleteKey`, to a persistent registry at
`data/remote-blobs.db` (under the standard ommega data directory). Each digest
maps to the remote alias and the configured B device id. Registration happens
at remote key generation, at remote blob upgrade, lazily at `beginOperation`
for keys created before the registry existed, and is a no-op when remote is
not configured. Release happens unconditionally at `deleteKey`; an unknown
digest is a no-op, so local keys are unaffected.

An alias is scheduled for remote deletion only when its last registered blob
digest is released. Because `upgradeKey` emits a new blob for the same alias
and registers it before keystore2 collects the superseded blob, a superseded
`deleteKey` cannot tear down the still-live remote session. Re-registering an
alias deletes any outstanding pending row, so a live reference always wins.
Every failure mode fails toward not deleting: registry open/write errors are
logged and swallowed, and a missed registration only means a session is never
removed.

Queued deletions are drained by the `ommega-remote-gc` worker thread, which
runs only while the remote relay is configured and never holds the TA lock
during network I/O. It POSTs `/api/delete/` with a deterministic
`delete:<alias>` request id so retries collapse into the same server-side
task. A server answer of `deleted: true` or `false` completes the row; a
validation rejection drops it permanently; every other failure (unreachable
relay, an old server without the route, an old B relay without the task type,
malformed response) retries with `min(60s * 2^attempts, 6h)` exponential
backoff. Rows pending for more than 30 days are dropped rather than retried
forever; the stale session then simply remains on the remote side.

On the relay server the request is strict per backend: `server_keybox` mode
removes the alias from its own session map and persists `sessions.json`;
`physical` mode enqueues a short idempotent `delete` task to the named B
device. The B relay removes the alias from its session cache and deletes the
session file under the same lock that serializes every session write, then
fsyncs the sessions directory. A missing alias is a successful no-op
(`deleted: false`). No KeyMint/HAL/TEE call is made on B: the persisted
keyblob becoming unreachable is what retires the key. An in-flight
sign/decrypt/agree that cloned the session before the delete cannot republish
it afterwards and fails with the normal "no key for alias" error; only a new
`attest` may recreate the alias. `deleteAllKeys` performs no remote action —
keystore2 deletes each blob individually through `deleteKey`.

Remote keys created before this mechanism have no registered digest, so their
first-use `beginOperation` backfill is what makes them deletable; a remote key
never used again after the update is never scheduled for deletion. Switching
between `physical` and `server_keybox` modes can leave sessions behind in the
other backend, because each backend tracks its own session store.
