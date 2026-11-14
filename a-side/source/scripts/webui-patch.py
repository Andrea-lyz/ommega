#!/usr/bin/env python3
"""Apply and verify the locally reviewed WebUI bundle glue.

The opaque vendor base is pinned separately from the appended script sources.
Every path (including an already-current suffix and --check) verifies that base.
After deliberately adapting a new base, --rebaseline records it and patches it.
--check is strictly read-only and verifies all local shell resource hashes.
"""
from __future__ import annotations

import argparse
import hashlib
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "template" / "webroot" / "assets"
GLUE = ROOT / "scripts" / "webui-compat.js"
AUTH_GLUE = ROOT / "scripts" / "webui-auth-signing.js"
CPU_GLUE = ROOT / "scripts" / "webui-cpu-mask.js"
CAPABILITY_GLUE = ROOT / "scripts" / "webui-capability-mask.js"
APP_DETAIL_GLUE = ROOT / "scripts" / "webui-app-detail.js"
DIAG_GLUE = ROOT / "scripts" / "webui-diagnostics.js"
MARKER = "OMMEGA_TARGET_COMPAT"
GLUE_HEADER = "\n// ── Ommega detector compatibility policy"

# Reviewed vendor base with the legacy SPL editor, automation and rebind writers
# removed. Unrelated vendor initialization and verified-boot hash/key stay intact.
BASELINE = "911f3ab6d7114e83dfd7aad5cafebf9a822e73183c31842ce974043c35799828"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bundle_path() -> Path:
    bundles = sorted(ASSETS.glob("index-*.js"))
    if len(bundles) != 1:
        raise SystemExit("expected exactly one WebUI bundle in " + str(ASSETS)
                         + ", found: " + ", ".join(path.name for path in bundles))
    return bundles[0]


def split_bundle(text: str) -> tuple[str, str | None]:
    if GLUE_HEADER not in text:
        if MARKER in text:
            raise ValueError("cannot locate the existing glue boundary")
        return text, None
    if text.count(GLUE_HEADER) != 1:
        raise ValueError("ambiguous WebUI glue boundary")
    at = text.index(GLUE_HEADER)
    base, glue = text[:at], text[at:]
    if MARKER in base or MARKER not in glue:
        raise ValueError("invalid WebUI glue boundary")
    return base, glue


def verify_readonly_spl(text: str) -> None:
    """Reject the complete shipped legacy write surface, not merely visible UI."""
    forbidden = ("security-patch-dialog", "all-patch", "boot-patch", "system-patch",
                 "vendor-patch", "devconfig-securityPatch",
                 "security-patch-auto-toggle", "set-patch-now", "save-patch",
                 "get-patch", "patchAuto", "--security-patch",
                 "source.android.com/docs/security/bulletin/pixel",
                 "Tricky-Addon-Update-Target-List/main/.patch")
    for token in forbidden:
        if token in text:
            raise ValueError("shipped SPL write/fetch surface remains: " + token)
    if re.search(r"ommegaTrust\s*\(\s*['\"]patch['\"]", text):
        raise ValueError("shipped SPL trust writer remains")
    if re.search(r"\b(?:resetprop|setprop)\b[^\n`]*(?:security_patch|patchlevel)", text):
        raise ValueError("shipped SPL property writer remains")


def rebaseline(path: Path, digest: str) -> None:
    source = Path(__file__).resolve()
    text = source.read_text(encoding="utf-8")
    updated, count = re.subn(r'^BASELINE = "[0-9a-f]{64}"$',
                            'BASELINE = "' + digest + '"', text, flags=re.MULTILINE)
    if count != 1:
        raise ValueError("failed to rewrite the baseline in " + str(source))
    source.write_text(updated, encoding="utf-8", newline="\n")
    print("recorded new baseline " + digest + " for " + path.name)


def versioned_assets(html: str, webroot: Path) -> str:
    """Hash the actual local src/href target; missing resources fail closed."""
    pattern = re.compile(r'''(?P<prefix>\b(?:src|href)=["'])(?P<path>\./[^"'?]+\.(?:js|css))(?:\?[^"']*)?(?P<quote>["'])''')

    def replace(match: re.Match) -> str:
        target = (webroot / match.group("path")[2:]).resolve()
        if not target.is_relative_to(webroot.resolve()):
            raise ValueError("shell resource escapes webroot: " + match.group("path"))
        digest = sha256(target)[:10]
        return match.group("prefix") + match.group("path") + "?v=" + digest + match.group("quote")

    return pattern.sub(replace, html)


def refresh_asset_versions(*, check_only: bool = False) -> None:
    webroot = ROOT / "template" / "webroot"
    page = webroot / "index.html"
    html = page.read_text(encoding="utf-8")
    verify_readonly_spl(html)
    updated = versioned_assets(html, webroot)
    if updated != html:
        if check_only:
            raise ValueError("WebUI shell resource hashes are stale; run scripts/webui-patch.py")
        page.write_text(updated, encoding="utf-8", newline="\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="verify without writing")
    mode.add_argument("--rebaseline", action="store_true",
                      help="record a deliberately adapted base, then patch")
    args = parser.parse_args()
    try:
        path = bundle_path()
        text = path.read_bytes().decode("utf-8")
        base, previous_glue = split_bundle(text)
        glue = "".join(source.read_text(encoding="utf-8") for source in (
            GLUE, AUTH_GLUE, CPU_GLUE, CAPABILITY_GLUE, APP_DETAIL_GLUE, DIAG_GLUE))
        if not glue.startswith(GLUE_HEADER):
            raise ValueError("glue source header does not match its boundary")
        verify_readonly_spl(base + glue)
        # Never skip this check merely because the suffix is current.
        digest = hashlib.sha256(base.encode("utf-8")).hexdigest()
        if digest != BASELINE:
            if not args.rebaseline:
                raise ValueError("WebUI base differs from the recorded baseline (" + digest
                                 + "). Review/adapt the base, then explicitly --rebaseline.")
            rebaseline(path, digest)
        if args.check:
            if previous_glue != glue:
                raise ValueError("WebUI glue is missing or differs from scripts; run scripts/webui-patch.py")
            verify_readonly_spl(text)
            refresh_asset_versions(check_only=True)
            print("WebUI base, glue, read-only SPL surface and shell hashes verified: " + path.name)
            return 0
        patched = base + glue
        if patched != text:
            path.write_bytes(patched.encode("utf-8"))
            if path.read_bytes() != patched.encode("utf-8"):
                raise ValueError("WebUI patch readback failed")
        refresh_asset_versions()
        print("WebUI base and glue current: " + path.name + " -> " + sha256(path))
        return 0
    except (ValueError, OSError) as error:
        print("ERROR: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
