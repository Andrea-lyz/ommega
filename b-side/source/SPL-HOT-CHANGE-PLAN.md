# Adaptive SPL hot-window integration plan

Status: proposed

Base: `master@a1bd830` after PR #15.

This plan turns the completed Qualcomm KeyMint lab experiment into a gated
production design. The lab result is stored locally under
`artifacts/spl-isolation-20261017/HOT-SPL-CHANGE.md`; that directory is not
tracked by Git.

## Goal

For an attestation task that requests A's current OS and patch dates, B must:

1. validate the complete target before entering a secure-state window;
2. exclude every KeyMint caller that could observe or mutate the temporary
   TA-global profile;
3. apply the target with one `0x2116` request;
4. mint exactly one hardware-backed chain through the stock KeyMint HAL;
5. restore and verify B's baseline before returning the result;
6. preserve B's properties, credential path, existing keyblobs and ability to
   serve future baseline requests without rebooting; and
7. fail closed with durable evidence if any step is ambiguous.

The production result must contain B's real hardware root and target dates. It
must not edit a certificate, substitute a software chain, weaken verification,
or change B's Android properties.

## Proven facts from the lab run

The following are already established on the authorized `PHB110` device and
should be reused as design inputs:

- direct `QSEECom_start_app` against `keymaster64` succeeds when the helper is
  launched through `/system/bin/linker64`;
- `0x0200 GET_VERSION` returned API `4.1`, TA `4.1012`;
- `0x0207 SET_VERSION` and one 37-byte `0x2116` configure frame returned TA
  status `0`;
- `0x2116` accepts exactly `OS_VERSION`, `OS_PATCH_LEVEL`, and
  `VENDOR_PATCH_LEVEL`; all three are required;
- Boot patch level is not writable through `0x2116`;
- a target profile of OS `160000`, System `202609`, Vendor `20260901` produced
  a real four-certificate chain with Boot `20251201`, and the chain verified to
  Google hardware root
  `cedb1cb6dc896ae5ec797348bce9286753c2b38ee71ce0fbe34a9a1248800dfc`;
- a manual five-minute window produced seven target sessions, all with the
  requested date shape and a valid chain;
- a later OS `170000` window was observed as passing by the operator; its new
  certificate exports still need independent archival verification;
- apply/restore complete-helper times were `16.958 ms` and `10.495 ms`; the
  single TA sends were `623 us` and `339 us`;
- no reboot, HAL reload, property write or service restart was used.

This proves the primitive. It does not prove full client exclusion, old-keyblob
continuity, crash recovery, or that the current relay is ready to use it.

## Existing interfaces and missing work

| Layer | Current state | Work required |
|---|---|---|
| A request | `remote.rs` already sends OS, System, Vendor and Boot metadata inside `device_attest_context` | Add an explicit capability requirement and tests for missing/malformed fields |
| Server | `fulfill.rs` parses and forwards the four date fields | Advertise/require B isolation capability; reject unsupported routing; allocate one isolated window per task |
| B relay | `handle_generate_attest` currently calls the ordinary HAL and ignores the requested SPL fields | Add target parsing, a single global transaction lease, and call the hot-window executor |
| Direct TA | Proven only in standalone lab helpers | Add a resident, bounded client that builds the exact 37-byte frame and never retries |
| Client exclusion | Lab froze Keystore2 and the already-frozen RKP app cgroup | Prove admission coverage for every relevant caller before deployment |
| Recovery | Lab restored baseline in a shell/helper flow | Add an fsynced state machine, startup repair path and fail-closed quarantine |
| Keyblob continuity | Existing pre-window keyblob was not signed again after the window | Make post-window old-keyblob signing and credential continuity acceptance gates |

## Target model

Introduce a typed structure rather than passing loose JSON values into the
KeyMint call:

```rust
struct RequestedSpl {
    os_version: u32,       // e.g. 160000 or 170000
    system_patch: u32,     // canonical certificate form, YYYYMM
    vendor_patch: u32,     // YYYYMMDD
    boot_patch: Option<u32>, // YYYYMMDD
}

struct SplBaseline {
    os_version: u32,
    system_patch: u32,
    vendor_patch: u32,
    boot_patch: u32,
}
```

Validation rules:

- OS version must be a two-digit Android major plus four zeroes.
- System patch must resolve to a valid six-digit YYYYMM value; an A-side
  YYYYMMDD request is normalized to YYYYMM only for certificate tag 706.
- Vendor patch must be a valid YYYYMMDD value.
- Boot patch is read-only through the lab primitive. It must equal the current
  baseline when supplied, otherwise the task is rejected.
