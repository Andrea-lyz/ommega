# Experimental A authorization / B signing

Build with `python build.py --release --abi arm64-v8a --experimental-remote-auth-signing`.
The archive and module version include `auth-a-sign-b`; the module name includes
`Auth A / Sign B EXP`. Ordinary builds leave the experiment disabled.

For new EC/RSA signing keys with a user secure ID and an authentication type,
A retains the caller's authentication policy in its encrypted keyblob. Its
existing authorization path checks the HAT MAC, user/authenticator ID, allowed
authentication type, operation challenge or timeout before contacting B.
Password fallback and timeout-based authentication are supported by that path.
B receives NO_AUTH_REQUIRED instead of A's user authentication tags. Requested
UNLOCKED_DEVICE_REQUIRED is preserved on B. The physical private key and signing
remain on B. Remote unavailability is an error, without local-keybox fallback.

This applies without package-name exceptions, including Google Pay. Requests
without an attestation challenge retain the ordinary local generation path.
Caller-supplied attestation issuers, mixed decrypt/agreement purposes, trusted
presence and trusted confirmation retain their existing behavior.

B's certificate is returned unchanged and truthfully reports B's no-auth
policy. The authorization policy exposed by A's virtual KeyMint is enforced by
A's software service, not by B's hardware. A verifier requiring hardware user
authentication tags 504/505 may reject the key. Wallet and detector acceptance
must be measured separately from successful authenticated signing.

This experiment reuses the existing authenticated relay transport. B and the
server do not independently verify A's HAT or a fresh per-signature A approval.
A relay credential holder can access a B no-auth key without A's authorization
path. This is a compatibility experiment, not an end-to-end hardware
authentication or new remote authorization protocol.

Existing B-auth-bound keys keep their original policy and can still return
-26. They are not replaced or migrated. Test using a fresh repro-app key, or
the application's own disable/re-enable fingerprint enrollment flow. A new
public key may require registration by the application. Do not clear GMS or
system keystore data as part of this experiment.

The package contains only A changes. It needs no B module or server update.
Installing the known-good A package restores ordinary generation behavior;
experiment-created keys keep their no-auth B policy until the application
deletes and recreates them. Keep the original package for rollback.
