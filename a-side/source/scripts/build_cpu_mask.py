#!/usr/bin/env python3
"""Build the bundled exp6 Zygisk library with the module's existing NDK."""
from __future__ import annotations

import argparse
from pathlib import Path
import subprocess

from build_soterta_svc import cargo_config_ndk_root
from setup_cargo_config import detect_ndk_root, resolve_host_tag

ROOT = Path(__file__).resolve().parents[1]


def build_cpu_mask(abi: str, release: bool = True) -> Path:
    triples = {"arm64-v8a": "aarch64-linux-android", "x86_64": "x86_64-linux-android"}
    ndk = detect_ndk_root(cargo_config_ndk_root())
    host = resolve_host_tag(ndk, None)
    tools = ndk / "toolchains" / "llvm" / "prebuilt" / host / "bin"
    compiler = tools / "clang++.exe"
    if not compiler.is_file():
        compiler = tools / "clang++"
    source = ROOT / "cpu-mask"
    output = ROOT / "target" / "cpu-mask" / abi / ("release" if release else "debug") / f"{abi}.so"
    output.parent.mkdir(parents=True, exist_ok=True)
    command = [str(compiler), f"--target={triples[abi]}31", "-std=c++17", "-shared", "-fPIC",
        "-O2" if release else "-O0", "-Wall", "-Wextra", "-Werror", "-fno-exceptions",
        "-fno-rtti", "-fno-threadsafe-statics", "-fvisibility=hidden", "-fvisibility-inlines-hidden",
        "-ffunction-sections", "-fdata-sections", "-static-libstdc++",
        "-Wl,--no-undefined,--gc-sections,--exclude-libs,ALL,-z,max-page-size=16384",
        "-I", str(source / "third_party"),
        *(str(source / "src" / name) for name in
          ("main.cpp", "property_view.cpp", "property_storage.cpp", "runtime_profile.cpp")),
        "-llog", "-o", str(output)]
    subprocess.run(command, cwd=ROOT, check=True)
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--abi", choices=["arm64-v8a", "x86_64"], default="arm64-v8a")
    parser.add_argument("--debug", action="store_true")
    args = parser.parse_args()
    print(build_cpu_mask(args.abi, not args.debug))
