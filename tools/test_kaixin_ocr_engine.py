#!/usr/bin/env python3
import importlib.util
import unittest
from pathlib import Path


HELPER = Path(__file__).with_name("kaixin_ocr_engine.py")
SPEC = importlib.util.spec_from_file_location("kaixin_ocr_engine", HELPER)
assert SPEC and SPEC.loader
ocr = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ocr)


class AdaptiveScalingTests(unittest.TestCase):
    def test_screen_line_is_not_upscaled(self) -> None:
        self.assertEqual(ocr.adaptive_limits("balanced", (48, 360, 3)), (1920, 0.0))

    def test_tiny_input_has_bounded_upscale(self) -> None:
        self.assertEqual(ocr.adaptive_limits("balanced", (10, 100, 3)), (1920, 15.0))
        self.assertEqual(ocr.adaptive_limits("accurate", (10, 100, 3)), (2560, 20.0))

    def test_fast_profile_never_upscales(self) -> None:
        self.assertEqual(ocr.adaptive_limits("fast", (5, 100, 3)), (1280, 0.0))

    def test_max_side_override_is_clamped(self) -> None:
        self.assertEqual(ocr.adaptive_limits("balanced", (48, 360, 3), 100), (320, 0.0))
        self.assertEqual(ocr.adaptive_limits("balanced", (48, 360, 3), 9000), (4096, 0.0))

    def test_unknown_profile_falls_back_to_balanced(self) -> None:
        self.assertEqual(ocr.normalize_profile("unknown"), "balanced")


if __name__ == "__main__":
    unittest.main()
