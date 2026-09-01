#!/usr/bin/env python3
"""Bump the product patch version across A/B/server Cargo manifests and the B-app."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

CARGO_FILES = (
    ROOT / "a-side" / "source" / "Cargo.toml",
    ROOT / "a-side" / "source" / "ommega-injector" / "Cargo.toml",
    ROOT / "b-side" / "source" / "Cargo.toml",
    ROOT / "server" / "source" / "Cargo.toml",
)

APP_GRADLE = ROOT / "b-app" / "source" / "app" / "build.gradle.kts"
VERSION_RE = re.compile(r'^version = "(\d+)\.(\d+)\.(\d+)"', re.MULTILINE)


def bump_patch(major: int, minor: int, patch: int) -> str:
    return f"{major}.{minor}.{patch + 1}"


def replace_first_version(path: Path, new_version: str) -> None:
    text = path.read_text(encoding="utf-8")
    updated, count = VERSION_RE.subn(f'version = "{new_version}"', text, count=1)
    if count != 1:
        raise SystemExit(f"expected one package version in {path}, found {count}")
    path.write_text(updated, encoding="utf-8")


def bump_app_gradle(new_version: str) -> None:
    text = APP_GRADLE.read_text(encoding="utf-8")
    text, name_count = re.subn(
        r'versionName = "[^"]+"',
        f'versionName = "{new_version}-ommega"',
        text,
        count=1,
    )
    if name_count != 1:
        raise SystemExit(f"expected one versionName in {APP_GRADLE}, found {name_count}")

    def inc_code(match: re.Match[str]) -> str:
        return f"versionCode = {int(match.group(1)) + 1}"

    text, code_count = re.subn(r"versionCode = (\d+)", inc_code, text, count=1)
    if code_count != 1:
        raise SystemExit(f"expected one versionCode in {APP_GRADLE}, found {code_count}")
    APP_GRADLE.write_text(text, encoding="utf-8")


def main() -> int:
    root_toml = CARGO_FILES[0].read_text(encoding="utf-8")
    match = VERSION_RE.search(root_toml)
    if not match:
        raise SystemExit(f"no package version in {CARGO_FILES[0]}")
    new_version = bump_patch(int(match.group(1)), int(match.group(2)), int(match.group(3)))
    for path in CARGO_FILES:
        replace_first_version(path, new_version)
    bump_app_gradle(new_version)
    print(new_version)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
