//! Host-only harness: issue a keybox from a material bundle.
//!
//! The Android binary cannot run on a PC, but this crate has no Android
//! dependency, so the same engine can be exercised against the real server here:
//!
//! cargo run --release --example issue -- <bundle.json> [<output dir>]

use std::path::Path;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bundle = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("usage: issue <bundle.json> [<output dir>]"))?;
    let output = args.get(1).map(String::as_str).unwrap_or(".");
    let paths = ommega_rkp_engine::paths::AppPaths::for_root(Path::new(output), Path::new(output));
    let result = ommega_rkp_engine::issue_from_bundle(&paths, Path::new(bundle))?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
