//! Host-only helper: assemble a version 1 material bundle from device files.
//!
//! cargo run --example make-bundle -- <stock_csr.bin> <seed.bin> <device_info.json> <fingerprint> <out.json>

use std::path::Path;

use anyhow::{Context, Result};
use ciborium::value::Value as Cbor;
use ommega_rkp_engine::cbor::{decode, encode};
use serde_json::{Value, json};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 5 {
        anyhow::bail!(
            "usage: make-bundle <stock_csr.bin> <seed.bin> <device_info.json> <fingerprint> <out.json>"
        );
    }
    let stock = std::fs::read(&args[0]).context("read stock CSR")?;
    let seed = std::fs::read(&args[1]).context("read seed")?;
    if seed.len() != 32 {
        anyhow::bail!("seed must be 32 bytes, got {}", seed.len());
    }
    let device_bytes = std::fs::read(&args[2]).context("read device info")?;
    // The captured files carry a UTF-8 BOM.
    let device_bytes = device_bytes
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .map(|rest| rest.to_vec())
        .unwrap_or(device_bytes);
    let device_info: Value = serde_json::from_slice(&device_bytes)?;
    let value = decode(&stock).context("decode stock CSR")?;
    let Cbor::Array(items) = value else {
        anyhow::bail!("stock CSR is not a CBOR array");
    };
    let chain = items
        .get(2)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("stock CSR has no DICE chain element"))?;
    let chain_bytes = encode(&Cbor::Array(match chain {
        Cbor::Array(entries) => entries,
        other => vec![other],
    }))?;
    let bundle = json!({
        "version": 1,
        "dice_chain_hex": hex::encode(&chain_bytes),
        "signing": {"kind": "p256-scalar", "bytes_hex": hex::encode(&seed)},
        "device": device_info,
        "issuance": {
            "fingerprint": args[3],
            "server_url": "https://remoteprovisioning.googleapis.com/v1",
            "protocol": "v3"
        }
    });
    let out = Path::new(&args[4]);
    std::fs::write(out, serde_json::to_vec_pretty(&bundle)?)?;
    println!(
        "wrote {} ({} bytes, chain {} bytes)",
        out.display(),
        std::fs::metadata(out)?.len(),
        chain_bytes.len()
    );
    Ok(())
}
