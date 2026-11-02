# ommega-rkp-engine

RKP issuance engine: DICE chain + signing material -> AuthenticatedRequest CSR v3
-> verified keybox. It is the backend of the Automation page in the A-side WebUI.

Ported from [Duck ToolBox](https://github.com/eltavine/Duck-ToolBox)
`crates/duck-rkp` (MIT, Copyright (c) 2026 Eltavine, see `LICENSE`). The port
keeps the imported-material flow only:

- dropped: the `duckd` CLI, the saved-profile layer, the device collection and
  the degenerate-chain provisioning paths;
- replaced: `duck-core::AppPaths` and its fs helpers by `src/paths.rs`, and the
  `duck-platform` rustls bundle plus reqwest 0.13 by reqwest 0.12 with the
  rustls(ring) stack this module already ships for Android;
- kept verbatim: CBOR, COSE, CSR v3 construction, DICE/CSR verification, chain
  selection, keybox XML export and its round-trip validation.

The version 1 bundle contract is unchanged from the original:

```json
{
  "version": 1,
  "dice_chain_hex": "<complete original CBOR chain>",
  "signing": {"kind": "p256-scalar", "bytes_hex": "<32-byte scalar>"},
  "device": {"brand": "…", "model": "…", "device": "…", "product": "…",
             "manufacturer": "…", "fused": 1, "vb_state": "green",
             "os_version": "15", "security_level": "tee",
             "bootloader_state": "locked", "boot_patch_level": 20250801,
             "system_patch_level": 20250801, "vendor_patch_level": 20250801},
  "issuance": {"fingerprint": "…",
               "server_url": "https://remoteprovisioning.googleapis.com/v1",
               "protocol": "v3"}
}
```

`keybox_path` in the result points at the exported XML; the caller decides where
to install it. Two host-only examples exist for validation without a device:

```
cargo run --example make-bundle -- <stock_csr.bin> <seed.bin> <device_info.json> <fingerprint> <out.json>
cargo run --example issue -- <bundle.json> <output dir>
```
