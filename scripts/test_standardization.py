"""Failure-path coverage for generated contracts and the dual-installer release gate."""

import json
from pathlib import Path
import tempfile
import unittest

from generate_engine_contract import render_engine_contract
from release import validate_pair
from build_support.manifests import write_package_hash_manifest, verify_package_hash_manifest

ROOT = Path(__file__).resolve().parents[1]


class StandardizationTests(unittest.TestCase):
    def setUp(self):
        parent = ROOT / "dist/test-fixtures"
        parent.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=parent)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_contract_rejects_overlapping_mode_bits_and_insufficient_capacity(self):
        source = json.loads((ROOT / "shared/engine_contract.json").read_text(encoding="utf-8"))
        target = self.root / "shared/engine_contract.json"
        target.parent.mkdir()
        target.write_text(json.dumps(source), encoding="utf-8")
        self.assertEqual(len(render_engine_contract(self.root)), 2)
        source["mode_flags"]["MODE_FUZZY_PINYIN"] = source["mode_flags"]["MODE_DOUBLE_PINYIN"]
        target.write_text(json.dumps(source), encoding="utf-8")
        with self.assertRaises(ValueError):
            render_engine_contract(self.root)
        source["limits"]["full_candidates"] = 128
        target.write_text(json.dumps(source), encoding="utf-8")
        with self.assertRaises(ValueError):
            render_engine_contract(self.root)

    def test_release_requires_both_new_installers_and_matching_staged_versions(self):
        version = "2026.1111.1200"
        paths = []
        for variant in ("ime", "ocr"):
            stage = self.root / "dist" / f"kaixin-package-{variant}"
            stage.mkdir(parents=True)
            (stage / "VERSION").write_text(version, encoding="utf-8")
            write_package_hash_manifest(stage)
            path = stage.parent / f"kaixin-setup-{variant}-{version}-20261111-120000.exe"
            path.write_bytes(b"test installer fixture")
            paths.append(path)
        self.assertEqual(validate_pair(self.root, version, set()), paths)
        with self.assertRaises(RuntimeError):
            validate_pair(self.root, version, {paths[0]})
        (self.root / "dist/kaixin-package-ocr/VERSION").write_text("old", encoding="utf-8")
        with self.assertRaises(RuntimeError):
            validate_pair(self.root, version, set())

    def test_integrity_manifest_rejects_corruption_and_parent_traversal(self):
        payload = self.root / "payload.bin"
        payload.write_bytes(b"original")
        manifest = write_package_hash_manifest(self.root)
        self.assertEqual(verify_package_hash_manifest(self.root), 1)
        payload.write_bytes(b"corrupted")
        with self.assertRaises(RuntimeError):
            verify_package_hash_manifest(self.root)
        manifest.write_text("0" * 64 + " *../outside.bin\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            verify_package_hash_manifest(self.root)


if __name__ == "__main__":
    unittest.main()
