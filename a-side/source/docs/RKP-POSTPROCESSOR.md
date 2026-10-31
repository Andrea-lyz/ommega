# Native RKP certificate postprocessor

Native Android RKP key creation observes the system property
remote_provisioning.use_cert_processor. Its default is false. Android boolean true
values (1, true, y, yes, on) enable the optional service rkp_cert_processor.service.
No per-package or detector-name selection is involved.

When disabled, the original KeyMint certificates are retained and the concatenated
RKP issuer chain is appended, as keystore2 expects. When enabled with a leaf,
IKeystoreCertificatePostProcessor receives CertificateChain containing the first
leaf and the original concatenated RKP chain. A successful response replaces both
parts exactly. Lookup/transaction failure retains every original certificate and
appends the untouched RKP chain. Missing leaves do not call the service. The
returned chain is assembled for store_new_key, which untangles concatenated DER.

Service discovery retries the existing single-lookup helper for at most five
seconds, starting a lazy service where supported. A discovery timeout disables
further waits for that process lifetime, matching AOSP's failure latch without
leaking an unbounded detached waiter. This deadline covers discovery, not the
synchronous processor transaction itself; no secure-world call is interrupted.

This applies only to native RKP key generation, not a remote B chain or a caller-
selected local issuer. It does not change caller routing, hardware profile,
attestation fields or B operations. Android 12-17 can build the optional AIDL;
platforms without the property/service keep the original assembly path.

Pure mock contract tests cover disabled and missing-leaf paths, exact replacement,
transaction or unavailable-service fallback, preservation of unexpected original
certificates on failure and empty processor output. Android compilation validates
the generated interface. Actual service integration requires a separately
approved A-device test; no B device is used for these tests.

Contract references (android17-release):
- system/security/keystore2/postprocessor_client/src/lib.rs
- system/security/keystore2/aidl/android/security/postprocessor/
