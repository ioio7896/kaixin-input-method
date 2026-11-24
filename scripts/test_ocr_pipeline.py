# SPDX-License-Identifier: Apache-2.0
"""OCR regression suite: benchmark metric checks + Rust native OCR unit tests."""

from __future__ import annotations

from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory
import unittest

from build_support.config import RAPIDOCR_REQUIRED_MODELS
from build_support.manifests import validate_rapidocr_layout
from ocr_benchmark import distance, normalized, percentile

ROOT = Path(__file__).resolve().parents[1]


class OcrPipelineTests(unittest.TestCase):
    def test_staged_ocr_uses_an_explicit_file_list(self):
        with TemporaryDirectory() as temporary:
            package = Path(temporary)
            self.assertEqual(validate_rapidocr_layout(package, False), [])
            rapidocr = package / "RapidOCRLite"
            models = rapidocr / "models"
            models.mkdir(parents=True)
            required = [rapidocr / name for name in ("LICENSE", "README.md", "README-CN.md")]
            required.extend(
                [rapidocr / "rapidocrlite.dll", *(models / n for n in RAPIDOCR_REQUIRED_MODELS)]
            )
            for path in required:
                path.write_bytes(b"fixture")
            self.assertEqual(validate_rapidocr_layout(package, True), [])
            (rapidocr / "leftover.py").write_text("pass", encoding="utf-8")
            errors = validate_rapidocr_layout(package, True)
            self.assertEqual(errors, ["unexpected staged OCR entry: RapidOCRLite/leftover.py"])
            self.assertTrue(validate_rapidocr_layout(package, False))

    def test_benchmark_counts_errors_and_keeps_tail(self):
        self.assertEqual(distance("你好ABC", "你好ADC!"), 2)
        self.assertEqual(normalized("你 好\nABC"), "你好ABC")
        self.assertEqual(percentile([1] * 18 + [100, 200], 0.95), 100)

    def test_rust_native_ocr_policy_crop_and_merge_unit_tests(self):
        completed = subprocess.run(
            ["cargo", "test", "--bin", "srf_ime_ocr"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        self.assertEqual(
            completed.returncode,
            0,
            f"cargo test --bin srf_ime_ocr failed:\n{completed.stdout}\n{completed.stderr}",
        )


if __name__ == "__main__":
    unittest.main()
