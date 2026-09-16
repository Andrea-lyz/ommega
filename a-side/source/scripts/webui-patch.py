#!/usr/bin/env python3
"""Apply the detector-compatibility glue to the prebuilt WebUI bundle.

The A-side WebUI ships as a prebuilt Vite bundle (template/webroot/assets/
index-*.js) whose source is not part of this repository, so the per-package
compatibility toggle in scripts/webui-compat.js is appended to the bundle as a
self-contained glue block instead of being rebuilt from source.

The patch is guarded by a sha256 baseline of the unpatched bundle. When upstream
ships a new bundle the baseline check fails on purpose, so the glue gets
re-adapted instead of being appended to a file it was not written for. Use
--rebaseline to record the new bundle after adapting the glue.

Usage:
    python3 scripts/webui-patch.py            # apply (idempotent)
    python3 scripts/webui-patch.py --check    # verify, for CI
"""
from __future__ import annotations

import argparse
import hashlib
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "template" / "webroot" / "assets"
GLUE = ROOT / "scripts" / "webui-compat.js"
MARKER = "OMMEGA_TARGET_COMPAT"

# sha256 of the unpatched bundle this glue was written against.
BASELINE = "88e369b9dcc6176c50dcc2666e0e49c568d966ab9400f4a817f57227d39a9309"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bundle_path() -> Path:
    bundles = sorted(ASSETS.glob("index-*.js"))
    if len(bundles) != 1:
        raise SystemExit(
            "expected exactly one WebUI bundle in "
            + str(ASSETS)
            + ", found: "
            + ", ".join(path.name for path in bundles)
        )
    return bundles[0]


def rebaseline(path: Path, digest: str) -> None:
    source = Path(__file__).resolve()
    text = source.read_text(encoding="utf-8")
    updated = text.replace('BASELINE = "' + BASELINE + '"', 'BASELINE = "' + digest + '"')
    if updated == text:
        raise SystemExit("failed to rewrite the baseline in " + str(source))
    source.write_text(updated, encoding="utf-8")
    print("recorded new baseline " + digest + " for " + path.name)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify the patch without writing")
    parser.add_argument(
        "--rebaseline",
        action="store_true",
        help="record the current unpatched bundle hash as the new baseline, then patch",
    )
    args = parser.parse_args()

    path = bundle_path()
    text = path.read_text(encoding="utf-8")
    glue = GLUE.read_text(encoding="utf-8")
    digest = sha256(path)
    patched = MARKER in text

    if args.check:
        if patched and text.endswith(glue):
            print("WebUI bundle patch present: " + path.name)
            return 0
        if patched:
            print(
                "ERROR: " + path.name + " is patched with a different glue than "
                "scripts/webui-compat.js",
                file=sys.stderr,
            )
            return 1
        if digest != BASELINE:
            print(
                "ERROR: " + path.name + " does not match the recorded baseline "
                "(" + digest + "). Adapt scripts/webui-compat.js for the new bundle, "
                "then run --rebaseline.",
                file=sys.stderr,
            )
            return 1
        print("ERROR: " + path.name + " is not patched; run scripts/webui-patch.py", file=sys.stderr)
        return 1

    if patched and text.endswith(glue):
        print(path.name + " is already patched")
        return 0

    if digest != BASELINE:
        if not args.rebaseline:
            print(
                "ERROR: " + path.name + " does not match the recorded baseline "
                "(" + digest + "). Adapt scripts/webui-compat.js for the new bundle, "
                "then run --rebaseline.",
                file=sys.stderr,
            )
            return 1
        rebaseline(path, digest)

    if patched:
        print("ERROR: " + path.name + " is patched with a different glue", file=sys.stderr)
        return 1

    path.write_text(text + glue, encoding="utf-8")
    if not path.read_text(encoding="utf-8").endswith(glue):
        raise SystemExit("patch verification failed: the bundle does not end with the glue")
    print("patched " + path.name + " -> " + sha256(path))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
