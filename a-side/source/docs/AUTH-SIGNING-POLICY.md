# Authentication-signing policy

The WebUI's **Settings > Authentication signing** entry controls new remote EC/RSA
authentication-signing keys. It has three modes:

| Mode | New eligible key |
| --- | --- |
| Auto (default) | Preserve the requested authentication policy for a recognized probe (relayed to B); certify other eligible requests locally on A. |
| Payment compatibility | A certifies the key locally with its own keybox and enforces the authentication token itself; the certificate describes A's real policy. |
| Preserve authentication | Send the original authentication requirements to B. B's hardware key is authentication-bound, so a signature over the A-authorized transport can still fail with -26. |

The compatibility path keeps the caller's authentication policy and certifies
the key locally: A's keybox signs the chain and A's KeyMint enforces the token the
app produced, so USER_AUTH_TYPE, AUTH_TIMEOUT and the absence of
NO_AUTH_REQUIRED describe the key exactly as the app requested. The existing A
authorization path checks the HAT MAC, user/authenticator ID, allowed
authentication type and operation challenge or timeout; password fallback is
supported. No relay request is made for these keys, so B never holds an
authentication-bound key it cannot use.

This policy applies only with remote mode enabled, an attestation challenge,
no caller-supplied attestation issuer, a user secure ID and nonzero auth type,
and purposes limited to SIGN/VERIFY. Decrypt/agreement, attestation issuers,
trusted presence, trusted confirmation and on-body requirements retain their
existing behavior. Requests without a challenge retain local generation.
Remote unavailability for eligible challenged keys is an error in every mode;
it never silently switches the key to a local keybox.

## Token verification boundary

Before a system HAT is re-MACed with Ommega's local key, its shape and original
MAC are verified through A's native KeyMint verifier. This applies to direct and
mirrored ingestion. System addAuthToken success means caching, not a TEE MAC
verdict. A mirrored verification/localization failure is logged and dropped from
Ommega while preserving the successful System result; a direct failure is returned.
Invalid tokens never reach ENFORCEMENTS through this default path.

The existing main.force_skip_system_biometric_hat_verification setting is an
explicit insecure override, defaults to false and emits a warning when used. It
is not automatically enabled for mirrored calls. Turning it on removes the original
MAC guarantee and must not be counted as passing the strict authentication gate.
The verifier may generate/cache a native A-side HMAC verification key; it never
contacts B. Deployment requires A-side authentication/lockscreen regression tests.
No credential, CE super-key, request-flag or key-storage policy is changed.

## Creation policy

Auto makes a decision for each new key. Its known authentication probe must
match the complete TrustAttestor creation contract: alias
`TrustAttestor_silent_auth_<UUID-v4>`, EC P-256, SIGN/VERIFY, SHA-256, auth type 3,
a 32-byte attestation challenge, per-operation authentication (timeout absent
or zero), and no unlocked-device requirement. The general eligibility checks
above also apply. A probe name alone or generic biometric key parameters do not
select preservation. Unknown patterns use payment compatibility. Recognition
does not use a package-name list or classify all keys belonging to one app.

Preserve always sends the original requested authentication policy to B.
Payment compatibility always uses A-authorized B signing for eligible keys,
including recognized probes. These explicit choices override Auto recognition.
B's real certificate is returned unchanged in every case. A hardware-bound B
key cannot be authenticated with A's HAT; preserved keys can therefore return
-26 after A authentication. Probe acceptance does not establish hardware user
authentication guarantees for the other, compatible keys.

The decision is resolved once per creation and reused through retries. There is
no mutable global or thread-local request mode. Existing keys are not changed.

## Scope and exceptions

Scope first decides whether a caller reaches Ommega. An app exception overrides
the default auth-signing mode only for callers already in scope. Exceptions
remain saved and effective when global scope is enabled, and do not enroll an
out-of-scope app. Core system-service exclusions remain unchanged.

Package identities for authentication signing come from Keystore's caller AAID
at key creation. In global scope, the separate takeover-mode policy resolves a
caller package only for `getSecurityLevel` when explicit modes are configured;
the scope decision itself does not depend on that lookup. For a shared UID, all
package records in the AAID are considered. If explicit overrides
conflict, Preserve wins over Auto, which wins over Compatibility; packages
without an override inherit without canceling an explicit override.

The Apps page places **Global scope** below its search bar. Search uses a native
text input outside the legacy header, with its own controller and text-selection
styles. The retired vendor editor is disabled and hidden. Android WebView does
not give a document-parsed input a text-editor connection, so the field is
recreated once when the Apps view opens, before the user touches it, and focus
is always left to that touch; preparing it during the tap loses the connection.
Navigation does not focus an editor or request the soft keyboard. App-list
refreshes notify the search controller without reading or writing editor text;
filtering waits until IME composition finishes. Search matches app names and
package ids, not policy or certificate badges. Only the explicit Clear action
resets the search text.
App names come from the manager's injected package bridge. Android WebView only
answers a bridge method called through its injected object, so every label query
keeps the live `ksu` receiver; the page falls back to the packaged
`applist.json`, then to the package id, when the bridge is unavailable.
Each app has an individual policy page with Auto, Payment compatibility and
Preserve authentication choices. An app with no explicit override displays its inherited global default;
choosing a mode writes only that app's exception and preserves the global default
and all other exceptions. The policy page remains accessible in global scope.
Browser history, native WebView back events, Escape and the Back button return
to the application list.
The **Positive key ID compatibility** switch is a separate global setting in
Settings and is saved independently of the application scope list.

## Storage and saving

The policy file is `/data/misc/keystore/ommega/data/auth-signing.toml`. The manager
reads the same file through `/data/adb/ommega/ommegadata/data/auth-signing.toml`:

```toml
version = 1
default = "auto"

[apps]
"com.example.app" = "preserve"
```

Allowed modes are `auto`, `compatibility` and `preserve`. No file means Auto
without exceptions. Unsupported versions, malformed files and unreadable
configurations cause an explicit error for affected new-key requests; they
never silently select a different policy. Parsing is cached by file metadata,
and atomic replacements are observed on subsequent key creation without a
service restart. Neither TEE nor StrongBox reads a separate copy.

The WebUI uses the packaged Rust `ommega-auth-policy` writer. It validates the
replacement, locks the data directory across the source-byte comparison and
atomic rename, preserves metadata, fsyncs the file and directory, and rejects
stale input or a concurrent editor. Locks release when the process exits. Its
operations use fixed destinations and do not initialize Binder, HALs or a daemon.
The same executable separately writes the app scope, system-app list and security-mode files;
its auth operation writes only the auth policy. The UI verifies the saved
policy by reading it back. Other config writers and
global-scope changes do not rewrite this file. Read/parse/save failures remain
visible and do not report successful payment or detector validation.

Only newly created keys use changed policy. Existing keys keep their keyblob,
public key and B policy; they never change backend or regenerate after an
authentication failure. Re-enrollment through an application's normal flow may
be needed to create a new key and register its public key. Do not clear system
Keystore or GMS data as a policy update step.

## Trust boundary and validation

Authentication in compatibility mode is enforced by A's software service, not
B's hardware. The existing relay transport authenticates its client, but B and
the server do not independently verify A's HAT or a fresh per-signature A
approval. A relay credential holder can access a B no-auth key outside A's
authorization path. This mode is not end-to-end hardware user authentication.

A detector's probe result does not establish the compatibility key's hardware
authentication guarantees. Authenticated signing, detector acceptance and
wallet/payment acceptance require separate device tests. This A-only policy
needs no B-module or server update.
