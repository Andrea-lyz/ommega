# Isolated grant scope

An Ommega grant to an isolated UID delegates the caller's Keystore2 scope to that
isolated process. This applies in list and global scope without package-name
rules. Ordinary isolated processes, SDK sandboxes and native services remain
outside application-global scope.

The grant transaction stores its process binding in the existing database:
Android UID, anchor PID, process start time and kernel boot ID. The oldest live
process with that isolated UID anchors the binding; its forked helpers may use
the same grant while the anchor remains alive. Reusing a UID or PID, or booting
again, does not reactivate the delegation. A new grant to a new process instance
expires grants from previous instances of that UID.

Before routing an isolated caller, the injector asks whether its real Binder UID
and PID have an active process-bound grant. The request's alias, GRANT ID and KEY_ID
do not participate in this scope decision. Every enabled Keystore2 surface uses
the same effective caller scope, including native and synthetic security-level
Binders. Disabling all interception switches disables delegation routing too.

Delegation does not replace a caller's UID or SID and does not grant key permissions.
Keystore2 continues to enforce the grant's access vector, including GET_INFO and
USE. Revoking the last grant or deleting its key removes delegated scope. The
binding is kept in the database across daemon reloads; a dead anchor cannot
authorize a new process. Lookup failures return SYSTEM_ERROR, except confirmed
Ommega transport unavailability, which follows the existing System fallback.

The private Binder readback path uses the Keystore2 Binder explicitly passed by
the grant owner. An isolated process's public Java API can still be unable to
resolve Keystore2 through ServiceManager; delegation does not widen service
discovery policy.
