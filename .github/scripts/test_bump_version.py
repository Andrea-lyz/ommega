"""Version consistency tests on disposable copies, never the checkout itself."""
from pathlib import Path
import shutil
import tempfile
import unittest
import bump_version as versioning

class VersionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.files = (*versioning.CARGOS, *versioning.LOCKS, *versioning.APPS)
        for relative in self.files:
            dest = self.root / relative
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(versioning.ROOT / relative, dest)

    def snapshot(self):
        return {p: (self.root / p).read_bytes() for p in self.files}

    def test_all_components_and_idempotent_apk_codes(self):
        versioning.set_version(self.root, "9.8.7")
        self.assertEqual(versioning.check_versions(self.root), "9.8.7")
        snapshot = self.snapshot()
        versioning.set_version(self.root, "9.8.7")
        self.assertEqual(self.snapshot(), snapshot)

    def test_malformed_input_leaves_files_unchanged(self):
        snapshot = self.snapshot()
        with self.assertRaises(ValueError):
            versioning.set_version(self.root, "1.4.1\nother")
        self.assertEqual(self.snapshot(), snapshot)

    def test_broken_manifest_does_not_partially_update_other_files(self):
        app = self.root / next(iter(versioning.APPS))
        app.write_text("broken", encoding="utf-8")
        snapshot = self.snapshot()
        with self.assertRaises(ValueError):
            versioning.set_version(self.root, "9.8.7")
        self.assertEqual(self.snapshot(), snapshot)

    def test_check_detects_independent_strongbox_version(self):
        versioning.set_version(self.root, "9.8.7")
        app = self.root / "StrongBoxCapabilityMask/app/build.gradle.kts"
        app.write_text(app.read_text(encoding="utf-8").replace('versionName = "9.8.7"', 'versionName = "0.1.0"'), encoding="utf-8")
        with self.assertRaises(ValueError):
            versioning.check_versions(self.root)

if __name__ == "__main__":
    unittest.main()
