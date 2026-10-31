#!/usr/bin/env python3
"""Check the A/B Rust and Zygisk profile wire order without device access."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[4]

class ProfileContract(unittest.TestCase):
    def test_wire_schema_is_identical(self):
        a = (ROOT / "a-side/source/common/src/cpu_mask.rs").read_text(encoding="utf-8")
        b = (ROOT / "b-side/source/common/src/cpu_mask.rs").read_text(encoding="utf-8")
        self.assertEqual(a, b)
        header = (ROOT / "a-side/source/cpu-mask/src/profile_fields.h").read_text(encoding="utf-8")
        rust_keys = re.findall(r'"(ro\.[^"]+)"', a.split("pub const PROPERTIES:", 1)[1].split("];", 1)[0])
        cpp_keys = re.findall(r'"(ro\.[^"]+)"', header)
        self.assertEqual(rust_keys, cpp_keys)
        self.assertEqual(len(rust_keys), len(set(rust_keys)))
        mappings = (ROOT / "a-side/source/cpu-mask/src/identity_profile.h").read_text(encoding="utf-8")
        direct_keys = re.findall(r'\{"(ro\.[^"]+)", kHostValues', mappings)
        self.assertEqual(direct_keys, rust_keys[10:])
        self.assertEqual(len(rust_keys), 95)
        self.assertIn("PROFILE_VERSION: u32 = 2", a)

    def test_java_cache_indexes_match_wire_properties(self):
        header = (ROOT / "a-side/source/cpu-mask/src/profile_fields.h").read_text(encoding="utf-8")
        keys = re.findall(r'"(ro\.[^"]+)"', header)
        java = (ROOT / "a-side/source/cpu-mask/src/main.cpp").read_text(encoding="utf-8")
        mappings = dict((name, keys[int(index)]) for name, index in
                        re.findall(r'\{"([A-Z0-9_]+)", (\d+)\}', java))
        expected = {
            "SOC_MODEL": "ro.soc.model", "SOC_MANUFACTURER": "ro.soc.manufacturer",
            "FINGERPRINT": "ro.build.fingerprint", "ID": "ro.build.id",
            "DISPLAY": "ro.build.display.id", "TYPE": "ro.build.type",
            "TAGS": "ro.build.tags", "USER": "ro.build.user", "HOST": "ro.build.host",
            "BOARD": "ro.product.board", "HARDWARE": "ro.hardware",
            "BOOTLOADER": "ro.bootloader", "INCREMENTAL": "ro.build.version.incremental",
            "RELEASE": "ro.build.version.release", "SDK": "ro.build.version.sdk",
            "CODENAME": "ro.build.version.codename", "SUPPORTED_ABIS": "ro.product.cpu.abilist",
            "SUPPORTED_32_BIT_ABIS": "ro.product.cpu.abilist32",
            "SUPPORTED_64_BIT_ABIS": "ro.product.cpu.abilist64",
        }
        self.assertEqual(mappings, expected)
        self.assertIn(""""SDK_INT", 'I', property(32)""", java)
        self.assertIn("parseSdkFull(property(52)", java)
        self.assertIn(""""TIME", 'J', property(21)""", java)

if __name__ == "__main__":
    unittest.main()