- A missing target must follow an explicit policy. It must not silently use a
  fixed test value.
- Values are checked before the first TA write. A partial or unknown frame is
  never sent because the TA may have already written earlier map entries before
  rejecting a later one.

## Secure-state transaction

Add one B-side state machine and one process-wide lease:

```text
IDLE
  -> LEASED
  -> CLIENTS_EXCLUDED
  -> BASELINE_RECORDED
  -> TARGET_APPLIED
  -> KEYMINT_RETURNED
  -> BASELINE_RESTORED
  -> CLIENTS_RELEASED
  -> RESULT_POSTED

any uncertainty
  -> RECOVERY_REQUIRED
  -> (deterministic baseline restore + read-only verification)
  -> QUARANTINED or IDLE
```

Required properties:

- Only one task may hold the lease. Other relay workers wait or return a
  retryable busy result; they never enter KeyMint concurrently.
- The journal stores job id, target, baseline, phase, timestamps, helper/TA
  versions and a transaction nonce. It never stores the generated keyblob or
  private key material.
- The journal fsyncs before every external side effect and after each observed
  result. Atomic replace is acceptable only with directory fsync.
- The task id is the idempotency key. A duplicate task can report the completed
  result or return a recovery-required status, but cannot mint another key.
- The result is posted only after `BASELINE_RESTORED` and postflight checks
  pass. A certificate delivered to A must not imply an unverified B state.
- On relay startup, an unfinished journal forces the executor to restore the
  recorded baseline while clients remain excluded. Unknown state is never
  treated as idle.

Suggested module boundaries:

```text
b-side/source/src/keymaster/spl_profile.rs   parsing/validation, no I/O
b-side/source/src/keymaster/spl_ta.rs         resident QSEECom client
b-side/source/src/keymaster/spl_window.rs     lease/state machine/RAII restore
b-side/source/src/bin/relay.rs                task integration and result contract
b-side/source/template/p0-lifecycle.sh       startup recovery ordering
```

## Client exclusion gate

This is the first hard gate and cannot be replaced by an assertion in Rust.

### Observed UI outage: separate Keystore2 freezing from the SPL change

During the five-minute manual window, the operator observed that the lock screen,
fingerprint authentication and normal application startup were unavailable until
the window ended. The hold script froze Keystore2 (`pid 1292`) for the entire
window and unfroze it only after restoring baseline. The SPL value and the
Keystore2 freezer were therefore changed together in both directions. That test
does not establish that the TA date alone broke those services; the frozen
Keystore2 is already a sufficient explanation for app startup and credential
authentication failures.

The following rules follow from that observation:

- A five-minute, request-visible hold is diagnostic-only and must never become
  the production design.
- Freezing Keystore2 for an unbounded period is a stop condition: it can make
  the device look broken even while its UI process is alive.
- Production must separate and measure the two effects before release: a short
  KeyMint stall caused by admission control, and the certificate date actually
  observed by A.
- No full-app lockout is acceptable. Temporary KeyMint call queuing may be
  acceptable only if calls resume automatically after baseline restore, do not
  cross existing Binder deadlines, and unlock/fingerprint success is unchanged.

Two implementation candidates should be evaluated in this order.

#### Candidate A: bounded micro-window admission (minimum viable path)

Keep the existing stock-HAL generation path, but make the exclusive window a
single bounded transaction rather than a held profile:

```text
freeze verified client set
  -> apply target
  -> one stock-HAL generateKey
  -> restore baseline
  -> verify baseline
  -> unfreeze clients
  -> post result
```

The resident TA client and event-driven freezer confirmation should reduce the
normal window from seconds to roughly the measured tens of milliseconds plus
scheduler noise. Keystore2-dependent calls queue during that interval and then
resume. This is viable only if a disposable device proves the window remains
inside all caller deadlines and that no direct caller bypasses the frozen set.
It also still has the catastrophic case where a secure-world call never
completes while the clients remain frozen; that path must quarantine the relay
without killing the call.

#### Candidate B: freeze the stock KeyMint HAL and use a direct-TA transaction

This can avoid freezing Keystore2 and the application UID cgroups. Instead:

```text
quiesce and freeze the stock KeyMint HAL process
  -> direct QSEECom apply target
  -> direct QSEECom KeyMint generateKey
  -> direct QSEECom restore baseline
  -> unfreeze the stock HAL
```

The relay's direct TA session is the only KeyMint producer during the window.
Keystore2 remains alive; application requests block at the frozen HAL and are
handled after it resumes. This is preferable for UI continuity if two proofs
hold:

1. freezing and observing `cgroup.events=frozen 1` guarantees that the stock HAL
   has completed all in-flight TA work; and
