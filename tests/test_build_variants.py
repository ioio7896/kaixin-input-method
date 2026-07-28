from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest


REPO_ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("kaixin_build", REPO_ROOT / "build.py")
assert SPEC is not None and SPEC.loader is not None
kaixin_build = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = kaixin_build
SPEC.loader.exec_module(kaixin_build)


class PackageVariantParsingTests(unittest.TestCase):
    def test_all_selects_every_canonical_variant(self) -> None:
        self.assertEqual(
            kaixin_build.parse_package_variant_ids("all"),
            list(kaixin_build.PACKAGE_VARIANTS),
        )

    def test_full_alias_selects_ocr_variant(self) -> None:
        self.assertEqual(kaixin_build.parse_package_variant_ids("full"), ["ocr"])

    def test_alias_and_canonical_id_are_deduplicated(self) -> None:
        self.assertEqual(
            kaixin_build.parse_package_variant_ids("full,ocr,ime"),
            ["ocr", "ime"],
        )

    def test_all_cannot_be_mixed_with_individual_variants(self) -> None:
        with self.assertRaisesRegex(ValueError, "cannot be combined"):
            kaixin_build.parse_package_variant_ids("all,full")

    def test_unknown_variant_lists_all_supported_spellings(self) -> None:
        with self.assertRaisesRegex(ValueError, r"all, ime, ocr, full"):
            kaixin_build.parse_package_variant_ids("unknown")


if __name__ == "__main__":
    unittest.main()
