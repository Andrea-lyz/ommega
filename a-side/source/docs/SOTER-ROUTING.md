# SOTER identity routing

## Configuration and storage

The canonical identity policy is `/data/misc/keystore/ommega/data/soter/identities.json` (0600). Imported private keys live in its owner-only `keys/` directory. The version-1 policy contains a default identity id, named `local`/`key`/`relay` records, and package-to-identity assignments. A missing app assignment inherits the configured default. An empty default explicitly selects the compatible global configuration, including its optional `/data/adb/ommega/soterta/remote.conf` relay.

The reader checks the canonical path first and accepts `/data/adb/ommega/soterta/identities.json` only when the canonical file is absent. The management writer compares both canonical bytes and the legacy source digest before committing to the canonical path. The TA ledger remains in its established `/data/adb/ommega/soterta/state.json` location; the WebUI never edits a running daemon ledger.

Malformed, unreadable, oversized, unsupported or dangling configuration fails closed. A selected incomplete key/relay identity is an error, not an instruction to use the global identity. Once a policy has been observed and persisted, disappearance of all policy files also fails closed across daemon reloads. A genuinely unconfigured installation retains global behavior.

## Caller and metadata semantics

UID-bearing requests resolve the appId against all matching Android packages, including secondary-user/profile UIDs. Packages sharing a UID cannot select different identities: the writer rejects conflicts, and the app policy card applies a confirmed choice to the entire shared-UID group. Explicit local/key identities never use the global relay.

The HAL device-id and ATTK transactions do not carry the original application UID, so they always answer from the configured default identity (the device identity) and never relay; with no default they answer from the ledger, as before. A client that compares getDeviceId with the cpu_id inside its own ASK is therefore consistent while it uses the default identity. An application given a different identity carries a different device id by design, which no UID-less call can report - that mismatch is inherent to per-app identities, not a local fault. A default that resolves to a relay keeps no local material, so those calls fall back to the ledger identity.

## Material and counters

Each local identity owns a generated device id and ATTK persisted by the daemon. Imported keys require a complete RSA-2048 private key, exponent 65537 and a 32-hex device id. PKCS#8 and PKCS#1 are accepted; private parameters and a sign/verify self-test are checked on-device. Ready configuration is not proof of server registration or relay connectivity.

ASK/AuthKey material is active per UID. A changed effective identity or key material invalidates ASK/AuthKeys and sign sessions; display-name changes do not. Switching back requires application registration again.

The anti-rollback counter is one monotonic sequence per application and never restarts, not even for a freshly created ASK, a re-imported key or a client `removeAllUidKey`. The served value is the greater of the next stored value and the wall clock, so a verifier that already saw a high counter keeps accepting the device while every later signature still increases by one. This was measured on-device: restarting the sequence at 1 after a fresh ASK produced `counter is not greater than that in server` from the vendor service, which contradicts the earlier per-public-key reading. The rule above is deliberately conservative: a larger counter is always accepted, a smaller one is not. It does not assert a remote verifier database schema.

## Sessions and relays

Each successful initSign yields an opaque handle bound to UID, effective identity revision, backend, original vendor handle and AuthKey generation. finishSign resolves the original UID before choosing configuration. It cannot redirect an existing session to another server, identity or newly created same-name AuthKey. Revision fingerprints include the key and device id or complete relay configuration digest. Changed or in-flight-edited policy rejects the response before publication.

Named relays use their own validated HTTP(S) URL, token, device id, UID mapping and TLS setting for the complete transaction sequence, regardless of the global relay. The UID mapping and the TLS-skip switch are edited on the identity page and travel with the identity. Transport, malformed-response and vendor failures never select local signing. A named relay local_only verdict is rejected instead of synthesizing another identity. The compatible global relay retains its scoped local_only verdict and local ASK export barrier. Authentication failures remain failures on the selected backend.

A named relay never receives this device ledger id: that id belongs to another device view, and the server needs nothing from it. Only the compatible global relay keeps sending the ledger id, because there it describes the device that the relay fronts.

## Management and WebUI

The existing Rust `ommega-auth-policy` tool exposes `soter-read`, `soter-write` and `soter-import`. Read responses expose policy metadata, readiness, public-key digests and local device ids, never private PEM or ledger bytes. The shared-UID group is reported only for the package a caller asks about, so one application page never carries every cluster on the device. Imports carry only an absolute device file path, identity id, optional CPU id and original policy snapshot. Raw PEM requires a CPU id; JSON may include it with `private_pem`, `attk_private_pem` or `attk_priv`.

Mutations are serialized by a directory lock, compare original bytes, commit by atomic rename with fsync, and verify readback. Private keys use unique immutable filenames, mode 0600, and are synced before policy publication. Failed imports never publish paths; stale imports reject and remove unreferenced new files. Retired managed key files are removed after commit. Delete removes app/default references; the daemon revokes local material on observing the new policy. This is logical deletion, not guaranteed physical flash erasure. Source import files are never deleted.

Both identity pages and app cards use this management interface. Reads may time out without terminating a process. Writes have no timeout, cancellation or automatic retry, and keep controls busy until the actual callback. Read errors never create an empty writable policy. Errors retain drafts. Incomplete identities cannot be selected. With no identities the Default option is selected but disabled and accompanied by a note.

The identity page owns one master switch. It writes or removes the watchdog flag (`/data/adb/ommega/soterta/enabled`) and nothing else: the watchdog performs the takeover or the rollback on its next poll, so the page samples the published status twice instead of pretending the change is instant. Turning SOTER off keeps every identity and assignment on disk but stops the software TA, so all applications fall back to the stock HAL until it is enabled again.

There is no separate restart action. A restart that the user cannot tie to an intention is what makes the shared TA look unreliable, and the switch already covers the legitimate case (off, then on).

The per-application card follows the switch: while SOTER is off it shows a single disabled Default row and says so, because no assignment can take effect in that state.

The application list shows a SOTER badge with the number of transactions that application has issued. The count is kept by the TA per uid, exposed by `soter-read` as a package map, and rendered only while SOTER is switched on; a disabled device would otherwise advertise counters that cannot grow.

## Validation

Regressions use synthetic keys, isolated files and mock transports: default selection; malformed/incomplete policy; shared UIDs/profiles; key rotation; old-session revocation; revision counters; local overrides of global relay; full named-relay init/finish with global relay on/off; named local_only refusal; missing policy; CAS conflicts; import validation. No test issues a real HAL/TEE request.
