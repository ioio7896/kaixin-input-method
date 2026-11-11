# SPDX-License-Identifier: Apache-2.0
"""OCR recovery/geometry regressions. Native model quality is checked separately."""
from __future__ import annotations

import argparse
import base64
from collections import OrderedDict
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import kaixin_ocr_engine as helper
from kaixin_ocr_policy import (ImageAnalysis, MAX_REFINEMENTS, MAX_WORK_PIXELS,
    WorkBudget, analyze_image, image_tiles, rect_area, should_retry_preprocess,
    refinement_regions, needs_detector_retry)
from ocr_benchmark import distance, normalized, percentile


class GeometryTests(unittest.TestCase):
    def test_4k_has_full_coverage_with_bounded_overlap(self):
        tiles = image_tiles((2160, 3840, 3), 1920)
        self.assertGreater(len(tiles), 1)
        self.assertLess(sum(map(rect_area, tiles)), 3840 * 2160 * 1.3)
        for x in [0, 1183, 1280, 1376, 1920, 2464, 2560, 2656, 3839]:
            for y in [0, 984, 1080, 1176, 2159]:
                self.assertTrue(any(x0 <= x < x1 and y0 <= y < y1 for x0, y0, x1, y1 in tiles))
        self.assertTrue(all(max(x1-x0, y1-y0) <= 1920 for x0, y0, x1, y1 in tiles))

    def test_boundary_does_not_duplicate_whole_image(self):
        tiles = image_tiles((1921, 1921, 3), 1920)
        self.assertLess(sum(map(rect_area, tiles)), 1921 * 1921 * 1.3)
        self.assertEqual(image_tiles((48, 360, 3), 1920), [(0, 0, 360, 48)])

    def test_huge_plan_is_rejected_without_partial_output(self):
        with self.assertRaises(ValueError):
            image_tiles((16384, 16384, 3), 1280)

    def test_optional_budget_stops_but_primary_never_silently_truncates(self):
        clock = [0.0]
        budget = WorkBudget(10, lambda: clock[0])
        for _ in range(MAX_REFINEMENTS):
            self.assertTrue(budget.reserve(100, "small-text", optional=True))
        self.assertFalse(budget.reserve(100, "small-text", optional=True))
        self.assertTrue(budget.reserve(100, "primary"))
        clock[0] = 11
        self.assertFalse(budget.reserve(100, "contrast", optional=True))
        with self.assertRaises(TimeoutError):
            budget.reserve(100, "primary")
        with self.assertRaises(TimeoutError):
            WorkBudget(10).reserve(MAX_WORK_PIXELS + 1, "primary")

    def test_valid_single_character_is_not_a_retry_trigger(self):
        for analysis in (ImageAnalysis(), ImageAnalysis(5, 0, 40)):
            self.assertFalse(should_retry_preprocess([{"text": "好", "score": .99}], analysis))
        self.assertTrue(should_retry_preprocess([{"text": "?", "score": .4}], ImageAnalysis(5, 0, 40)))
        self.assertFalse(should_retry_preprocess([], ImageAnalysis()))

    def test_small_component_inside_recognized_line_does_not_duplicate_a_suffix(self):
        lines = [{"text": "金额 128.50 元", "score": .99,
                  "box": [[0, 0], [300, 0], [300, 30], [0, 30]]}]
        analysis = ImageAnalysis(8, 4, 255, [(270, 0, 300, 30)], [(270, 0, 300, 30)])
        self.assertFalse(needs_detector_retry(lines, analysis))
        self.assertEqual(refinement_regions(lines, analysis), [])

    def test_dedup_keeps_repeated_text_at_distinct_positions(self):
        def line(x, score):
            return {"text": "测试", "score": score, "box": [[x, 0], [x+40, 0], [x+40, 20], [x, 20]]}
        self.assertEqual(len(helper.merge_tile_lines([line(0, .8), line(1, .95), line(100, .9)])), 2)

    def test_seam_prefix_cannot_replace_a_complete_line(self):
        clipped = {"text": "跨越分块接缝", "score": .9999, "tile_edge": True,
                   "box": [[100, 0], [200, 0], [200, 20], [100, 20]]}
        full = {"text": "跨越分块接缝的完整文字", "score": .99, "tile_edge": True,
                "box": [[100, 0], [400, 0], [400, 20], [100, 20]]}
        self.assertEqual([x["text"] for x in helper.merge_tile_lines([clipped, full])], [full["text"]])

    def test_seam_stitches_overlapping_suffix_and_prefix(self):
        left = {"text": "第一段接缝文字", "score": .99, "clip_right": True,
                "box": [[0, 0], [150, 0], [150, 20], [0, 20]]}
        right = {"text": "接缝文字继续识别", "score": .98, "clip_left": True,
                 "box": [[80, 0], [260, 0], [260, 20], [80, 20]]}
        merged = helper.merge_tile_lines([left, right])
        self.assertEqual([x["text"] for x in merged], ["第一段接缝文字继续识别"])

    def test_benchmark_counts_errors_and_keeps_tail(self):
        self.assertEqual(distance("你好ABC", "你好ADC!"), 2)
        self.assertEqual(normalized("你 好\nABC"), "你好ABC")
        self.assertEqual(percentile([1] * 18 + [100, 200], .95), 100)


