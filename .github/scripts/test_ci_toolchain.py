"""Exercise CI discovery with disposable commands; never install host packages."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("resolve_libclang.sh")
BASH = shutil.which("bash")


@unittest.skipUnless(BASH and os.name == "posix", "requires a POSIX bash environment")
class LibclangTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bin = self.root / "bin"
        self.lib = self.root / "lib"
        self.bin.mkdir()
        self.lib.mkdir()
        self.output = self.root / "output"
        self.calls = self.root / "calls"
        self.environment = dict(os.environ, PATH=str(self.bin), GITHUB_OUTPUT=str(self.output))
        self.environment["TEST_ROOT"] = str(self.root)
        self.command("sudo", "exec \"$@\"")
        self.command("apt-get", 'printf "%s\\n" "$*" >> "$TEST_ROOT/calls"\n'
                     'if [[ "$1" == install ]]; then : > "$TEST_ROOT/installed"; fi')

    def command(self, name, body):
        path = self.bin / name
        path.write_text(f"#!{BASH}\nset -euo pipefail\n{body}\n", encoding="utf-8")
        path.chmod(0o755)

    def run_discovery(self):
        return subprocess.run([BASH, str(SCRIPT)], env=self.environment,
                              capture_output=True, text=True)

    def assert_path(self):
        self.assertEqual(self.output.read_text(), f"libclang-path={self.lib}\n")

    def test_preinstalled_unversioned_llvm_skips_package_install(self):
        (self.lib / "libclang.so").touch()
        self.command("llvm-config", 'printf "%s/lib\\n" "$TEST_ROOT"')
        result = self.run_discovery()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_path()
        self.assertFalse(self.calls.exists())

    def test_versioned_llvm_and_soname_skip_package_install(self):
        (self.lib / "libclang.so.1").touch()
        self.command("llvm-config-18", 'printf "%s/lib\\n" "$TEST_ROOT"')
        result = self.run_discovery()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_path()
        self.assertFalse(self.calls.exists())

    def test_missing_library_installs_packages_then_resolves_again(self):
        self.command("llvm-config", '[[ -f "$TEST_ROOT/installed" ]] || exit 1\n'
                     ': > "$TEST_ROOT/lib/libclang.so"\n'
                     'printf "%s/lib\\n" "$TEST_ROOT"')
        result = self.run_discovery()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_path()
        self.assertEqual(self.calls.read_text().splitlines(), [
            "update", "install -y --no-install-recommends clang libclang-dev llvm-dev"])

    def test_unavailable_library_after_install_fails(self):
        self.command("llvm-config", "exit 1")
        result = self.run_discovery()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
