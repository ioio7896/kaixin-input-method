import copy
from datetime import datetime, timezone
import unittest

from release_validation import validate_metadata
from release_version import timestamp_version


class ReleaseGateTests(unittest.TestCase):
    def setUp(self):
        self.components = {name: {"schema": 1, "version": "2026.1111.37", "protocol": 5,
                                  "git": "unknown", "component": kind, "bits": bits}
                           for name, kind, bits in (("engine", "engine", 64),
                                                    ("x64", "tsf", 64), ("x86", "tsf", 32))}

    def test_unknown_git_does_not_bypass_version_or_protocol_gate(self):
        validate_metadata(self.components, "2026.1111.37", 5)
        for name in self.components:
            for field, stale in (("version", "2026.1110.843"), ("protocol", 4), ("bits", 16)):
                components = copy.deepcopy(self.components)
                components[name][field] = stale
                with self.subTest(name=name, field=field), self.assertRaises(RuntimeError):
                    validate_metadata(components, "2026.1111.37", 5)

    def test_missing_component_or_known_commit_mismatch_fails(self):
        with self.assertRaises(RuntimeError):
            validate_metadata({"engine": self.components["engine"]}, "2026.1111.37", 5)
        with self.assertRaises(RuntimeError):
            validate_metadata(self.components, "2026.1111.37", 5, "123456789abc")

    def test_timestamp_uses_shanghai_without_leading_zeroes(self):
        self.assertEqual(timestamp_version(datetime(2026, 1, 1, 16, 3, tzinfo=timezone.utc)),
                         "2026.102.3")
        self.assertEqual(timestamp_version(datetime(2026, 11, 10, 16, tzinfo=timezone.utc)),
                         "2026.1111.0")


if __name__ == "__main__":
    unittest.main()
