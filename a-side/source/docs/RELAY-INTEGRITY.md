# Relay attestation integrity and performance

Remote key creation validates the returned challenge, application ID, security
level, KeyMint profile and version authorization tags. Certificate chains must
contain 2 through 16 DER certificates in leaf-to-root order. Each adjacent issuer
name and signature is checked, including the final root's self-signature. Native
caller-selected ATTEST_KEY delegation additionally requires the expected public
key, issuer name and exclusive hardware ATTEST_KEY authorization.

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
Timing probes may continue to observe this difference; a compile result or the
absence of a local sleep is not a claim that a remote RTT falls below a threshold.

A keeps pooled HTTP clients per TLS verification policy. At debug level,
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
