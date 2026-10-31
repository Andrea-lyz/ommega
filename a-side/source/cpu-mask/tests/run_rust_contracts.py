#!/usr/bin/env python3
"""Run source auth/profile unit tests on the host with property IO disabled.

Supplementary contract tests only; this does not replace the Android gate.
"""
from pathlib import Path
import argparse
import json
import subprocess
import tempfile

SOURCE = Path(__file__).resolve().parents[2]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--toolchain", default="nightly-2026-09-01")
    parser.add_argument("--host", default="x86_64-pc-windows-msvc")
    args = parser.parse_args()
    target = SOURCE / "target"
    target.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="profile-contract-", dir=target) as directory:
        root = Path(directory).resolve()
        if root.parent != target.resolve():
            raise RuntimeError("unexpected temporary harness location")
        print(f"Temporary contract harness: {root}", flush=True)
        manifest = """[package]
name = "ommega-profile-contract-tests"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
serde = { version = "1.0.229", features = ["derive"] }
kmr-wire = { path = PATH }
""".replace("PATH", json.dumps((SOURCE / "wire").as_posix()))
        (root / "Cargo.toml").write_text(manifest, encoding="utf-8")
        src = root / "src"
        src.mkdir()
        imports = """// Property reads are a forbidden mock boundary in these pure tests.
extern crate self as rsproperties;
pub fn get<T>(_: &str) -> Result<T, std::io::Error> {
    panic!("host contract tests must not read Android device properties")
}
"""
        for name in ("auth_signing", "cpu_mask"):
            path = (SOURCE / f"common/src/{name}.rs").as_posix()
            imports += f"#[path = {json.dumps(path)}]\nmod {name};\n"
        # Match only the generated data carriers; no Binder runtime in host tests.
        imports += """
pub struct Certificate { pub encodedCertificate: Vec<u8> }
pub struct CertificateChain { pub leafCertificate: Vec<u8>, pub remainingChain: Vec<u8> }
"""
        chain = (SOURCE / "src/keymaster/cert_postprocessor/chain.rs").as_posix()
        imports += f"#[path = {json.dumps(chain)}]\nmod rkp_chain;\n"
        (src / "lib.rs").write_text(imports, encoding="utf-8")
        result = subprocess.run([
            "cargo", f"+{args.toolchain}", "test", "--offline", "--manifest-path",
            str(root / "Cargo.toml"), "--target", args.host, "--target-dir",
            str(target / "host-contract-build"), "--", "--nocapture"
        ], cwd=SOURCE)
        return result.returncode

if __name__ == "__main__":
    raise SystemExit(main())
