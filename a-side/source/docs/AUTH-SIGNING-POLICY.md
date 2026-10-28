# Authentication-signing policy

The WebUI's **Settings > Authentication signing** entry controls new remote EC/RSA
authentication-signing keys. It has three modes:

| Mode | New eligible key |
| --- | --- |
| Auto (default) | Preserve the requested B authentication policy for a recognized probe; otherwise use payment compatibility. |
| Payment compatibility | A authenticates the operation; a physical no-auth key on B signs it. |
| Preserve authentication | Send the original authentication requirements to B. A's HAT cannot authenticate to B, so real authenticated operations can still fail with -26. |

The compatibility path retains the caller's original authentication policy in
A's encrypted keyblob. The existing A authorization path checks the HAT MAC,
user/authenticator ID, allowed authentication type and operation challenge or
timeout before contacting B. Password fallback is supported. B receives
NO_AUTH_REQUIRED in place of A's user-authentication tags, while requested
UNLOCKED_DEVICE_REQUIRED is retained. B holds the physical private key and
produces the signature. B's certificate is returned byte-for-byte and describes
B's actual policy; no authentication tags are synthesized in that certificate.

This policy applies only with remote mode enabled, an attestation challenge,
no caller-supplied attestation issuer, a user secure ID and nonzero auth type,
and purposes limited to SIGN/VERIFY. Decrypt/agreement, attestation issuers,
trusted presence, trusted confirmation and on-body requirements retain their
existing behavior. Requests without a challenge retain local generation.
Remote unavailability for eligible challenged keys is an error in every mode;
it never silently switches the key to a local keybox.

## Automatic recognition

Recognition applies to one key creation, not the whole application or UID.
The known TrustAttestor auth probe must match all of these conditions:

- Alias `TrustAttestor_silent_auth_<UUID-v4>`.
- EC P-256, SIGN/VERIFY, SHA-256, user-auth type 3, a 32-byte challenge.
- Per-operation authentication (timeout absent or zero), no unlocked-device
  requirement, and the general eligibility requirements above.

Unknown patterns use payment compatibility. Recognition does not use package
names, a generic random challenge or a generic auth type as detector evidence.
Probe changes require a rule update or an explicit per-app mode. The resolved
creation decision is carried in the internal request and reused through retries;
there is no mutable global or thread-local request mode.

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

The Apps page places **Global scope** below its search bar. Each app has an
individual policy page with Auto, Payment compatibility and Preserve authentication
choices. An app with no explicit override displays its inherited global default;
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
