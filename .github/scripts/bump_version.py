#!/usr/bin/env python3
"""Set/check a coherent product version; setting the same version is idempotent."""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[2]
CARGOS = (
    "a-side/source/Cargo.toml",
    "a-side/source/ommega-injector/Cargo.toml",
    "b-side/source/Cargo.toml",
    "server/source/Cargo.toml",
)
LOCKS = {
    "a-side/source/Cargo.lock": ("ommega", "ommega-injector"),
    "b-side/source/Cargo.lock": ("ommegaclient-b",),
    "server/source/Cargo.lock": ("relay_rs",),
}
APPS = {
    "b-app/source/app/build.gradle.kts": "-ommega",
    "StrongBoxCapabilityMask/app/build.gradle.kts": "",
}

def read_version(root: Path = ROOT) -> str:
    return tomllib.loads((root / CARGOS[0]).read_text(encoding="utf-8"))["package"]["version"]

def replace_one(text: str, pattern: str, replacement, label: str) -> str:
    updated, count = re.subn(pattern, replacement, text, count=1, flags=re.MULTILINE)
    if count != 1:
        raise ValueError(f"missing version field: {label}")
    return updated

def set_version(root: Path, version: str) -> None:
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError(f"invalid version: {version}")
    updates = {}
    for relative in CARGOS:
        text = (root / relative).read_text(encoding="utf-8")
        updates[relative] = replace_one(text, r'^version = "[^"]+"', f'version = "{version}"', relative)
    for relative, names in LOCKS.items():
        text = (root / relative).read_text(encoding="utf-8")
        for name in names:
            pattern = rf'(\[\[package\]\]\nname = "{re.escape(name)}"\nversion = ")[^"]+("\n)'
            text = replace_one(text, pattern, lambda m: m[1] + version + m[2], relative + ":" + name)
        updates[relative] = text
    for relative, suffix in APPS.items():
        text = (root / relative).read_text(encoding="utf-8")
        match = re.search(r'versionName = "([^"]+)"', text)
        if not match:
            raise ValueError(f"missing versionName: {relative}")
        desired = version + suffix
        if match[1] != desired:
            text = replace_one(text, r'versionName = "[^"]+"', f'versionName = "{desired}"', relative)
            text = replace_one(text, r'versionCode = (\d+)', lambda m: f"versionCode = {int(m[1]) + 1}", relative)
        updates[relative] = text
    # Validate every file before writing any of them.
    for relative, updated in updates.items():
        target = root / relative
        if target.read_text(encoding="utf-8") != updated:
            target.write_text(updated, encoding="utf-8", newline="\n")

def check_versions(root: Path = ROOT) -> str:
    version = read_version(root)
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError(f"invalid product version: {version}")
    for relative in CARGOS:
        actual = tomllib.loads((root / relative).read_text(encoding="utf-8"))["package"]["version"]
        if actual != version:
            raise ValueError(f"version mismatch: {relative}")
    for relative, names in LOCKS.items():
        packages = tomllib.loads((root / relative).read_text(encoding="utf-8"))["package"]
        for name in names:
            actual = [p["version"] for p in packages if p["name"] == name]
            if actual != [version]:
                raise ValueError(f"version mismatch: {relative}:{name}")
    for relative, suffix in APPS.items():
        text = (root / relative).read_text(encoding="utf-8")
        name = re.search(r'versionName = "([^"]+)"', text)
        code = re.search(r'versionCode = (\d+)', text)
        if not name or name[1] != version + suffix or not code or int(code[1]) < 1:
            raise ValueError(f"version mismatch: {relative}")
    return version

def main() -> int:
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--version", help="set an explicit product version")
    mode.add_argument("--check", action="store_true", help="check all versions without changes")
    args = parser.parse_args()
    if not args.check:
        current = read_version()
        major, minor, patch = map(int, current.split("."))
        set_version(ROOT, args.version or f"{major}.{minor}.{patch + 1}")
    print(check_versions())
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