2. no other process can open `keymaster64` directly during the window.

The direct generation command is now identified as `0x3108`. QTI serializes a
CBOR map with key parameters, optional attestation-key data and certificate-time
tuples, then parses key 23 (keyblob), key 32 (certificate count) and repeated
key 33 (certificate bytes). A host-only model and tests have been written under
the local lab archive. The remaining gap is implementation validation on Android
and the client-exclusion proof, not discovery of an unknown command.

The SELinux policy allows `/dev/smcinvoke` (`tee_device`) to many HAL domains in
addition to the KeyMint HAL. Freezing the KeyMint HAL therefore closes its
Binder path but does not, without runtime evidence, prove that every possible
direct `keymaster64` caller is excluded. Candidate B remains a research
milestone until direct holders can be inventoried or an additional kernel/vendor
admission gate exists. A second HAL process or a new process loading the same TA
is not an independent secure-world context.

### First existing-key probe result

A controlled probe on 2026-10-20 used an existing OS `160000` baseline KeyBlob
and a temporary OS `170000` target. `0x2115` reached the TA but returned `-62`
(`KEY_REQUIRES_UPGRADE`) before issuing certificates. `0x2116` baseline restore
then returned status 0, and the HAL was recovered to `frozen=0` after a local
postflight bug left it frozen briefly. KeyMint/Keystore2/Gatekeeper/Vold PIDs,
boot ID, unlock state and System/Vendor properties remained unchanged.

This is a hard constraint for the reattestation candidate: a KeyBlob created
under an older OS version cannot be reattested under a newer OS target without
an upgrade. An upgrade would make the Blob newer than the restored baseline and
reintroduce the reuse problem. The next useful experiment is on a disposable
device, testing the version matrix without treating B as the matrix host:

- OS16 KeyBlob + OS16 target + newer patch levels;
- OS17 KeyBlob + OS17 target + restore;
- whether the OS17 Blob remains usable at OS16 baseline or must remain in an
  active target window.

The probe must not be repeated on B without a new explicit request.

The lab excluded Keystore2 and the already-frozen RKPD app. A production design
must enumerate and cover every live caller able to read the changed SPL or
mutate a keyblob during the window, including:

- KeyMint HAL callers that bypass Keystore2;
- Keystore2 and its worker threads;
- RKP/RemotelyProvisionedComponent callers;
- SecureClock and SharedSecret users where they can enter the same TA;
- kernel/compat client paths;
- any preserved old B-side module that still owns a KeyMint handle.

Before implementation is enabled, produce a read-only client inventory and an
admission proof. Acceptable mechanisms must have an atomic close-and-wait
contract and preserve in-flight secure calls. cgroup freezing is acceptable
only for a complete, verified set of clients and cgroup paths recomputed from
PIDs, not hardcoded. A long-held Keystore2 freeze fails the user-experience
acceptance even when it is technically effective.

If the complete set cannot be proven on the current B-only environment, the
feature remains compiled out and delivered as lab tooling only. The next safe
validation target is a disposable Qualcomm device, not B.

## Relay protocol changes

1. Parse `os_version`, `os_patch_level`, `vendor_patch_level` and
   `boot_patch_level` from the nested `device_attest_context` into
   `RequestedSpl`.
2. Require an A-side flag such as `adaptive_spl_required = true`. A B profile
   without `spl_window_supported` must return a capability error instead of
   minting baseline dates.
3. Extend the `profile` result with the supported target range, Boot policy,
   isolation mode and protocol version.
4. In `handle_generate_attest`, acquire the exclusive lease before any
   exclusion/TA call. Keep all normal parsing and validation outside the
   secure-state window.
5. Call the stock HAL exactly once for the target key. Disable automatic
   `upgradeKey`, replay, abort-based cleanup and speculative retries while the
   target window is active.
6. Restore baseline in an RAII guard and in the explicit success path. The
   guard is a recovery attempt; success still requires reading the configured
   baseline state and completing postflight checks.
7. Return structured fields such as `spl_profile_applied=true`,
   `spl_restore_status=verified`, `transaction_id`, and `boot_policy=exact`.
8. Do not let server-keybox fallback turn a B isolation failure into a
   different attestation mode unless A explicitly requested fallback and the
   response identifies its source.

## Failure and recovery contract

- Target preflight failure: no TA write, release the lease as `FAILED_BEFORE_TEE`.
- Target send errored or status unknown: never retry target; attempt one
  deterministic baseline restore, keep clients unavailable, save evidence and
  quarantine.
- HAL generate entered the secure world: wait for completion. Never use timeout,
  kill, Ctrl+C or process termination to interrupt it.