def fake_rapidocr_module():
    module = types.ModuleType("rapidocr")
    module.EngineType = types.SimpleNamespace(PPOCR_CPP="ppocr_cpp")
    module.LangDet = module.LangRec = types.SimpleNamespace(CH="ch")
    module.ModelType = types.SimpleNamespace(SMALL="small", MEDIUM="medium")
    module.OCRVersion = types.SimpleNamespace(PPOCRV6="PP-OCRv6")
    return module


class FakeNative:
    def __init__(self):
        self.closed = False

    def close(self):
        self.closed = True


class FakeEngine:
    created = []

    def __init__(self, params):
        self.params = params
        self._engine = FakeNative()
        self.initial_native = self._engine
        self.calls = []
        self.created.append(self)

    def _ensure_engine(self):
        return self._engine

    def __call__(self, image, **kwargs):
        import numpy as np
        self.calls.append(kwargs)
        height, width = image.shape[:2]
        return types.SimpleNamespace(txts=("好",), scores=(.99,),
            boxes=np.array([[[5, 5], [min(width-1, 30), 5], [min(width-1, 30), min(height-1, 28)], [5, min(height-1, 28)]]]),
            elapse_list=[.001])


class EngineFixture:
    def setUp(self):
        FakeEngine.created = []
        self.service = helper.RapidOcrService.__new__(helper.RapidOcrService)
        self.service.args = argparse.Namespace(rec_batch_num=8, fast_mode=False, download_retries=1)
        self.service.model_root = ROOT / "RapidOCRLite/python/rapidocr/models"
        self.service.rapidocr_root = ROOT / "RapidOCRLite"
        self.service._rapidocr_cls = FakeEngine
        self.service._engines = OrderedDict()
        self.service._warmed = set()
        self.service.cache_limit = 2
        self.patch = patch.dict(sys.modules, {"rapidocr": fake_rapidocr_module()})
        self.patch.start()

    def tearDown(self):
        self.service.close()
        self.patch.stop()


class CacheTests(EngineFixture, unittest.TestCase):
    def test_profiles_use_distinct_real_models_and_fast_mode_is_opt_in(self):
        for profile, expected in (("fast", ("small", "small")), ("balanced", ("small", "medium")), ("accurate", ("medium", "medium"))):
            params, _ = self.service._engine_params(profile)
            self.assertTrue(params["Det.model_path"].endswith(f"det_{expected[0]}.onnx"))
            self.assertTrue(params["Rec.model_path"].endswith(f"rec_{expected[1]}.onnx"))
            self.assertFalse(params["Rec.fast_mode"])
        self.service.args.fast_mode = True
        self.assertTrue(self.service._engine_params("fast")[0]["Rec.fast_mode"])
        self.assertFalse(self.service._engine_params("balanced")[0]["Rec.fast_mode"])

    def test_lru_closes_evicted_native_resources(self):
        fast, _, _, _ = self.service._load_engine("fast")
        native_fast = fast._engine
        balanced, _, _, _ = self.service._load_engine("balanced")
        native_balanced = balanced._engine
        self.service._load_engine("fast")
        self.service._load_engine("accurate")
        self.assertTrue(native_balanced.closed)
        self.assertFalse(native_fast.closed)
        self.assertEqual(len(self.service._engines), 2)

    def test_actual_native_limit_participates_in_cache_key(self):
        first = self.service._load_engine("balanced", limit=1920)[0]
        second = self.service._load_engine("balanced", limit=1280)[0]
        self.assertIsNot(first, second)
        self.assertEqual(second.params["Det.limit_side_len"], 1280)


