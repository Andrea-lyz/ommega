"""Offline regression tests for the pinned base, glue and real shell hashes.

Run: python -B a-side/source/scripts/webui-patch.test.py
All mutation/tamper cases operate inside TemporaryDirectory, never on a device.
"""
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1]
GLUES = ["compat", "auth-signing", "cpu-mask", "capability-mask", "app-detail", "diagnostics"]


class WebuiPatchTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="ommega-ui-patch-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.scripts = self.root / "scripts"
        self.scripts.mkdir()
        for name in ["webui-patch.py"] + ["webui-" + kind + ".js" for kind in GLUES]:
            shutil.copyfile(SOURCE / "scripts" / name, self.scripts / name)
        self.webroot = self.root / "template" / "webroot"
        shutil.copytree(SOURCE / "template" / "webroot", self.webroot)
        self.bundle = next((self.webroot / "assets").glob("index-*.js"))
        self.page = self.webroot / "index.html"
        self.script = self.scripts / "webui-patch.py"

    def run_patch(self, *arguments):
        env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")
        return subprocess.run([sys.executable, "-B", str(self.script), *arguments],
                              cwd=self.root, env=env, capture_output=True, text=True, encoding="utf-8")

    def snapshot(self):
        return {p.relative_to(self.root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
                for p in self.root.rglob("*") if p.is_file()}

    def assert_check_unchanged(self, expected):
        before = self.snapshot()
        result = self.run_patch("--check")
        self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
        self.assertEqual(self.snapshot(), before, "--check must never repair or rewrite")
        return result

    def mutate(self, path, transform):
        path.write_text(transform(path.read_text(encoding="utf-8")), encoding="utf-8", newline="\n")

    def test_shipped_patch_and_both_check_modes_are_readonly(self):
        self.assert_check_unchanged(0)
        before = self.snapshot()
        result = self.run_patch()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(before, self.snapshot(), "idempotent apply changed reviewed bytes")

    def test_current_glue_cannot_hide_a_changed_base(self):
        self.mutate(self.bundle, lambda text: "// unexpected prefix\n" + text)
        for flags in [("--check",), ()]:
            before = self.snapshot()
            result = self.run_patch(*flags)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("baseline", result.stderr)
            self.assertEqual(before, self.snapshot())

    def test_only_explicit_rebaseline_accepts_a_reviewed_base(self):
        self.mutate(self.bundle, lambda text: "// reviewed adaptation fixture\n" + text)
        self.assertEqual(self.run_patch("--rebaseline").returncode, 0)
        self.assert_check_unchanged(0)
        self.assertNotEqual(self.run_patch("--check", "--rebaseline").returncode, 0)

    def test_stale_glue_is_rebuilt_without_changing_the_base(self):
        self.mutate(self.scripts / "webui-diagnostics.js", lambda text: text + "\n// new fixture glue\n")
        self.assert_check_unchanged(1)
        self.assertEqual(self.run_patch().returncode, 0)
        self.assert_check_unchanged(0)

    def test_stale_shell_query_is_detected_and_rehashed(self):
        self.mutate(self.page, lambda text: re.sub(r"(index-[^\"?]+\.css)\?v=[^\"]+", r"\1?v=stale", text))
        result = self.assert_check_unchanged(1)
        self.assertIn("hashes", result.stderr)
        self.assertEqual(self.run_patch().returncode, 0)
        self.assert_check_unchanged(0)

    def test_changed_real_shell_bytes_require_new_hashes(self):
        self.mutate(self.webroot / "modern.js", lambda text: text + "\n// shell changed\n")
        self.assert_check_unchanged(1)
        self.assertEqual(self.run_patch().returncode, 0)
        digest = hashlib.sha256((self.webroot / "modern.js").read_bytes()).hexdigest()[:10]
        self.assertIn("./modern.js?v=" + digest, self.page.read_text(encoding="utf-8"))
        self.assert_check_unchanged(0)

    def test_missing_shell_resource_fails_closed(self):
        (self.webroot / "a-modern.css").unlink()
        self.assert_check_unchanged(1)

    def test_legacy_spl_writers_cannot_be_rebaselined(self):
        for writer in ['ommegaTrust("patch", "auto");', 'resetprop ro.build.version.security_patch 2026-11-01']:
            with self.subTest(writer=writer):
                original = self.bundle.read_bytes()
                self.mutate(self.bundle, lambda text: writer + "\n" + text)
                before = self.snapshot()
                result = self.run_patch("--rebaseline")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("SPL", result.stderr)
                self.assertEqual(before, self.snapshot())
                self.bundle.write_bytes(original)

    def test_legacy_html_controls_cannot_pass_check(self):
        self.mutate(self.page, lambda text: text.replace("</body>", '<button id="set-patch-now"></button></body>'))
        self.assert_check_unchanged(1)

    def test_ambiguous_glue_boundary_is_rejected(self):
        self.mutate(self.bundle, lambda text: text + "\n// ── Ommega detector compatibility policy fake\n")
        self.assert_check_unchanged(1)


if __name__ == "__main__":
    unittest.main()