- Generate error after target apply: restore baseline once, return the KeyMint
  error, and do not issue another key generation for that task.
- Restore error: do not report success, do not release clients, do not reboot,
  and place the relay into a recovery-required state that rejects new tasks.
- Lost result POST: journal prevents another generateKey. Server retry sees the
  recorded transaction state.
- Unexpected KeyMint, Keystore2, Vold, Gatekeeper, lock-state or relay-health
  change: stop the transaction and collect read-only evidence aligned to the
  task id and PIDs.

## Keyblob and credential acceptance

These checks are mandatory before calling the feature deployable:

1. Sign a fresh message with a keyblob created before the target window after
   baseline restoration.
2. Create the target-window key once and prove which later operations remain
   valid. If it cannot be used after baseline restoration, document that it is
   verification-only and do not install it as a reusable app key.
3. Prove no `upgradeKey` occurs under the temporary higher profile unless the
   transaction explicitly accepts the resulting ratchet.
4. Verify PIN/biometric unlock, CE access, existing attestation, sign and
   decrypt flows after restoration without reboot.
5. Preserve a tested baseline artifact and verify its hash before mutation.
   Never assume the stock lower value is recoverable when keyblobs may have
   ratcheted forward.

## Tests and milestones

### M1: pure protocol and state-machine work

- Unit tests for B-CBOR encoding, 37-byte size assertion, YYYYMM/YYYMMDD
  validation, Boot rejection and duplicate task idempotence.
- Host model tests for every transition and failure edge.
- No Android secret-world calls and no feature enablement.

### M2: resident direct-TA client, disabled

- The static command and CBOR response paths are now recovered; implement the
  bounded QSEEcom client behind a build/runtime feature flag using that model.
- Unit-test request bytes, version handshake, TEE-only SPU bypass and response
  decoding against the host vectors.
- Keep the production call path unreachable until the exclusion gate passes.

### M3: read-only admission inventory

- Enumerate KeyMint/Keystore/RKP/SecureClock/SharedSecret clients and their
  cgroup paths on an approved non-disposable environment.
- Measure a no-op short freeze of each candidate cgroup to distinguish caller
  timeout sensitivity from destruction caused by the frozen dependency.
- Decide between Candidate A (bounded full-client window) and Candidate B
  (KeyMint HAL freeze plus direct-TA generation) using evidence, not wishful
  reasoning about concurrency.
- Produce a coverage matrix with one of: admitted by the same gate, proven
  inactive, or unresolved.
- Treat any unresolved row as a deployment blocker.

### M4: disposable-device validation

- Run apply -> single generate -> restore repeatedly on a disposable Qualcomm
  test device with the same TA ABI class.
- Include process crash and lost-response simulations that never kill an
  in-flight secure call.
- Verify old keyblob signing, unlock continuity and every postflight invariant.

### M5: B shadow integration

- Merge the parser, journal and capability reporting with hardware mutation
  disabled.
- Observe real task ordering, deadlines and duplicate delivery behavior.
- Do not send `0x2116` to B.

### M6: B controlled acceptance

- Requires a new exact approval naming the command, helper hash, target,
  baseline and recovery path.
- Start with one task and one generate call.
- Keep the feature automatically disabled if any preflight, stop condition or
  restore verification fails.

## Definition of done

- A request for A's dates yields a signed hardware chain with matching OS,
  System, Vendor and unchanged Boot values.
- Conflicting Boot values are rejected before entering the window.
- One task maps to one target application and one `generateKey`; duplicates
  cannot mint again.
- All relevant KeyMint clients are either covered by the admission contract or
  proven unable to call the TA during the window.
- Lock-screen and fingerprint behavior remain usable, or any synchronous
  KeyMint call pauses only for a measured bounded interval and resumes without
  an authentication failure or app restart.
- Baseline restoration is observed and journaled before the result leaves B.
- Existing keyblobs, credentials, unlock state and relay health remain valid.
- Crash and lost-response tests never require a reboot or interruption of an
  in-flight TEE call.
- The feature can be disabled without changing B's persistent SPL properties or
  requiring a hard reboot.

## Explicit non-goals

- Changing Boot patch level through `0x2116`.
- Changing verified-boot state, vbmeta digest, lock state or security level.
- Provisioning or spoofing `TEE_DEVICE_IDS`.
- Repairing the device's pre-existing unavailable RKP service.
- Editing certificates, extracting or reusing factory private keys.
- Deploying HAL reload, framework restart or hard reboot as recovery.
- Freezing Keystore2 for the entire lifetime of a profile or presenting the
  resulting device-wide outage as an acceptable production tradeoff.