try:
    import cv2
    import numpy as np
except ImportError:
    cv2 = np = None


@unittest.skipIf(cv2 is None, "Install numpy and opencv-python-headless for image tests")
class ImageTests(EngineFixture, unittest.TestCase):
    def recognize_memory(self, image, **kwargs):
        loader = types.ModuleType("rapidocr.utils.load_image")
        loader.LoadImage = lambda: lambda _: image
        with patch.dict(sys.modules, {"rapidocr.utils.load_image": loader}):
            return self.service.recognize(
                {"image": "memory", "image_bytes_base64": base64.b64encode(b"test").decode()},
                "auto", .5, profile="balanced", **kwargs)

    def test_warmup_runs_detection_and_recognition_and_language_reuses_cache(self):
        first = self.service.warmup("zh", "balanced")
        second = self.service.warmup("en", "balanced")
        third = self.service.warmup("zh_tw", "balanced")
        self.assertTrue(first["warmed_recognition"])
        self.assertTrue(second["already_warm"] and third["already_warm"])
        self.assertEqual(len(FakeEngine.created), 1)
        self.assertEqual([call["use_det"] for call in FakeEngine.created[0].calls], [True, False])

    def test_blank_and_low_contrast_text_are_distinguished(self):
        image = np.full((150, 600, 3), 220, np.uint8)
        self.assertEqual(analyze_image(image).components, 0)
        cv2.putText(image, "Small text 12345", (10, 50), cv2.FONT_HERSHEY_SIMPLEX, .5, (180, 180, 180), 1, cv2.LINE_AA)
        result = analyze_image(image)
        self.assertGreaterEqual(result.small_components, 3)
        self.assertTrue(result.small_regions)
        self.assertTrue(result.low_contrast)

    def test_auto_and_single_character_execute_only_once_after_warmup(self):
        image = np.full((100, 160, 3), 255, np.uint8)
        self.service.warmup("en", "balanced")
        engine = FakeEngine.created[0]
        result = self.recognize_memory(image)
        self.assertEqual(result["text"], "好")
        self.assertFalse(result["auto_language_retry"])
        self.assertEqual(len(result["attempts"]), 1)
        self.assertEqual(len(FakeEngine.created), 1)
        self.assertTrue(engine.calls[-1]["use_det"])

    def test_failed_inference_rebuilds_handle_with_bounded_retry(self):
        class FailOnce(FakeEngine):
            failed = False

            def __call__(self, image, **kwargs):
                if not FailOnce.failed:
                    FailOnce.failed = True
                    raise RuntimeError("simulated native failure")
                return super().__call__(image, **kwargs)

        self.service._rapidocr_cls = FailOnce
        result = self.recognize_memory(np.full((100, 160, 3), 255, np.uint8))
        self.assertEqual(result["text"], "好")
        self.assertEqual([x["ok"] for x in result["attempts"]], [False, True])
        self.assertTrue(FakeEngine.created[0].initial_native.closed)
        self.assertEqual(len(self.service._engines), 1)

    def test_upscaled_region_boxes_return_to_source_coordinates(self):
        class SmallRegion(FakeEngine):
            def __call__(self, image, **kwargs):
                if image.shape[0] == 200:
                    return types.SimpleNamespace(txts=(), scores=(), boxes=np.empty((0, 4, 2)), elapse_list=[.001])
                return types.SimpleNamespace(txts=("小字",), scores=(.99,),
                    boxes=np.array([[[0, 0], [60, 0], [60, 45], [0, 45]]]), elapse_list=[.001])

        self.service._rapidocr_cls = SmallRegion
        region = (80, 60, 120, 90)
        with patch.object(helper, "analyze_image", return_value=ImageAnalysis(3, 3, 255, [region], [region])):
            result = self.recognize_memory(np.full((200, 200, 3), 255, np.uint8))
        self.assertEqual(result["lines"][0]["box"], [[80., 60.], [120., 60.], [120., 90.], [80., 90.]])
        self.assertEqual([x["kind"] for x in result["attempts"]], ["primary", "small-text"])


if __name__ == "__main__":
    unittest.main()
